{
  description = "Rust bindings and safe API for libghostty";

  nixConfig = {
    extra-substituters = ["https://ghostty.cachix.org"];
    extra-trusted-public-keys = ["ghostty.cachix.org-1:QB389yTa6gTyneehvqG58y0WnHjQOqgnA+wBnpWWxns="];
  };

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/release-26.05";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    zig = {
      url = "github:mitchellh/zig-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    ghostty = {
      url = "github:ghostty-org/ghostty/3425025e585a3403340d4dc6d65132f42e05e605";
    };
  };

  outputs = {
    nixpkgs,
    flake-utils,
    crane,
    rust-overlay,
    zig,
    ghostty,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };

        rustVersion = "1.90.0";
        # crane's devShell puts this toolchain's cargo/rustc ahead of the
        # devToolchain in packages, so it must carry the cross targets too or
        # its sysroot (without their std) shadows the devToolchain's.
        buildToolchain = pkgs.rust-bin.stable.${rustVersion}.minimal.override {
          targets = rustTargets;
        };
        rustTargets =
          pkgs.lib.optionals pkgs.stdenv.isLinux [
            "x86_64-unknown-linux-gnu"
            "x86_64-unknown-linux-musl"
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            # iOS builds go through the vendored zig xcframework path and
            # need the target's std so the dev shell can cargo-check them.
            "aarch64-apple-ios"
            "aarch64-apple-ios-sim"
          ];

        checkToolchain = pkgs.rust-bin.stable.${rustVersion}.default.override {
          extensions = ["clippy" "rustfmt"];
        };

        devToolchain = pkgs.rust-bin.stable.${rustVersion}.default.override {
          extensions = ["rust-src" "rust-std" "clippy" "rustfmt" "rust-analyzer"];
          targets = rustTargets;
        };

        # Miri and AddressSanitizer need nightly. Both build their own std
        # from rust-src.
        nightlyToolchain = pkgs.rust-bin.selectLatestNightlyWith (
          toolchain:
            toolchain.default.override {
              extensions = ["rust-src" "rust-std" "miri"];
              targets = rustTargets;
            }
        );

        craneLib = (crane.mkLib pkgs).overrideToolchain buildToolchain;
        craneCheckLib = (crane.mkLib pkgs).overrideToolchain checkToolchain;
        unfilteredRoot = ./.;

        zigPkg = zig.packages.${system}."0.16.0";
        ghosttyLib = ghostty.packages.${system}.libghostty-vt;
        ghosttyDebugLib = ghostty.packages.${system}.libghostty-vt-debug;

        miriCommand = pkgs.writeShellApplication {
          name = "libghostty-miri";
          runtimeInputs = [nightlyToolchain];
          text = ''
            cargo miri test --locked -p libghostty-vt-sys --lib
            cargo miri test --locked -p libghostty-vt --lib alloc::tests::
            # Issue #74 soundness regression tests (both clipboard findings
            # live in this module). Miri aborts the test process at the first
            # UB it finds, so a broken tree reports only the first failing test.
            cargo miri test --locked -p libghostty-vt --lib miri_soundness
          '';
        };

        # AddressSanitizer instruments Rust code only: Zig has no ASan, so
        # libghostty's own loads and stores go unchecked (that is what
        # alloc::testing::Guard is for). It still checks every access the
        # wrapper makes to memory libghostty allocates through its default
        # allocator, which is malloc, since ASan replaces malloc. On Linux, it
        # also runs LeakSanitizer, which catches a Drop that forgets to free
        # a libghostty object.
        #
        # Terminal pages (the cells, styles and graphemes behind grid refs
        # and render rows) are the exception: libghostty maps them straight
        # from the OS, so a view outliving them goes unnoticed. Checking
        # libghostty's side with Valgrind is deliberately left for later.
        #
        # There is no MemorySanitizer app: it needs every store instrumented,
        # and Zig can't be. Run over this suite on Linux, it flagged 15 of
        # the 84 tests, every one a report about bytes libghostty wrote:
        # buffers filled through Zig's memcpy, heap blocks Zig reused after
        # a free, callback arguments from a Zig caller. Unpoisoning those at
        # the FFI boundary would hide exactly what MSan is there to check,
        # and the tests that stay in Rust are already Miri's.
        asanCommand = pkgs.writeShellApplication {
          name = "libghostty-asan";
          runtimeInputs =
            [nightlyToolchain pkgs.pkg-config]
            # Linux reports are bare addresses without llvm-symbolizer on
            # PATH. macOS symbolizes with atos instead, and links with the
            # system cc like the dev shell does.
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [pkgs.stdenv.cc pkgs.llvm];
          text = ''
            # The same prebuilt ReleaseFast library the cargo-test check
            # links. A Debug build bundles Zig's UBSan runtime, whose weak
            # handlers the ASan runtime's own would silently replace; the
            # cargo-test-ubsan check covers that build instead.
            export PKG_CONFIG_PATH=${ghosttyLib.dev}/share/pkgconfig
            export RUSTFLAGS=-Zsanitizer=address
            # rustdoc builds the doctests and doesn't read RUSTFLAGS.
            export RUSTDOCFLAGS=-Zsanitizer=address
            # -Zbuild-std instruments std too, so std code the wrapper passes
            # pointers and lengths to is checked as well. Cargo only allows it
            # with an explicit --target, which also keeps RUSTFLAGS away from
            # the build scripts.
            args=(
              --locked -Zbuild-std --target "$(rustc --print host-tuple)"
              --features libghostty-vt-sys/pkg-config --features libghostty-vt/png
            )
            cargo test "''${args[@]}" -p libghostty-vt-sys -p libghostty-vt --lib --tests
            cargo test "''${args[@]}" -p libghostty-vt --doc
          '';
        };

        src = pkgs.lib.fileset.toSource {
          root = unfilteredRoot;
          fileset = pkgs.lib.fileset.unions [
            (craneLib.fileset.commonCargoSources unfilteredRoot)
            (pkgs.lib.fileset.fileFilter (
              file:
                file.hasExt "h"
                || file.hasExt "zig"
                || file.hasExt "zon"
                || file.hasExt "md"
                || file.hasExt "ttf"
            ) unfilteredRoot)
          ];
        };

        commonArgs =
          {
            pname = "libghostty-rs";
            version = "0.2.1";
            inherit src;
            strictDeps = true;
            # No workspace member enables the optional `png` feature, so turn it
            # on here; otherwise the PNG decoder is never type-checked, linted,
            # doc-checked or tested in CI. Setting it in the shared arguments
            # also builds the dependency artifacts with it, so the checks
            # below don't each rebuild png and its dependencies.
            cargoExtraArgs = "--locked --features libghostty-vt-sys/pkg-config --features libghostty-vt/png";

            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.clang
            ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.cctools
              pkgs.xcbuild
            ];

            buildInputs =
              [
                ghosttyLib
                pkgs.libclang
                pkgs.openssl
              ]
              ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
                pkgs.apple-sdk
                pkgs.libiconv
              ];
          }
          // pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin {
            DEVELOPER_DIR = "${pkgs.apple-sdk}";
            SDKROOT = "${pkgs.apple-sdk.sdkroot}";
          };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        application = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
          }
        );
      in {
        packages.default = application;

        checks = {
          default = application;

          cargo-check = craneLib.mkCargoDerivation (
            commonArgs
            // {
              inherit cargoArtifacts;
              pnameSuffix = "-check";
              buildPhaseCargoCommand = "cargoWithProfile check ${commonArgs.cargoExtraArgs} --workspace --all-targets";
            }
          );

          cargo-clippy = craneCheckLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              # Without denying warnings, clippy findings never fail CI.
              cargoClippyExtraArgs = "--workspace --all-targets -- --deny warnings";
            }
          );

          cargo-doc = craneCheckLib.cargoDoc (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoDocExtraArgs = "--workspace --no-deps";
              RUSTDOCFLAGS = "-D warnings";
            }
          );

          # Regenerate the bindings from the pinned Ghostty headers and fail if
          # they differ from the checked-in ones. GHOSTTY_INCLUDE_DIR is used
          # instead of GHOSTTY_SOURCE_DIR so that the build script keeps linking
          # the prebuilt library rather than building Ghostty from source.
          #
          # The headers come from the flake input, but users build against
          # the commit pinned in build.rs. Nothing else ties the two together,
          # so check that they agree first; otherwise bumping only one of them
          # would leave this check green while comparing the wrong headers.
          bindings-fresh = craneCheckLib.mkCargoDerivation (
            commonArgs
            // {
              # The shared deps are built without the bindgen-tool feature and in
              # the release profile, so they would be unpacked and then ignored.
              # A dev build of just the generator is quicker than a release one.
              cargoArtifacts = null;
              pnameSuffix = "-bindings";
              GHOSTTY_INCLUDE_DIR = "${ghostty}/include";
              LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
              buildPhaseCargoCommand = ''
                if ! grep -qF 'const GHOSTTY_COMMIT: &str = "${ghostty.rev}";' \
                  crates/libghostty-vt-sys/build.rs; then
                  echo "error: the Ghostty pin in flake.nix (${ghostty.rev}) does not" >&2
                  echo "match GHOSTTY_COMMIT in crates/libghostty-vt-sys/build.rs." >&2
                  echo "Bump both to the same commit." >&2
                  exit 1
                fi
                # Not commonArgs.cargoExtraArgs: it also enables libghostty-vt
                # features, which cargo rejects with only the -sys crate selected.
                cargo run --locked --features libghostty-vt-sys/pkg-config -p libghostty-vt-sys \
                  --features libghostty-vt-sys/bindgen-tool --bin gen-bindings
                cargo fmt -p libghostty-vt-sys
                if ! diff -u ${src}/crates/libghostty-vt-sys/src/bindings.rs \
                  crates/libghostty-vt-sys/src/bindings.rs; then
                  echo "error: crates/libghostty-vt-sys/src/bindings.rs is out of date." >&2
                  echo "Regenerate it from the dev shell with:" >&2
                  echo "  cargo run -p libghostty-vt-sys --features bindgen-tool --bin gen-bindings && cargo fmt -p libghostty-vt-sys" >&2
                  exit 1
                fi
              '';
              doInstallCargoArtifacts = false;
              installPhaseCommand = "touch $out";
            }
          );

          cargo-fmt = craneCheckLib.cargoFmt {
            pname = "libghostty-rs";
            version = "0.2.1";
            inherit src;
          };

          cargo-test = craneLib.cargoTest (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoTestExtraArgs = "--workspace --all-targets";
            }
          );

          # `--all-targets` does not include doctests, so run them separately.
          # Only for the safe crate: the -sys bindings carry C header comments
          # that rustdoc would otherwise try to compile as Rust.
          cargo-doctest = craneLib.cargoTest (
            commonArgs
            // {
              inherit cargoArtifacts;
              pnameSuffix = "-doctest";
              cargoTestExtraArgs = "-p libghostty-vt --doc";
            }
          );

          # Upstream ghostty exercises both link modes in CI, so run the test
          # suite against the shared library as well. Cargo adds the emitted
          # link-search paths to the test binaries' library path, so the
          # ghostty .so is found at runtime.
          cargo-test-dynamic = craneLib.cargoTest (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoExtraArgs = "${commonArgs.cargoExtraArgs} --features libghostty-vt-sys/link-dynamic";
              cargoTestExtraArgs = "--workspace --all-targets";
            }
          );

          # The UndefinedBehaviorSanitizer counterpart. rustc has no UBSan;
          # the dev profile's debug assertions, overflow checks and std's
          # checks of unsafe preconditions play its part. Zig has none for
          # Zig code either; its Debug safety checks (bounds, overflow,
          # unreachable code, bad casts) play it there, along with Ghostty's
          # own integrity checks, which only Debug runs. Debug also builds
          # Ghostty's own C and C++ (src/simd and wuffs) with
          # -fsanitize-c=full, and libghostty bundles Zig's UBSan runtime,
          # which panics on the first report. simdutf and highway opt out of
          # it upstream.
          #
          # This is the libghostty-vt-debug package rather than
          # LIBGHOSTTY_VT_SYS_OPTIMIZE=Debug because that only affects
          # building Ghostty from source, which the sandbox can't fetch. It
          # needs no nightly or network either, so it runs here on every
          # platform rather than as an app.
          cargo-test-ubsan = craneLib.mkCargoDerivation (
            commonArgs
            // {
              # The shared dependency artifacts are built in the release
              # profile, so this would only unpack and then ignore them.
              cargoArtifacts = null;
              pnameSuffix = "-ubsan";
              CARGO_PROFILE = "dev";
              buildInputs = [ghosttyDebugLib] ++ pkgs.lib.remove ghosttyLib commonArgs.buildInputs;
              buildPhaseCargoCommand = ''
                cargoWithProfile test ${commonArgs.cargoExtraArgs} --workspace --all-targets
                cargoWithProfile test ${commonArgs.cargoExtraArgs} -p libghostty-vt --doc
              '';
              doInstallCargoArtifacts = false;
              installPhaseCommand = "touch $out";
            }
          );
        };

        apps.miri = flake-utils.lib.mkApp {
          drv = miriCommand;
        };

        apps.asan = flake-utils.lib.mkApp {
          drv = asanCommand;
        };

        devShells.default = craneLib.devShell {
          packages = [
            devToolchain
            zigPkg
            pkgs.clang
            pkgs.libclang
            pkgs.pkg-config
            pkgs.openssl
            pkgs.cmake
            pkgs.ninja
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
            pkgs.libx11
            pkgs.libxcursor
            pkgs.libxrandr
            pkgs.libxinerama
            pkgs.libxi
            pkgs.libGL
            pkgs.libxkbcommon
            pkgs.wayland
          ];

          shellHook = ''
            export LIBCLANG_PATH=${pkgs.libclang.lib}/lib
          '' + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
            # Unset Nix Darwin SDK env vars and remove the xcbuild
            # xcrun wrapper so Zig's SDK detection uses the real
            # system xcrun/xcode-select.
            unset SDKROOT
            unset DEVELOPER_DIR
            export PATH=$(echo "$PATH" | tr ':' '\n' | grep -v xcbuild | tr '\n' ':')
          '' + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
            # Make Ghostling able to find libGL on Linux.
            export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:${pkgs.lib.makeLibraryPath [
              pkgs.libglvnd
              pkgs.wayland
              pkgs.libx11
              pkgs.libxkbcommon
              pkgs.libxi
            ]}"
          '';
        };

        devShells.miri = pkgs.mkShell {
          packages = [nightlyToolchain];
        };
      }
    );
}
