#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR_DIR="$ROOT_DIR/vendor/grammars"
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

declare -A GRAMMARS=(
  [c]="https://github.com/tree-sitter/tree-sitter-c.git"
  [cpp]="https://github.com/tree-sitter/tree-sitter-cpp.git"
  [rust]="https://github.com/tree-sitter/tree-sitter-rust.git"
  [python]="https://github.com/tree-sitter/tree-sitter-python.git"
  [javascript]="https://github.com/tree-sitter/tree-sitter-javascript.git"
  [go]="https://github.com/tree-sitter/tree-sitter-go.git"
  [bash]="https://github.com/tree-sitter/tree-sitter-bash.git"
  [json]="https://github.com/tree-sitter/tree-sitter-json.git"
  [toml]="https://github.com/tree-sitter-grammars/tree-sitter-toml.git"
  [zig]="https://github.com/tree-sitter-grammars/tree-sitter-zig.git"
  [nix]="https://github.com/nix-community/tree-sitter-nix.git"
)

mkdir -p "$VENDOR_DIR"

for lang in "${!GRAMMARS[@]}"; do
  url="${GRAMMARS[$lang]}"
  dest="$VENDOR_DIR/$lang"
  clone_dir="$TEMP_DIR/$lang"

  echo "Fetching $lang from $url"
  git clone --depth=1 --filter=blob:none "$url" "$clone_dir" 2>/dev/null

  rm -rf "$dest"
  mkdir -p "$dest/src"

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
  mkdir -p "$dest/queries"
  if [[ -f "$clone_dir/queries/highlights.scm" ]]; then
    cp "$clone_dir/queries/highlights.scm" "$dest/queries/"
  fi

  echo "  -> $dest"
done

echo "Done. Vendored ${#GRAMMARS[@]} grammars."
