#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "cargo-run-signed-macos: expected executable path from cargo" >&2
  exit 64
fi

exe="$1"
shift

identity="${DIFFY_CODESIGN_IDENTITY:-Diffy Dev}"

if [[ "${DIFFY_SKIP_CODESIGN:-0}" == "1" ]]; then
  exec "$exe" "$@"
fi

if [[ "$identity" == "-" ]]; then
  echo "cargo-run-signed-macos: ad-hoc signing will not keep Keychain access stable" >&2
  echo "cargo-run-signed-macos: set DIFFY_CODESIGN_IDENTITY to a real signing identity" >&2
  exit 65
fi

if ! security find-identity -v -p codesigning | grep -Fq "\"$identity\""; then
  echo "cargo-run-signed-macos: missing code-signing identity '$identity'" >&2
  echo "cargo-run-signed-macos: run scripts/setup-macos-dev-codesign.sh once" >&2
  exit 65
fi

/usr/bin/codesign --force --sign "$identity" --timestamp=none "$exe"
exec "$exe" "$@"
