#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage: scripts/install-macos-dev-app.sh [options] [-- diffy-args...]

Build and install a local "Diffy Dev.app" bundle for Computer Use and
accessibility testing. The bundle is separate from /Applications/Diffy.app.

options:
  --no-launch         install the bundle without opening it
  --release           package target/release/diffy instead of target/debug/diffy
  --install-dir PATH  install under PATH (default: ~/Applications)
  --app-path PATH     install to an exact .app path
  -h, --help          show this help
USAGE
}

[[ "$(uname -s)" == "Darwin" ]] || {
  echo "install-macos-dev-app: macOS only" >&2
  exit 64
}

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="${DIFFY_DEV_APP_NAME:-Diffy Dev}"
BUNDLE_ID="${DIFFY_DEV_BUNDLE_ID:-io.github.seatedro.diffy.dev}"
INSTALL_DIR="${DIFFY_DEV_INSTALL_DIR:-$HOME/Applications}"
APP_PATH="${DIFFY_DEV_APP_PATH:-}"
PROFILE="debug"
LAUNCH=1

configure_dev_linker() {
  [[ "${DIFFY_DEV_USE_DEFAULT_LINKER:-0}" != "1" ]] || return 0

  local developer_dir="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
  local xcode_clang="$developer_dir/Toolchains/XcodeDefault.xctoolchain/usr/bin/clang"
  local sdkroot clang_rt host_triple target_env target_cc_env sdk_version

  [[ -x "$xcode_clang" ]] || return 0
  sdkroot="$(env DEVELOPER_DIR="$developer_dir" /usr/bin/xcrun --sdk macosx --show-sdk-path)"
  clang_rt="$(find "$developer_dir/Toolchains/XcodeDefault.xctoolchain/usr/lib/clang" \
    -path '*/lib/darwin/libclang_rt.osx.a' -print | sort | tail -n1)"
  [[ -n "$sdkroot" && -n "$clang_rt" ]] || return 0

  host_triple="$(rustc -vV | awk '/^host: / { print $2 }')"
  target_env="$(printf '%s' "$host_triple" | tr '[:lower:]-' '[:upper:]_')"
  target_cc_env="$(printf '%s' "$host_triple" | tr '-' '_')"
  sdk_version="$(basename "$sdkroot" | sed -E 's/^MacOSX([0-9.]+)\.sdk$/\1/')"

  export DEVELOPER_DIR="$developer_dir"
  export SDKROOT="$sdkroot"
  export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-${DIFFY_DEV_MACOS_DEPLOYMENT_TARGET:-$sdk_version}}"
  export CC="$xcode_clang"
  export "CC_${target_cc_env}=$xcode_clang"
  export "CFLAGS_${target_cc_env}=-isysroot $sdkroot"
  export "CARGO_TARGET_${target_env}_LINKER=$xcode_clang"
  export "CARGO_TARGET_${target_env}_RUSTFLAGS=-C link-arg=-Wl,-syslibroot,$sdkroot -C link-arg=$clang_rt"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-launch)
      LAUNCH=0
      shift
      ;;
    --release)
      PROFILE="release"
      shift
      ;;
    --install-dir)
      [[ $# -ge 2 ]] || { echo "install-macos-dev-app: --install-dir needs a path" >&2; exit 64; }
      INSTALL_DIR="$2"
      shift 2
      ;;
    --app-path)
      [[ $# -ge 2 ]] || { echo "install-macos-dev-app: --app-path needs a path" >&2; exit 64; }
      APP_PATH="$2"
      shift 2
      ;;
    --)
      shift
      break
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      break
      ;;
  esac
done

if [[ -z "$APP_PATH" ]]; then
  APP_PATH="$INSTALL_DIR/$APP_NAME.app"
fi

cd "$ROOT_DIR"
configure_dev_linker

if [[ "$PROFILE" == "release" ]]; then
  cargo build --release --bin diffy
  BIN_PATH="$ROOT_DIR/target/release/diffy"
else
  cargo build --bin diffy
  BIN_PATH="$ROOT_DIR/target/debug/diffy"
fi

VERSION="$(cargo pkgid -p diffy | sed 's/.*#//')"
CONTENTS_DIR="$APP_PATH/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

rm -rf "$APP_PATH"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

cp "$BIN_PATH" "$MACOS_DIR/diffy-bin"
chmod +x "$MACOS_DIR/diffy-bin"

cat > "$MACOS_DIR/diffy-dev" <<EOF
#!/usr/bin/env bash
set -euo pipefail
DIR="\$(cd "\$(dirname "\$0")" && pwd)"
export DIFFY_APP_DISPLAY_NAME="\${DIFFY_APP_DISPLAY_NAME:-$APP_NAME}"
export DIFFY_WINDOW_TITLE_PREFIX="\${DIFFY_WINDOW_TITLE_PREFIX:-diffy dev}"
export DIFFY_DISABLE_KEYRING="\${DIFFY_DISABLE_KEYRING:-1}"
export DIFFY_DEV_GITHUB_TOKEN_FILE="\${DIFFY_DEV_GITHUB_TOKEN_FILE:-1}"
exec "\$DIR/diffy-bin" "\$@"
EOF
chmod +x "$MACOS_DIR/diffy-dev"

if [[ "${DIFFY_DEV_SKIP_ICON:-0}" != "1" ]]; then
  if bash "$ROOT_DIR/scripts/prepare-macos-icon-assets.sh"; then
    cp "$ROOT_DIR/assets/packaging/macos/generated/AppIcon.icns" "$RESOURCES_DIR/AppIcon.icns"
    if [[ -f "$ROOT_DIR/assets/packaging/macos/generated/Assets.car" ]]; then
      cp "$ROOT_DIR/assets/packaging/macos/generated/Assets.car" "$RESOURCES_DIR/Assets.car"
    fi
  else
    echo "install-macos-dev-app: icon generation failed; continuing without app icon" >&2
  fi
fi

cat > "$CONTENTS_DIR/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>$APP_NAME</string>
  <key>CFBundleExecutable</key>
  <string>diffy-dev</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleIconName</key>
  <string>AppIcon</string>
  <key>CFBundleIdentifier</key>
  <string>$BUNDLE_ID</string>
  <key>CFBundleName</key>
  <string>$APP_NAME</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
  <key>CFBundleVersion</key>
  <string>$VERSION</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

printf 'APPL????' > "$CONTENTS_DIR/PkgInfo"

CODESIGN_IDENTITY="${DIFFY_DEV_CODESIGN_IDENTITY:--}"
/usr/bin/codesign --force --deep --sign "$CODESIGN_IDENTITY" --timestamp=none "$APP_PATH" >/dev/null

LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [[ -x "$LSREGISTER" ]]; then
  "$LSREGISTER" -f "$APP_PATH" >/dev/null 2>&1 || true
fi

echo "installed $APP_PATH"
echo "bundle id: $BUNDLE_ID"
echo "window title prefix: diffy dev"

if [[ "$LAUNCH" == "1" ]]; then
  /usr/bin/open -n "$APP_PATH" --args "$@"
fi
