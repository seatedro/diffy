#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/capture.sh [absolute-output-path]

Captures the focused diffy window through niri. If no diffy window is focused,
captures the most recently focused diffy window instead.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -gt 1 ]]; then
  usage >&2
  exit 1
fi

OUT_PATH="${1:-/tmp/diffy-live.png}"

if [[ "${OUT_PATH}" != /* ]]; then
  echo "capture path must be absolute: ${OUT_PATH}" >&2
  exit 1
fi

if ! command -v niri >/dev/null 2>&1; then
  echo "niri is required for compositor-native diffy capture" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for compositor-native diffy capture" >&2
  exit 1
fi

WINDOWS_JSON="$(niri msg -j windows)"

WINDOW_ID="$(
  printf '%s\n' "${WINDOWS_JSON}" | jq -r '
    ([.[] | select(.app_id == "diffy" and .is_focused) | .id] | first) //
    ([.[] | select(.app_id == "diffy")]
      | sort_by(.focus_timestamp.secs, .focus_timestamp.nanos)
      | last
      | .id) //
    empty
  '
)"

if [[ -z "${WINDOW_ID}" ]]; then
  echo "could not find a running diffy window in niri" >&2
  exit 1
fi

niri msg action screenshot-window --id "${WINDOW_ID}" --path "${OUT_PATH}"
printf '%s\n' "${OUT_PATH}"
