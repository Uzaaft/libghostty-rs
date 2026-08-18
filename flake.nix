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
      url = "github:ghostty-org/ghostty/22d13172cde98a0a4dda05d3d6a3fcb0dd8ed018";
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

        miriToolchain = pkgs.rust-bin.selectLatestNightlyWith (
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

        miriCommand = pkgs.writeShellApplication {
          name = "libghostty-miri";
          runtimeInputs = [miriToolchain];
          text = ''
            cargo miri test --locked -p libghostty-vt-sys --lib
            cargo miri test --locked -p libghostty-vt --lib alloc::tests::
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
            cargoExtraArgs = "--locked --features libghostty-vt-sys/pkg-config";

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
              cargoClippyExtraArgs = "--workspace --all-targets";
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
        };

        apps.miri = flake-utils.lib.mkApp {
          drv = miriCommand;
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
          packages = [miriToolchain];
        };
      }
    );
}
