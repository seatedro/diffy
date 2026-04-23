#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_ICON="$ROOT_DIR/assets/packaging/png/diffy-1024.png"
PACKAGING_DIR="$ROOT_DIR/assets/packaging"
PNG_DIR="$PACKAGING_DIR/png"
if ! command -v magick >/dev/null 2>&1; then
  echo "generate-packaging-assets: ImageMagick 'magick' is required" >&2
  exit 1
fi

mkdir -p "$PNG_DIR"

for size in 16 32 48 64 128 256 512 1024; do
  magick "$SOURCE_ICON" -resize "${size}x${size}" "$PNG_DIR/diffy-${size}.png"
done

magick \
  "$SOURCE_ICON" \
  -define icon:auto-resize=16,24,32,48,64,128,256 \
  "$PACKAGING_DIR/diffy.ico"

echo "Regenerated packaging assets in $PACKAGING_DIR"
