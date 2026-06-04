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

        rustPlatform = pkgs.makeRustPlatform {
          cargo = toolchain;
          rustc = toolchain;
        };

        cargoAfl = rustPlatform.buildRustPackage rec {
          pname = "cargo-afl";
          version = "0.18.1";

          src = pkgs.fetchCrate {
            inherit pname version;
            hash = "sha256-W2ELM28vHs8xjgh0gRyH/O17kDgMFxKNOnnlbputQb0=";
          };

          cargoHash = "sha256-ZJqSZGdqDd4E+PfGttsxyKsKZCBKzzCYHWhpJlTE1Zg=";

          # cargo-afl's CLI tests try to write below the read-only source tree.
          # The binary itself is enough for the dev shell and VM workflow.
          doCheck = false;
        };

        aflplusplusAvailable = pkgs.lib.elem pkgs.stdenv.hostPlatform.system [
          "x86_64-linux"
          "i686-linux"
        ];

        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
        unfilteredRoot = ./.;

        zigPkg = zig.packages.${system}."0.15.2";
        ghosttyCommit = "cb36966a752982014827a9cabcf630ec3788b3d9";

        # Keep this in sync with GHOSTTY_COMMIT in
        # crates/libghostty-vt-sys/build.rs. Nix must provide Ghostty sources
        # up front because sandboxed builds cannot fetch from git.
        ghosttySrc = pkgs.fetchFromGitHub {
          owner = "ghostty-org";
          repo = "ghostty";
          rev = ghosttyCommit;
          hash = "sha256-Bn1M2fBFf/F4zBhU1rNemIdBkE3o6ramsCotXigpUNM=";
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
            version = "0.1.1";
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

        application = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
          }
        );

        aflVm = nixpkgs.lib.nixosSystem {
          system = "x86_64-linux";
          specialArgs = { hostPkgs = pkgs; };
          modules = [
            ({ hostPkgs, pkgs, ... }: {
              networking.hostName = "libghostty-afl";

              nix.settings.experimental-features = ["nix-command" "flakes"];

              virtualisation.vmVariant.virtualisation = {
                host.pkgs = hostPkgs;
                graphics = false;
                memorySize = 4096;
                cores = 4;
              };

              fileSystems."/work" = {
                device = "work";
                fsType = "9p";
                options = [
                  "trans=virtio"
                  "version=9p2000.L"
                  "msize=104857600"
                ];
              };

              users.users.nixos = {
                isNormalUser = true;
                extraGroups = ["wheel"];
              };
              services.getty.autologinUser = "nixos";
              security.sudo.wheelNeedsPassword = false;

              environment.systemPackages = [
                pkgs.git
                pkgs.valgrind
                pkgs.cargo-valgrind
              ];

              system.stateVersion = "25.11";
            })
          ];
        };

        aflVmApp = pkgs.writeShellApplication {
          name = "afl-vm";
          text = ''
            repo="''${1:-$PWD}"
            export QEMU_OPTS="''${QEMU_OPTS-} -virtfs local,path=$repo,mount_tag=work,security_model=none,multidevs=remap"
            runner=$(echo ${aflVm.config.system.build.vm}/bin/run-*-vm)
            exec "$runner"
          '';
        };
      in {
        packages = {
          default = application;
          "cargo-afl" = cargoAfl;
          "afl-vm" = aflVm.config.system.build.vm;
        };

        apps.afl-vm = {
          type = "app";
          program = "${aflVmApp}/bin/afl-vm";
        };

        checks.default = application;

        devShells.default = craneLib.devShell {
          packages = [
            toolchain
            zigPkg
            cargoAfl
            pkgs.clang
            pkgs.libclang
            pkgs.pkg-config
            pkgs.openssl
            pkgs.cmake
            pkgs.git
            pkgs.gnumake
            pkgs.ninja
          ] ++ pkgs.lib.optionals aflplusplusAvailable [
            pkgs.aflplusplus
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
            # Valgrind is Linux-only (broken on aarch64-darwin in nixpkgs).
            # cargo-valgrind is a thin Cargo subcommand, but only useful when
            # valgrind itself is available, so gate both together.
            pkgs.valgrind
            pkgs.cargo-valgrind
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
