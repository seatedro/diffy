#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGING_DIR="$ROOT_DIR/assets/packaging"
ICON_SOURCE="$PACKAGING_DIR/AppIcon.icon"
PNG_DIR="$PACKAGING_DIR/png"
GENERATED_DIR="$PACKAGING_DIR/macos/generated"
APP_ICON_NAME="AppIcon"
DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-${DIFFY_MACOS_DEPLOYMENT_TARGET:-11.0}}"
REQUIRE_MODERN_ICON="${DIFFY_REQUIRE_MODERN_MAC_ICON:-0}"

ICONUTIL="/usr/bin/iconutil"
SYSTEM_XCRUN="/usr/bin/xcrun"
ICTOOL_DEFAULT="/Applications/Icon Composer.app/Contents/Executables/ictool"

warn() { echo "prepare-macos-icon-assets: $*" >&2; }
die() { warn "$@"; exit 1; }

is_nix_path() {
  [[ -n "${1:-}" && "$1" == /nix/store/* ]]
}

# Invoke the system xcrun, bypassing a Nix-provided DEVELOPER_DIR/SDKROOT
# that shadows the real Apple toolchain.
system_xcrun() {
  env -u DEVELOPER_DIR -u SDKROOT "$SYSTEM_XCRUN" "$@"
}

find_ictool() {
  if [[ -n "${DIFFY_ICTOOL:-}" && -x "$DIFFY_ICTOOL" ]]; then
    printf '%s\n' "$DIFFY_ICTOOL"
    return 0
  fi
  if [[ -x "$ICTOOL_DEFAULT" ]]; then
    printf '%s\n' "$ICTOOL_DEFAULT"
    return 0
  fi
  local mdfound
  if mdfound="$(mdfind "kMDItemFSName == 'Icon Composer.app'" 2>/dev/null | head -n1)"; then
    if [[ -n "$mdfound" && -x "$mdfound/Contents/Executables/ictool" ]]; then
      printf '%s\n' "$mdfound/Contents/Executables/ictool"
      return 0
    fi
  fi
  return 1
}

# Find actool from a *real* full Xcode install, not Nix's xcbuild stub and
# not the Command Line Tools (which don't ship actool).
find_actool() {
  local candidate developer_dir

  if [[ -n "${DIFFY_APPLE_DEVELOPER_DIR:-}" ]] && ! is_nix_path "$DIFFY_APPLE_DEVELOPER_DIR" \
      && [[ -x "$DIFFY_APPLE_DEVELOPER_DIR/usr/bin/actool" ]]; then
    printf '%s\n' "$DIFFY_APPLE_DEVELOPER_DIR/usr/bin/actool"
    return 0
  fi

  if [[ -n "${DEVELOPER_DIR:-}" ]] && ! is_nix_path "$DEVELOPER_DIR" \
      && [[ -x "$DEVELOPER_DIR/usr/bin/actool" ]]; then
    printf '%s\n' "$DEVELOPER_DIR/usr/bin/actool"
    return 0
  fi

  for developer_dir in \
      /Applications/Xcode.app/Contents/Developer \
      /Applications/Xcode-beta.app/Contents/Developer \
      /Applications/Xcode*.app/Contents/Developer; do
    if [[ -x "$developer_dir/usr/bin/actool" ]]; then
      printf '%s\n' "$developer_dir/usr/bin/actool"
      return 0
    fi
  done

  if candidate="$(system_xcrun --find actool 2>/dev/null)" && [[ -x "$candidate" ]]; then
    printf '%s\n' "$candidate"
    return 0
  fi

  return 1
}

# Render a single PNG at the given pixel size from AppIcon.icon using ictool.
render_icon_png() {
  local ictool="$1" out="$2" base="$3" scale="$4"
  if "$ictool" "$ICON_SOURCE" \
    --export-image \
    --output-file "$out" \
    --platform macOS \
    --rendition Default \
    --width "$base" \
    --height "$base" \
    --scale "$scale" >/dev/null 2>/dev/null; then
    return 0
  fi

  "$ictool" "$ICON_SOURCE" \
    --export-preview macOS Default "$base" "$base" "$scale" "$out" >/dev/null
}

# Build AppIcon.icns by rendering every required size from AppIcon.icon
# (glass-layered, matches the modern icon) and running iconutil.
build_icns_from_icon_source() {
  local ictool="$1"
  local iconset_dir="$GENERATED_DIR/${APP_ICON_NAME}.iconset"
  rm -rf "$iconset_dir"
  mkdir -p "$iconset_dir"

  # Pairs of: base-point-size scale output-filename
  local renditions=(
    "16  1 icon_16x16.png"
    "16  2 icon_16x16@2x.png"
    "32  1 icon_32x32.png"
    "32  2 icon_32x32@2x.png"
    "128 1 icon_128x128.png"
    "128 2 icon_128x128@2x.png"
    "256 1 icon_256x256.png"
    "256 2 icon_256x256@2x.png"
    "512 1 icon_512x512.png"
    "512 2 icon_512x512@2x.png"
  )

  local entry base scale name
  for entry in "${renditions[@]}"; do
    read -r base scale name <<<"$entry"
    render_icon_png "$ictool" "$iconset_dir/$name" "$base" "$scale"
  done

  "$ICONUTIL" --convert icns --output "$GENERATED_DIR/${APP_ICON_NAME}.icns" "$iconset_dir"
}

# Fallback when Icon Composer isn't available: build .icns from the raw
# pre-rendered PNGs under assets/packaging/png/. This loses glass fidelity
# but keeps the pipeline working on stripped-down machines.
build_icns_from_flat_pngs() {
  local iconset_dir="$GENERATED_DIR/${APP_ICON_NAME}.iconset"
  rm -rf "$iconset_dir"
  mkdir -p "$iconset_dir"

  cp "$PNG_DIR/diffy-16.png"   "$iconset_dir/icon_16x16.png"
  cp "$PNG_DIR/diffy-32.png"   "$iconset_dir/icon_16x16@2x.png"
  cp "$PNG_DIR/diffy-32.png"   "$iconset_dir/icon_32x32.png"
  cp "$PNG_DIR/diffy-64.png"   "$iconset_dir/icon_32x32@2x.png"
  cp "$PNG_DIR/diffy-128.png"  "$iconset_dir/icon_128x128.png"
  cp "$PNG_DIR/diffy-256.png"  "$iconset_dir/icon_128x128@2x.png"
  cp "$PNG_DIR/diffy-256.png"  "$iconset_dir/icon_256x256.png"
  cp "$PNG_DIR/diffy-512.png"  "$iconset_dir/icon_256x256@2x.png"
  cp "$PNG_DIR/diffy-512.png"  "$iconset_dir/icon_512x512.png"
  cp "$PNG_DIR/diffy-1024.png" "$iconset_dir/icon_512x512@2x.png"

  "$ICONUTIL" --convert icns --output "$GENERATED_DIR/${APP_ICON_NAME}.icns" "$iconset_dir"
}

write_info_plist() {
  local plist_path="$GENERATED_DIR/Info.plist"
  cat > "$plist_path" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIconFile</key>
  <string>${APP_ICON_NAME}</string>
EOF

  if [[ -f "$GENERATED_DIR/Assets.car" ]]; then
    cat >> "$plist_path" <<EOF
  <key>CFBundleIconName</key>
  <string>${APP_ICON_NAME}</string>
EOF
  fi

  cat >> "$plist_path" <<'EOF'
</dict>
</plist>
EOF
}

compile_modern_assets_car() {
  local actool
  if ! actool="$(find_actool)"; then
    if [[ "$REQUIRE_MODERN_ICON" == "1" ]]; then
      die "full Xcode is required to compile ${APP_ICON_NAME}.icon into Assets.car (install Xcode.app from the App Store)"
    fi
    warn "actool not available (full Xcode.app not installed); skipping Assets.car — modern glass rendering disabled, .icns fallback will be used"
    return 0
  fi

  warn "using actool at: $actool"

  local xcassets_dir="$GENERATED_DIR/Assets.xcassets"
  local compile_dir="$GENERATED_DIR/actool-output"
  local partial_plist="$GENERATED_DIR/actool-info.plist"
  rm -rf "$xcassets_dir" "$compile_dir" "$partial_plist"
  mkdir -p "$xcassets_dir" "$compile_dir"

  cat > "$xcassets_dir/Contents.json" <<'EOF'
{
  "info" : {
    "author" : "io.github.seatedro.diffy",
    "version" : 1
  }
}
EOF

  if ! env -u DEVELOPER_DIR -u SDKROOT "$actool" \
      "$ICON_SOURCE" \
      "$xcassets_dir" \
      --compile "$compile_dir" \
      --output-format human-readable-text \
      --notices \
      --warnings \
      --app-icon "$APP_ICON_NAME" \
      --standalone-icon-behavior all \
      --target-device mac \
      --minimum-deployment-target "$DEPLOYMENT_TARGET" \
      --platform macosx \
      --output-partial-info-plist "$partial_plist"; then
    if [[ "$REQUIRE_MODERN_ICON" == "1" ]]; then
      die "actool failed to compile ${APP_ICON_NAME}.icon"
    fi
    warn "actool failed; continuing with .icns fallback only"
    return 0
  fi

  if [[ -f "$compile_dir/Assets.car" ]]; then
    cp "$compile_dir/Assets.car" "$GENERATED_DIR/Assets.car"
  elif [[ "$REQUIRE_MODERN_ICON" == "1" ]]; then
    die "actool succeeded but did not emit Assets.car"
  else
    warn "actool did not emit Assets.car; continuing with .icns fallback only"
    return 0
  fi

  # If actool dropped its own .icns (with the modern representations baked
  # in) use it — it's strictly better than anything we can hand-roll.
  local compiled_icns
  compiled_icns="$(find "$compile_dir" -name '*.icns' -print -quit)"
  if [[ -n "$compiled_icns" ]]; then
    cp "$compiled_icns" "$GENERATED_DIR/${APP_ICON_NAME}.icns"
  fi
}

main() {
  [[ "$(uname -s)" == "Darwin" ]] || exit 0
  [[ -x "$ICONUTIL" ]] || die "missing $ICONUTIL (Command Line Tools not installed?)"
  [[ -d "$ICON_SOURCE" ]] || die "missing icon source at $ICON_SOURCE"

  mkdir -p "$GENERATED_DIR"
  rm -f "$GENERATED_DIR/Assets.car" \
        "$GENERATED_DIR/${APP_ICON_NAME}.icns" \
        "$GENERATED_DIR/Info.plist"

  local ictool
  if ictool="$(find_ictool)"; then
    warn "using ictool at: $ictool"
    build_icns_from_icon_source "$ictool"
  else
    warn "ictool not found (install Icon Composer.app for glass-rendered .icns); falling back to flat PNG resize"
    build_icns_from_flat_pngs
  fi

  compile_modern_assets_car
  write_info_plist
}

main "$@"
