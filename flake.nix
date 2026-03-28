{
  description = "Diffy - Rust native diff viewer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      pkgsFor = system: import nixpkgs { inherit system; };
      mkDevCommand = pkgs: pkgs.writeShellScriptBin "dev" ''
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
      packages = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          isLinux = pkgs.stdenv.isLinux;
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "diffy";
            version = "0.1.0";
            src = self;
            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.git
            ];

            buildInputs = pkgs.lib.optionals isLinux [
              pkgs.libxkbcommon
              pkgs.wayland
              pkgs.libGL
              pkgs.xorg.libX11
              pkgs.xorg.libXi
              pkgs.xorg.libXcursor
              pkgs.xorg.libXrandr
            ];
          };
        });

      devShells = forAllSystems (system:
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
              pkgs.watchexec
              (mkDevCommand pkgs)
            ] ++ pkgs.lib.optionals isLinux [
              pkgs.gcc
              pkgs.gdb
              pkgs.rr
              pkgs.strace
            ];

            shellHook = ''
              echo "Diffy dev shell ready"
              echo "Build: cargo build"
              echo "Test: cargo test"
              echo "Run: cargo run"
              echo "Debug binary: gdb ./target/debug/diffy | lldb ./target/debug/diffy | rr record ./target/debug/diffy"
              echo "Loop: dev once | dev watch"
            '';
          };
        });
    };
}
