{
  description = "Diffy - native Qt/C++ diff viewer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      qt = pkgs.qt6;
      devCommand = pkgs.writeShellScriptBin "dev" ''
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
      packages.${system}.default = pkgs.stdenv.mkDerivation {
        pname = "diffy";
        version = "0.1.0";
        src = self;

        nativeBuildInputs = [
          pkgs.cmake
          pkgs.ninja
          pkgs.pkg-config
          pkgs.git
          qt.wrapQtAppsHook
        ];

        buildInputs = [
          pkgs.curl
          pkgs.libgit2
          pkgs.tree-sitter
          qt.qtbase
          qt.qtdeclarative
        ];

        cmakeFlags = [ "-G" "Ninja" ];
      };

      devShells.${system}.default = pkgs.mkShell {
        inputsFrom = [ self.packages.${system}.default ];

        packages = [
          pkgs.nodejs_22
          pkgs.git
          pkgs.gcc
          pkgs.jq
          pkgs.gdb
          pkgs.lldb
          pkgs.rr
          pkgs.strace
          pkgs.watchexec
          devCommand
        ];

        shellHook = ''
          export DIFFY_REPO_ROOT="$PWD"
          echo "Diffy dev shell ready"
          echo "Build: cmake -S . -B build -G Ninja && cmake --build build"
          echo "Debug preset: cmake --preset Debug && cmake --build --preset Debug"
          echo "Debug binary: gdb ./build/Debug/diffy | lldb ./build/Debug/diffy | rr record ./build/Debug/diffy"
          echo "Loop: dev once | dev watch | dev preview"
        '';
      };
    };
}
