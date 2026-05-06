#!/usr/bin/env bash
# Diffy installer for macOS and Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/seatedro/diffy/master/scripts/install.sh | bash
#
# Flags (env or args):
#   --version v1.2.3      install a specific tag (default: latest)
#   --prefix /path        install root (default: /Applications on macOS,
#                         $XDG_DATA_HOME/diffy or ~/.local/share/diffy on Linux)
#   --no-path             skip PATH hint on Linux

set -euo pipefail

REPO="seatedro/diffy"
APP_NAME="Diffy"
BIN_NAME="diffy"

VERSION="${DIFFY_VERSION:-}"
PREFIX="${DIFFY_PREFIX:-}"
MODIFY_PATH=true

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      grep -E '^#( |$)' "$0" | sed -E 's/^# ?//'
      exit 0
      ;;
    -v|--version)
      VERSION="${2:-}"; shift 2
      [[ -n "$VERSION" ]] || { echo "error: --version requires a value" >&2; exit 1; }
      ;;
    --prefix)
      PREFIX="${2:-}"; shift 2
      [[ -n "$PREFIX" ]] || { echo "error: --prefix requires a path" >&2; exit 1; }
      ;;
    --no-path)
      MODIFY_PATH=false; shift
      ;;
    *)
      echo "warning: unknown option '$1'" >&2; shift
      ;;
  esac
done

if [[ -t 1 ]]; then
  C_RESET=$'\033[0m'; C_DIM=$'\033[2m'; C_RED=$'\033[31m'
  C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_BOLD=$'\033[1m'
else
  C_RESET=; C_DIM=; C_RED=; C_GREEN=; C_YELLOW=; C_BOLD=
fi

info()    { printf '%s==>%s %s\n' "$C_GREEN" "$C_RESET" "$*"; }
warn()    { printf '%s!!%s %s\n'  "$C_YELLOW" "$C_RESET" "$*" >&2; }
error()   { printf '%serror:%s %s\n' "$C_RED" "$C_RESET" "$*" >&2; exit 1; }
hint()    { printf '%s   %s%s\n' "$C_DIM" "$*" "$C_RESET"; }

raw_os="$(uname -s)"
case "$raw_os" in
  Darwin*) OS=darwin ;;
  Linux*)  OS=linux ;;
  MINGW*|MSYS*|CYGWIN*)
    error "Windows detected — use the PowerShell installer instead:
   powershell -c \"irm https://raw.githubusercontent.com/${REPO}/master/scripts/install.ps1 | iex\""
    ;;
  *) error "unsupported OS: $raw_os" ;;
esac

raw_arch="$(uname -m)"
case "$raw_arch" in
  arm64|aarch64) ARCH=aarch64 ;;
  x86_64|amd64)  ARCH=x64 ;;
  *) error "unsupported architecture: $raw_arch" ;;
esac

if [[ "$OS" == "darwin" && "$ARCH" == "x64" ]]; then
  if [[ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" == "1" ]]; then
    info "Rosetta detected — installing Apple Silicon build"
    ARCH=aarch64
  fi
fi

for tool in curl; do
  command -v "$tool" >/dev/null 2>&1 || error "missing required tool: $tool"
done

if [[ -z "$VERSION" ]]; then
  info "resolving latest release"
  # /releases/latest excludes prereleases — fall back to /releases[0] which
  # returns the most recent tag regardless of prerelease state. `|| true`
  # is required: under `set -eo pipefail` the 404 would kill the script
  # before we reach the fallback.
  VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
    | awk -F'"' '/"tag_name":/ {print $4; exit}' || true)"
  if [[ -z "$VERSION" ]]; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases?per_page=1" 2>/dev/null \
      | awk -F'"' '/"tag_name":/ {print $4; exit}' || true)"
  fi
  [[ -n "$VERSION" ]] || error "could not determine latest release tag"
fi
NUMERIC_VERSION="${VERSION#v}"
# cargo-packager uses the Cargo.toml package version in asset filenames, not
# the git tag; strip any prerelease suffix ("1.2.3-rc.5" -> "1.2.3").
PACKAGE_VERSION="${NUMERIC_VERSION%%-*}"

# Linux asset names use arch strings that differ from darwin.
case "$OS" in
  darwin)
    ASSET="${APP_NAME}_${PACKAGE_VERSION}_${ARCH}.dmg"
    ;;
  linux)
    case "$ARCH" in
      x64)     LINUX_ARCH=x86_64 ;;
      aarch64) LINUX_ARCH=aarch64 ;;
    esac
    ASSET="${BIN_NAME}_${PACKAGE_VERSION}_${LINUX_ARCH}.AppImage"
    ;;
esac
ASSET_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"

TMP_DIR="$(mktemp -d -t diffy-install.XXXXXXXX)"
MOUNT_POINT=""
cleanup() {
  if [[ -n "$MOUNT_POINT" && -d "$MOUNT_POINT" ]]; then
    hdiutil detach "$MOUNT_POINT" -quiet -force >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

ASSET_PATH="$TMP_DIR/$ASSET"
info "downloading ${C_BOLD}${ASSET}${C_RESET} (${VERSION})"
curl -fL --progress-bar -o "$ASSET_PATH" "$ASSET_URL" \
  || error "download failed — is '${VERSION}' a published release?
   see https://github.com/${REPO}/releases"

if command -v shasum >/dev/null 2>&1 || command -v sha256sum >/dev/null 2>&1; then
  SUMS_URL="https://github.com/${REPO}/releases/download/${VERSION}/SHA256SUMS"
  if curl -fsSL -o "$TMP_DIR/SHA256SUMS" "$SUMS_URL" 2>/dev/null; then
    info "verifying checksum"
    if command -v shasum >/dev/null 2>&1; then
      (cd "$TMP_DIR" && shasum -a 256 -c --ignore-missing --status SHA256SUMS) \
        || error "checksum mismatch for ${ASSET}"
    else
      (cd "$TMP_DIR" && sha256sum --check --ignore-missing --status SHA256SUMS) \
        || error "checksum mismatch for ${ASSET}"
    fi
  else
    warn "SHA256SUMS not published for ${VERSION}; skipping verification"
  fi
fi

install_macos() {
  : "${PREFIX:=/Applications}"
  local dest="${PREFIX}/${APP_NAME}.app"

  command -v hdiutil >/dev/null 2>&1 || error "hdiutil not found (required on macOS)"

  info "mounting disk image"
  MOUNT_POINT="$TMP_DIR/mount"
  mkdir -p "$MOUNT_POINT"
  hdiutil attach "$ASSET_PATH" -mountpoint "$MOUNT_POINT" -nobrowse -quiet -readonly

  local src="$MOUNT_POINT/${APP_NAME}.app"
  [[ -d "$src" ]] || error "${APP_NAME}.app not found inside DMG"

  if [[ -d "$dest" ]]; then
    if pgrep -f "${dest}/Contents/MacOS/" >/dev/null 2>&1; then
      error "${APP_NAME} is running — quit it and re-run this installer"
    fi
    info "removing existing ${dest}"
    rm -rf "$dest" 2>/dev/null || sudo rm -rf "$dest"
  fi

  info "installing to ${dest}"
  ditto "$src" "$dest" 2>/dev/null || sudo ditto "$src" "$dest"

  hdiutil detach "$MOUNT_POINT" -quiet -force >/dev/null 2>&1 || true
  MOUNT_POINT=""

  info "${C_BOLD}${C_GREEN}installed${C_RESET} ${APP_NAME} ${VERSION} → ${dest}"
  hint "launch: open -a ${APP_NAME}"
}

install_linux() {
  if [[ -z "$PREFIX" ]]; then
    PREFIX="${XDG_DATA_HOME:-$HOME/.local/share}/diffy"
  fi
  local bin_dir
  if [[ "$PREFIX" == "$HOME"* ]]; then
    bin_dir="$HOME/.local/bin"
  else
    bin_dir="${PREFIX}/bin"
  fi
  local dest_bin="${bin_dir}/${BIN_NAME}"
  local dest_img="${PREFIX}/${BIN_NAME}.AppImage"

  mkdir -p "$PREFIX" "$bin_dir"

  if [[ -f "$dest_bin" ]] && pgrep -x "$BIN_NAME" >/dev/null 2>&1; then
    error "${BIN_NAME} is running — quit it and re-run this installer"
  fi

  info "installing to ${dest_img}"
  install -m 0755 "$ASSET_PATH" "$dest_img"
  ln -sf "$dest_img" "$dest_bin"

  info "${C_BOLD}${C_GREEN}installed${C_RESET} ${BIN_NAME} ${VERSION} → ${dest_img}"

  if [[ "$MODIFY_PATH" == "true" ]] && ! command -v "$BIN_NAME" >/dev/null 2>&1; then
    case ":${PATH}:" in
      *":${bin_dir}:"*) ;;
      *)
        warn "${bin_dir} is not on your PATH"
        hint "add this line to your shell config (~/.bashrc, ~/.zshrc, etc.):"
        hint "  export PATH=\"${bin_dir}:\$PATH\""
        ;;
    esac
  fi

  hint "launch: ${BIN_NAME}  (or: ${dest_img})"
}

case "$OS" in
  darwin) install_macos ;;
  linux)  install_linux ;;
esac
