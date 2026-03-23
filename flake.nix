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

        rustVersion = "1.93.0";
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

        src = pkgs.lib.fileset.toSource {
          root = unfilteredRoot;
          fileset = pkgs.lib.fileset.unions [
            (craneLib.fileset.commonCargoSources unfilteredRoot)
            (pkgs.lib.fileset.fileFilter (file: file.hasExt "h" || file.hasExt "zig" || file.hasExt "zon" || file.hasExt "md") unfilteredRoot)
          ];
        };

        commonArgs = {
          inherit src;
          strictDeps = true;

          nativeBuildInputs = [
            pkgs.pkg-config
            zigPkg
            pkgs.clang
          ];

          buildInputs =
            [
              pkgs.libclang
              pkgs.openssl
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              pkgs.musl
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.libiconv
              pkgs.darwin.apple_sdk.frameworks.Security
            ];
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
            pkgs.xorg.libX11
            pkgs.xorg.libXcursor
            pkgs.xorg.libXrandr
            pkgs.xorg.libXinerama
            pkgs.xorg.libXi
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
            export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:${pkgs.libglvnd}/lib"
          '';
        };
      }
    );
}
