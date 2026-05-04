{
  description = "Diffy - Rust native diff viewer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = lib.genAttrs supportedSystems;
      pkgsFor = system: import nixpkgs { inherit system; };
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      mkDevCommand =
        pkgs:
        pkgs.writeShellScriptBin "dev" ''
          set -euo pipefail
          repo_root="''${DIFFY_REPO_ROOT:-$PWD}"
          if [ ! -x "$repo_root/scripts/dev-loop.sh" ]; then
            echo "dev: expected DIFFY_REPO_ROOT or current directory to point at the diffy repo" >&2
            exit 1
          fi
          exec "$repo_root/scripts/dev-loop.sh" "$@"
        '';
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          isLinux = pkgs.stdenv.isLinux;
          diffy = pkgs.rustPlatform.buildRustPackage {
            pname = "diffy";
            version = cargoToml.workspace.package.version;
            src = self;
            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.git
            ];

            buildInputs = [
              pkgs.openssl
            ]
            ++ lib.optionals isLinux [
              pkgs.libxkbcommon
              pkgs.wayland
              pkgs.libGL
              pkgs.libx11
              pkgs.libxi
              pkgs.libxcursor
              pkgs.libxrandr
              pkgs.dbus
            ];
          };
        in
        {
          default = diffy;
          inherit diffy;
        }
      );

      apps = forAllSystems (system: {
        default = self.apps.${system}.diffy;
        diffy = {
          type = "app";
          program = "${self.packages.${system}.diffy}/bin/diffy";
        };
      });

      overlays.default = final: _prev: {
        diffy = self.packages.${final.stdenv.hostPlatform.system}.diffy;
      };

      nixosModules.default =
        { pkgs, ... }:
        {
          environment.systemPackages = [
            self.packages.${pkgs.stdenv.hostPlatform.system}.diffy
          ];
        };
      nixosModules.diffy = self.nixosModules.default;

      darwinModules.default =
        { pkgs, ... }:
        {
          environment.systemPackages = [
            self.packages.${pkgs.stdenv.hostPlatform.system}.diffy
          ];
        };
      darwinModules.diffy = self.darwinModules.default;

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          isLinux = pkgs.stdenv.isLinux;
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.default ];

            packages = [
              pkgs.cargo
              pkgs.rustc
              pkgs.rustfmt
              pkgs.clippy
              pkgs.nodejs_22
              pkgs.uv
              pkgs.git
              pkgs.jq
              pkgs.lldb
              pkgs.lld
              pkgs.watchexec
              (mkDevCommand pkgs)
            ]
            ++ pkgs.lib.optionals isLinux [
              pkgs.gcc
              pkgs.gdb
              pkgs.rr
              pkgs.strace
            ];

            shellHook =
              (pkgs.lib.optionalString isLinux ''
                export LD_LIBRARY_PATH="${
                  pkgs.lib.makeLibraryPath [
                    pkgs.libxkbcommon
                    pkgs.wayland
                    pkgs.libGL
                    pkgs.vulkan-loader
                  ]
                }''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
              '')
              + ''
                echo "Diffy dev shell ready"
                echo "Build: cargo build"
                echo "Test: cargo test"
                echo "Run: cargo run"
                echo "Debug binary: gdb ./target/debug/diffy | lldb ./target/debug/diffy | rr record ./target/debug/diffy"
                echo "Loop: dev once | dev watch"
              '';
          };
        }
      );
    };
}
