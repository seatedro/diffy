#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT_DIR/.docs/reference-repos.txt"
TARGET_DIR="$ROOT_DIR/.docs/refs"

if [[ ! -f "$MANIFEST" ]]; then
  echo "Missing manifest: $MANIFEST" >&2
  exit 1
fi

mkdir -p "$TARGET_DIR"

while IFS= read -r repo_url; do
  [[ -z "$repo_url" ]] && continue
  [[ "$repo_url" =~ ^# ]] && continue

  repo_name="$(basename "$repo_url")"
  repo_name="${repo_name%.git}"
  dest="$TARGET_DIR/$repo_name"

  if [[ -d "$dest/.git" ]]; then
    echo "Updating $repo_name"
    git -C "$dest" pull --ff-only
  else
    echo "Cloning $repo_name"
    git clone --depth=1 --filter=blob:none "$repo_url" "$dest"
  fi
done < "$MANIFEST"
