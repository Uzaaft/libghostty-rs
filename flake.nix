{
  description = "Rust bindings and safe API for libghostty";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/release-25.11";
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
  };

  outputs = {
    nixpkgs,
    flake-utils,
    crane,
    rust-overlay,
    zig,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };

        rustVersion = "1.90.0";
        rustExtensions = ["rust-src" "rust-std" "clippy" "rustfmt" "rust-analyzer"];

        toolchain = pkgs.rust-bin.stable.${rustVersion}.default.override {
          extensions = rustExtensions;
          targets = pkgs.lib.optionals pkgs.stdenv.isLinux [
            "x86_64-unknown-linux-gnu"
            "x86_64-unknown-linux-musl"
          ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
        unfilteredRoot = ./.;

        zigPkg = zig.packages.${system}."0.15.2";
        ghosttyCommit = "fdbf9ff3a31d7531b691cb49c98fc465a1a503a0";

        # Keep this in sync with GHOSTTY_COMMIT in
        # crates/libghostty-vt-sys/build.rs. Nix must provide Ghostty sources
        # up front because sandboxed builds cannot fetch from git.
        ghosttySrc = pkgs.fetchFromGitHub {
          owner = "ghostty-org";
          repo = "ghostty";
          rev = ghosttyCommit;
          hash = "sha256-TW2dtJ1wZGtdyqQ4YAsfjbTLURhMISIMNK0c0aIy1xM=";
        };

        # Ghostty ships a zon2nix-generated link farm for its Zig package
        # dependencies. build.rs passes this through --system so Zig never
        # downloads packages during the Cargo build script.
        ghosttyZigDeps = pkgs.callPackage (ghosttySrc + "/build.zig.zon.nix") {
          name = "ghostty-zig-deps-${builtins.substring 0 7 ghosttyCommit}";
          zig_0_15 = zigPkg;
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
            version = "0.2.0";
            inherit src;
            strictDeps = true;
            GHOSTTY_SOURCE_DIR = "${ghosttySrc}";
            GHOSTTY_ZIG_SYSTEM_DIR = "${ghosttyZigDeps}";

            nativeBuildInputs = [
              pkgs.pkg-config
              zigPkg
              pkgs.clang
            ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.cctools
              pkgs.xcbuild
            ];

            buildInputs =
              [
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
        nonSimdCargoArtifacts = craneLib.buildDepsOnly (
          commonArgs
          // {
            pname = "libghostty-rs-non-simd-deps";
            LIBGHOSTTY_VT_SYS_SIMD = "false";
          }
        );
        ciValgrindCargoArtifacts = craneLib.buildDepsOnly (
          commonArgs
          // {
            pname = "libghostty-rs-ci-valgrind-deps";
            cargoExtraArgs = "--locked -p libghostty-vt";
            cargoTestExtraArgs = "--no-run";
            CARGO_PROFILE = "";

            # The Valgrind check must not inherit ReleaseFast Zig artifacts from
            # the normal package build. Debug keeps both Rust and Zig codegen in
            # Valgrind's supported instruction set and gives better reports.
            LIBGHOSTTY_VT_SYS_OPTIMIZE = "Debug";
            LIBGHOSTTY_VT_SYS_SIMD = "false";
          }
        );

        application = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            postInstall = ''
              ghostty_install=$(find target -path '*/out/ghostty-install' -type d | head -1)
              if [ -z "$ghostty_install" ]; then
                echo "expected Cargo build script to install libghostty-vt into OUT_DIR" >&2
                exit 1
              fi
              ghostty_install=$(realpath "$ghostty_install")

              cp -R "$ghostty_install"/. "$out"/
              substituteInPlace "$out"/share/pkgconfig/*.pc \
                --replace-fail "prefix=$ghostty_install" "prefix=$out"
            '';
          }
        );

        libghosttyVtNonSimd = craneLib.buildPackage (
          commonArgs
          // {
            cargoArtifacts = nonSimdCargoArtifacts;
            pname = "libghostty-vt-non-simd";
            LIBGHOSTTY_VT_SYS_SIMD = "false";
          }
        );

        ciValgrind = craneLib.buildPackage (
          commonArgs
          // {
            cargoArtifacts = ciValgrindCargoArtifacts;
            pname = "libghostty-rs-ci-valgrind";
            cargoBuildCommand = "";
            cargoTestCommand = "cargo valgrind test";
            cargoExtraArgs = "--locked -p libghostty-vt";
            cargoTestExtraArgs = "-- --test-threads=1";
            CARGO_PROFILE = "";
            LIBGHOSTTY_VT_SYS_OPTIMIZE = "Debug";
            LIBGHOSTTY_VT_SYS_SIMD = "false";
            nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
              pkgs.cargo-valgrind
              pkgs.valgrind
            ];

            installPhaseCommand = "mkdir -p $out";
          }
        );
      in {
        packages = {
          default = application;
          libghostty-vt-non-simd = libghosttyVtNonSimd;
        };

        checks =
          {
            default = application;
          }
          // pkgs.lib.optionalAttrs (system == "x86_64-linux") {
            ci-valgrind = ciValgrind;
          };

        devShells.default = craneLib.devShell {
          packages = [
            toolchain
            zigPkg
            pkgs.clang
            pkgs.libclang
            pkgs.pkg-config
            pkgs.openssl
            pkgs.cmake
            pkgs.ninja
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
            # Valgrind is Linux-only here. Keep it in the development shell so
            # CI and local Linux users exercise the same memory-checking path.
            pkgs.valgrind
            pkgs.libx11
            pkgs.libxcursor
            pkgs.libxrandr
            pkgs.libxinerama
            pkgs.libxi
            pkgs.libGL
            pkgs.libxkbcommon
            pkgs.wayland
          ] ++ pkgs.lib.optionals (system == "x86_64-linux") [
            pkgs.cargo-valgrind
          ];

          shellHook = ''
            export GHOSTTY_SOURCE_DIR=${ghosttySrc}
            export GHOSTTY_ZIG_SYSTEM_DIR=${ghosttyZigDeps}
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
      }
    );
}
