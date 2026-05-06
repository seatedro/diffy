#!/usr/bin/env bash
set -euo pipefail

APP_NAME="Diffy"
APP_BUNDLE_NAME="${APP_NAME}.app"
PACKAGES_DIR="dist/packages"
PACKAGE_ARCH="${DIFFY_MACOS_PACKAGE_ARCH:-$(uname -m)}"

die() {
  echo "package-macos-release: $*" >&2
  exit 1
}

require_env() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    die "${name} is required"
  fi
}

artifact_arch() {
  case "$PACKAGE_ARCH" in
    arm64 | aarch64) echo "aarch64" ;;
    x86_64 | amd64 | x64) echo "x64" ;;
    *) die "unsupported macOS package arch: ${PACKAGE_ARCH}" ;;
  esac
}

package_version() {
  node -e '
    const fs = require("fs");
    const toml = fs.readFileSync("Cargo.toml", "utf8");
    const match = toml.match(/\[workspace\.package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/);
    if (!match) process.exit(1);
    process.stdout.write(match[1]);
  '
}

write_notary_key() {
  local key_path="$1"

  if [[ -n "${APPLE_NOTARY_PRIVATE_KEY:-}" ]]; then
    printf '%s' "$APPLE_NOTARY_PRIVATE_KEY" > "$key_path"
  elif [[ -n "${APPLE_NOTARY_PRIVATE_KEY_BASE64:-}" ]]; then
    printf '%s' "$APPLE_NOTARY_PRIVATE_KEY_BASE64" | base64 -d > "$key_path"
  else
    die "APPLE_NOTARY_PRIVATE_KEY or APPLE_NOTARY_PRIVATE_KEY_BASE64 is required"
  fi

  chmod 600 "$key_path"
}

submit_for_notarization() {
  local path="$1"

  xcrun notarytool submit "$path" \
    --key "$notary_key" \
    --key-id "$APPLE_NOTARY_KEY_ID" \
    --issuer "$APPLE_NOTARY_ISSUER_ID" \
    --wait
}

sign_macho_files() {
  local app="$1"

  while IFS= read -r -d '' path; do
    if file "$path" | grep -q 'Mach-O'; then
      /usr/bin/codesign \
        --force \
        --timestamp \
        --options runtime \
        --sign "$APPLE_CODESIGN_IDENTITY" \
        "$path"
    fi
  done < <(find "$app/Contents" -type f -print0)
}

require_env APPLE_CODESIGN_IDENTITY
require_env APPLE_NOTARY_KEY_ID
require_env APPLE_NOTARY_ISSUER_ID

[[ "$(uname -s)" == "Darwin" ]] || die "macOS packaging must run on macOS"
command -v cargo >/dev/null 2>&1 || die "cargo not found"
command -v node >/dev/null 2>&1 || die "node not found"
xcrun --find notarytool >/dev/null
xcrun --find stapler >/dev/null

version="$(package_version)"
arch="$(artifact_arch)"
work_dir="$(mktemp -d)"
notary_key="${work_dir}/AuthKey_${APPLE_NOTARY_KEY_ID}.p8"
app_zip="${work_dir}/${APP_NAME}-${version}-${arch}.zip"
stage_dir="${work_dir}/stage"
trap 'rm -rf "$work_dir"' EXIT
write_notary_key "$notary_key"

cargo packager --release --formats app

app_path="$(find "$PACKAGES_DIR" -maxdepth 1 -type d -name "$APP_BUNDLE_NAME" -print -quit)"
if [[ -z "$app_path" ]]; then
  app_path="$(find "$PACKAGES_DIR" -maxdepth 1 -type d -name '*.app' -print -quit)"
fi
[[ -n "$app_path" && -d "$app_path" ]] || die "no .app bundle found in ${PACKAGES_DIR}"

sign_macho_files "$app_path"
/usr/bin/codesign \
  --force \
  --timestamp \
  --options runtime \
  --sign "$APPLE_CODESIGN_IDENTITY" \
  "$app_path"
/usr/bin/codesign --verify --deep --strict --verbose=2 "$app_path"

ditto -c -k --keepParent "$app_path" "$app_zip"
submit_for_notarization "$app_zip"
xcrun stapler staple "$app_path"
xcrun stapler validate "$app_path"

mkdir -p "$stage_dir"
ditto "$app_path" "${stage_dir}/${APP_BUNDLE_NAME}"
ln -s /Applications "${stage_dir}/Applications"

dmg_path="${PACKAGES_DIR}/${APP_NAME}_${version}_${arch}.dmg"
rm -f "$dmg_path"
hdiutil create \
  -volname "$APP_NAME" \
  -srcfolder "$stage_dir" \
  -ov \
  -format UDZO \
  "$dmg_path"

/usr/bin/codesign --force --timestamp --sign "$APPLE_CODESIGN_IDENTITY" "$dmg_path"
/usr/bin/codesign --verify --verbose=2 "$dmg_path"

submit_for_notarization "$dmg_path"
xcrun stapler staple "$dmg_path"
xcrun stapler validate "$dmg_path"
spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg_path"
spctl --assess --type execute --verbose=2 "$app_path"

echo "package-macos-release: wrote ${dmg_path}"
