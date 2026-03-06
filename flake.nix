{
  description = "Diffy - native Qt/C++ diff viewer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = [
          pkgs.nodejs_22
          pkgs.cmake
          pkgs.ninja
          pkgs.pkg-config
          pkgs.git
          pkgs.gcc
        ];

        buildInputs = [
          pkgs.curl
          pkgs.libgit2
          pkgs.tree-sitter
          pkgs.qt6.qtbase
          pkgs.qt6.qtdeclarative
        ];

        shellHook = ''
          echo "Diffy dev shell ready"
          echo "Build: cmake -S . -B build -G Ninja && cmake --build build"
        '';
      };
    };
}
