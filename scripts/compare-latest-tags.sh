#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_REPO="${1:-$ROOT_DIR}"

if [[ ! -d "$TARGET_REPO" ]]; then
  echo "error: target repo does not exist: $TARGET_REPO" >&2
  exit 1
fi

TARGET_REPO="$(cd "$TARGET_REPO" && pwd)"

if ! git -C "$TARGET_REPO" rev-parse --git-dir >/dev/null 2>&1; then
  echo "error: target path is not a git repository: $TARGET_REPO" >&2
  exit 1
fi

if [[ ! -f "$ROOT_DIR/crates/difftastic/Cargo.toml" ]]; then
  echo "Initializing crates/difftastic submodule"
  git -C "$ROOT_DIR" submodule update --init --recursive crates/difftastic
fi

if [[ "$(uname -s)" == Linux* ]]; then
  if ! command -v pkg-config >/dev/null 2>&1 || ! pkg-config --exists dbus-1; then
    echo "hint: Linux builds may need: sudo apt-get install libdbus-1-dev pkg-config" >&2
  fi
fi

mapfile -t TAGS < <(git -C "$TARGET_REPO" tag --sort=-v:refname)

if (( ${#TAGS[@]} < 2 )); then
  echo "error: need at least two tags in $TARGET_REPO to compare latest tags" >&2
  exit 1
fi

LATEST="${TAGS[0]}"
PREVIOUS="${TAGS[1]}"

echo "Opening Diffy comparison: $TARGET_REPO $PREVIOUS -> $LATEST (two-dot)"

cd "$ROOT_DIR"
exec cargo run -- --repo "$TARGET_REPO" --left "$PREVIOUS" --right "$LATEST" --compare-mode two-dot
