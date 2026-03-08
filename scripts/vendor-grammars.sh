#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR_DIR="$ROOT_DIR/vendor/grammars"
LOCK_FILE="$ROOT_DIR/vendor/grammars.lock"
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

mkdir -p "$VENDOR_DIR"

if [[ ! -f "$LOCK_FILE" ]]; then
  echo "Missing lock file: $LOCK_FILE" >&2
  exit 1
fi

while read -r lang url commit; do
  if [[ -z "${lang:-}" || "${lang:0:1}" == "#" ]]; then
    continue
  fi

  dest="$VENDOR_DIR/$lang"
  clone_dir="$TEMP_DIR/$lang"

  echo "Fetching $lang at $commit from $url"
  git init -q "$clone_dir"
  git -C "$clone_dir" remote add origin "$url"
  git -C "$clone_dir" fetch -q --depth=1 origin "$commit"
  git -C "$clone_dir" checkout -q FETCH_HEAD

  mkdir -p "$dest"
  rm -rf "$dest/src"
  mkdir -p "$dest/src" "$dest/queries"

  # Copy parser source
  if [[ -f "$clone_dir/src/parser.c" ]]; then
    cp "$clone_dir/src/parser.c" "$dest/src/"
  fi

  # Copy scanner if present (can be .c or .cc)
  if [[ -f "$clone_dir/src/scanner.c" ]]; then
    cp "$clone_dir/src/scanner.c" "$dest/src/"
  elif [[ -f "$clone_dir/src/scanner.cc" ]]; then
    cp "$clone_dir/src/scanner.cc" "$dest/src/"
  fi

  # Copy tree_sitter header directory (needed by parser.c)
  if [[ -d "$clone_dir/src/tree_sitter" ]]; then
    cp -r "$clone_dir/src/tree_sitter" "$dest/src/"
  fi

  # Copy highlights query
  rm -f "$dest/queries/highlights.scm"
  if [[ -f "$clone_dir/queries/highlights.scm" ]]; then
    cp "$clone_dir/queries/highlights.scm" "$dest/queries/"
  fi

  echo "  -> $dest"
done < "$LOCK_FILE"

echo "Done. Vendored grammars from $LOCK_FILE."
