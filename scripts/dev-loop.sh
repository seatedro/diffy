#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage: dev <command> [args...]

commands:
  once        run the default app check once
  watch       rerun the default app check when source files change
  run         run the app through cargo
  test        run the app library tests
  clippy      run clippy for the app crate
  fmt         check formatting
USAGE
}

cmd="${1:-once}"
if [[ $# -gt 0 ]]; then
  shift
fi

case "$cmd" in
  once | check)
    cargo check -p diffy --bin diffy "$@"
    ;;
  watch)
    watchexec -r -e rs,toml,lock -i target -- cargo check -p diffy --bin diffy "$@"
    ;;
  run)
    cargo run "$@"
    ;;
  test)
    cargo test -p diffy --lib "$@"
    ;;
  clippy)
    cargo clippy -p diffy --all-targets --all-features "$@"
    ;;
  fmt)
    cargo fmt --all --check "$@"
    ;;
  -h | --help | help)
    usage
    ;;
  *)
    usage
    exit 64
    ;;
esac
