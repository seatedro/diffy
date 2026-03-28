{ pkgs, ... }:
{
  packages = [
    pkgs.cargo
    pkgs.rustc
    pkgs.rustfmt
    pkgs.clippy
    pkgs.pkg-config
    pkgs.uv
    pkgs.git
    pkgs.jq
    pkgs.gdb
    pkgs.lldb
    pkgs.rr
    pkgs.strace
    pkgs.watchexec
    pkgs.qt6.qtbase
    pkgs.qt6.qtdeclarative
  ];

  enterShell = ''
    qt_declarative_prefix="${pkgs.qt6.qtdeclarative}"
    export DIFFY_REPO_ROOT="$PWD"
    export QMAKE="${pkgs.lib.getExe' pkgs.qt6.qtbase "qmake"}"
    export QT_ADDITIONAL_PACKAGES_PREFIX_PATH="$qt_declarative_prefix''${QT_ADDITIONAL_PACKAGES_PREFIX_PATH:+:''${QT_ADDITIONAL_PACKAGES_PREFIX_PATH}}"
    export CXXFLAGS="-F$qt_declarative_prefix/lib -I$qt_declarative_prefix/include''${CXXFLAGS:+ ''${CXXFLAGS}}"
    export RUSTFLAGS="-L framework=$qt_declarative_prefix/lib''${RUSTFLAGS:+ ''${RUSTFLAGS}}"
    echo "devenv ready: cargo build && cargo test"
    echo "run: cargo run"
    echo "debug ready: gdb ./target/debug/diffy | lldb ./target/debug/diffy | rr record ./target/debug/diffy"
  '';
}
