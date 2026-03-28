#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
env_file="${DIFFY_DEV_ENV_FILE:-${repo_root}/.diffy-dev.env}"

if [[ -f "${env_file}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${env_file}"
  set +a
fi

: "${DIFFY_DEV_REPO:=${HOME}/exa/monorepo-master}"
: "${DIFFY_DEV_LEFT:=master}"
: "${DIFFY_DEV_RIGHT:=rohit/apollo-servecontents-cutover}"
: "${DIFFY_DEV_LAYOUT:=split}"
: "${DIFFY_DEV_RENDERER:=}"
: "${DIFFY_DEV_FILE_INDEX:=0}"
: "${DIFFY_DEV_EXIT_AFTER_MS:=1400}"
: "${DIFFY_DEV_POLL_SECONDS:=0.5}"
: "${DIFFY_DEV_SKIP_TESTS:=0}"
: "${DIFFY_DEV_SKIP_SMOKE:=0}"
: "${DIFFY_DEV_RELEASE:=0}"

timestamp() {
  date +"%H:%M:%S"
}

log_step() {
  printf '[%s] %s\n' "$(timestamp)" "$*"
}

command_exists() {
  command -v "$1" >/dev/null 2>&1
}

if [[ "${DIFFY_DEV_RELEASE}" == "1" ]]; then
  cargo_profile_args=(--release)
  diffy_exe="${repo_root}/target/release/diffy"
else
  cargo_profile_args=()
  diffy_exe="${repo_root}/target/debug/diffy"
fi

build_project() {
  log_step "building diffy"
  (cd "${repo_root}" && cargo build "${cargo_profile_args[@]}")
}

run_tests() {
  if [[ "${DIFFY_DEV_SKIP_TESTS}" == "1" ]]; then
    return
  fi
  log_step "running cargo test"
  (cd "${repo_root}" && cargo test "${cargo_profile_args[@]}")
}

run_smoke() {
  if [[ "${DIFFY_DEV_SKIP_SMOKE}" == "1" ]]; then
    return
  fi

  log_step "running smoke"
  (
    cd "${repo_root}"
    export DIFFY_REPO_ROOT="${repo_root}"
    export QT_QPA_PLATFORM=offscreen
    export QT_QUICK_BACKEND=software
    export DIFFY_START_REPO="${DIFFY_DEV_REPO}"
    export DIFFY_START_LEFT="${DIFFY_DEV_LEFT}"
    export DIFFY_START_RIGHT="${DIFFY_DEV_RIGHT}"
    export DIFFY_START_COMPARE=1
    export DIFFY_REQUIRE_RESULTS=1
    export DIFFY_START_LAYOUT="${DIFFY_DEV_LAYOUT}"
    export DIFFY_START_FILE_INDEX="${DIFFY_DEV_FILE_INDEX}"
    export DIFFY_EXIT_AFTER_MS="${DIFFY_DEV_EXIT_AFTER_MS}"
    if [[ -n "${DIFFY_DEV_RENDERER}" ]]; then
      export DIFFY_START_RENDERER="${DIFFY_DEV_RENDERER}"
    fi
    "${diffy_exe}"
  )
}

run_once() {
  build_project
  run_tests
  run_smoke
}

watch_fingerprint() {
  (
    find "${repo_root}/src" "${repo_root}/qml" "${repo_root}/scripts" -type f
    printf '%s\n' \
      "${repo_root}/Cargo.toml" \
      "${repo_root}/Cargo.lock" \
      "${repo_root}/README.md" \
      "${repo_root}/flake.nix" \
      "${repo_root}/devenv.nix" \
      "${repo_root}/.gitignore"
  ) | sort | xargs -r stat -c '%Y %n' | sha256sum | awk '{print $1}'
}

watch_with_polling() {
  log_step "watchexec not found; using polling fallback"
  local last_fingerprint current_fingerprint
  run_once || true
  last_fingerprint="$(watch_fingerprint)"
  while sleep "${DIFFY_DEV_POLL_SECONDS}"; do
    current_fingerprint="$(watch_fingerprint)"
    if [[ "${current_fingerprint}" != "${last_fingerprint}" ]]; then
      last_fingerprint="${current_fingerprint}"
      run_once || true
    fi
  done
}

watch_with_watchexec() {
  log_step "watching with watchexec"
  exec watchexec \
    --watch "${repo_root}/src" \
    --watch "${repo_root}/qml" \
    --watch "${repo_root}/scripts" \
    --watch "${repo_root}/Cargo.toml" \
    --watch "${repo_root}/Cargo.lock" \
    --watch "${repo_root}/README.md" \
    --watch "${repo_root}/flake.nix" \
    --watch "${repo_root}/devenv.nix" \
    --watch "${repo_root}/.gitignore" \
    --exts rs,qml,toml,md,sh,nix \
    -- "${script_dir}/dev-loop.sh" once
}

run_watch() {
  if command_exists watchexec; then
    watch_with_watchexec
  else
    watch_with_polling
  fi
}

usage() {
  cat <<EOF
Usage: scripts/dev-loop.sh [once|watch]

Commands:
  once   Build, test, and run the offscreen smoke scenario once.
  watch  Re-run the once workflow whenever Rust/QML/tooling files change.

Environment overrides:
  DIFFY_DEV_REPO
  DIFFY_DEV_LEFT
  DIFFY_DEV_RIGHT
  DIFFY_DEV_LAYOUT
  DIFFY_DEV_RENDERER
  DIFFY_DEV_FILE_INDEX
  DIFFY_DEV_EXIT_AFTER_MS
  DIFFY_DEV_SKIP_TESTS
  DIFFY_DEV_SKIP_SMOKE
  DIFFY_DEV_RELEASE
EOF
}

main() {
  local mode="${1:-watch}"
  case "${mode}" in
    once)
      run_once
      ;;
    watch)
      run_watch
      ;;
    -h|--help|help)
      usage
      ;;
    *)
      echo "Unknown mode: ${mode}" >&2
      usage >&2
      exit 1
      ;;
  esac
}

main "$@"
