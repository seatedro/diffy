{ pkgs, ... }:
{
  packages = [
    pkgs.cmake
    pkgs.ninja
    pkgs.pkg-config
    pkgs.uv
    pkgs.git
    pkgs.gcc
    pkgs.jq
    pkgs.gdb
    pkgs.lldb
    pkgs.rr
    pkgs.strace
    pkgs.watchexec
    pkgs.curl
    pkgs.libgit2
    pkgs.tree-sitter
    pkgs.qt6.qtbase
    pkgs.qt6.qtdeclarative
  ];

  enterShell = ''
    echo "devenv ready: cmake -S . -B build -G Ninja && cmake --build build"
    echo "debug preset: cmake --preset Debug && cmake --build --preset Debug"
    echo "debug ready: gdb ./build/Debug/diffy | lldb ./build/Debug/diffy | rr record ./build/Debug/diffy"
  '';
}
