{
  description = "Diffy - native Qt/C++ diff viewer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      qt = pkgs.qt6;
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
        ];

        shellHook = ''
          echo "Diffy dev shell ready"
          echo "Build: cmake -S . -B build -G Ninja && cmake --build build"
        '';
      };
    };
}
