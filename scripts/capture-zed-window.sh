#!/usr/bin/env bash
set -euo pipefail

OUT_PATH="${1:-/tmp/zed-auto.png}"

if ! command -v niri >/dev/null 2>&1; then
  echo "niri is required for automatic Zed window capture" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for automatic Zed window capture" >&2
  exit 1
fi

WINDOW_ID="$(
  niri msg -j windows \
    | jq -r '.[] | select(.app_id=="dev.zed.Zed") | .id' \
    | head -n 1
)"

if [[ -z "${WINDOW_ID}" ]]; then
  echo "Could not find a running Zed window" >&2
  exit 1
fi

niri msg action screenshot-window --id "${WINDOW_ID}" --path "${OUT_PATH}"
printf '%s\n' "${OUT_PATH}"
