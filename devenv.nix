{ pkgs, ... }:
{
  packages = [
    pkgs.cmake
    pkgs.ninja
    pkgs.pkg-config
    pkgs.git
    pkgs.gcc
    pkgs.libgit2
    pkgs.tree-sitter
    pkgs.qt6.qtbase
    pkgs.qt6.qtdeclarative
  ];

  enterShell = ''
    echo "devenv ready: cmake -S . -B build -G Ninja && cmake --build build"
  '';
}
