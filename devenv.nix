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
    pkgs.jujutsu
    pkgs.jq
    pkgs.gdb
    pkgs.lldb
    pkgs.rr
    pkgs.strace
    pkgs.watchexec
  ];

  enterShell = ''
    export DIFFY_DEV_GITHUB_TOKEN_FILE="''${DIFFY_DEV_GITHUB_TOKEN_FILE:-1}"
    echo "devenv ready: cargo build && cargo test"
    echo "run: cargo run"
    echo "debug ready: gdb ./target/debug/diffy | lldb ./target/debug/diffy | rr record ./target/debug/diffy"
  '';
}
