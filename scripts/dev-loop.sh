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

: "${DIFFY_DEV_REPO:=${repo_root}}"
: "${DIFFY_DEV_LEFT:=HEAD~1}"
: "${DIFFY_DEV_RIGHT:=HEAD}"
: "${DIFFY_DEV_COMPARE_MODE:=}"
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

apply_startup_env() {
  export DIFFY_START_REPO="${DIFFY_DEV_REPO}"
  export DIFFY_START_LEFT="${DIFFY_DEV_LEFT}"
  export DIFFY_START_RIGHT="${DIFFY_DEV_RIGHT}"
  export DIFFY_START_COMPARE=1
  export DIFFY_START_LAYOUT="${DIFFY_DEV_LAYOUT}"
  export DIFFY_START_FILE_INDEX="${DIFFY_DEV_FILE_INDEX}"

  if [[ -n "${DIFFY_DEV_COMPARE_MODE}" ]]; then
    export DIFFY_START_COMPARE_MODE="${DIFFY_DEV_COMPARE_MODE}"
  else
    unset DIFFY_START_COMPARE_MODE
  fi

  if [[ -n "${DIFFY_DEV_RENDERER}" ]]; then
    export DIFFY_START_RENDERER="${DIFFY_DEV_RENDERER}"
  else
    unset DIFFY_START_RENDERER
  fi
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
    apply_startup_env
    export DIFFY_REQUIRE_RESULTS=1
    export DIFFY_EXIT_AFTER_MS="${DIFFY_DEV_EXIT_AFTER_MS}"
    "${diffy_exe}"
  )
}

run_open() {
  build_project
  log_step "launching diffy"
  (
    cd "${repo_root}"
    apply_startup_env
    unset DIFFY_REQUIRE_RESULTS
    unset DIFFY_EXIT_AFTER_MS
    "${diffy_exe}"
  )
}

run_once() {
  build_project
  run_tests
  run_smoke
}

stat_mtime() {
  if stat --version >/dev/null 2>&1; then
    stat -c '%Y %n' "$1"
  else
    stat -f '%m %N' "$1"
  fi
}

sha256_stream() {
  if command_exists sha256sum; then
    sha256sum | awk '{print $1}'
  else
    shasum -a 256 | awk '{print $1}'
  fi
}

watch_fingerprint() {
  while IFS= read -r path; do
    [[ -e "${path}" ]] || continue
    stat_mtime "${path}"
  done < <(
    {
      find "${repo_root}/src" "${repo_root}/scripts" -type f
      printf '%s\n' \
        "${repo_root}/Cargo.toml" \
        "${repo_root}/Cargo.lock" \
        "${repo_root}/README.md" \
        "${repo_root}/flake.nix" \
        "${repo_root}/devenv.nix" \
        "${repo_root}/.gitignore"
    } | sort
  ) | sha256_stream
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
    --watch "${repo_root}/scripts" \
    --watch "${repo_root}/Cargo.toml" \
    --watch "${repo_root}/Cargo.lock" \
    --watch "${repo_root}/README.md" \
    --watch "${repo_root}/flake.nix" \
    --watch "${repo_root}/devenv.nix" \
    --watch "${repo_root}/.gitignore" \
    --exts rs,toml,md,sh,nix \
    -- "${script_dir}/dev-loop.sh" once
}

watch_open_with_watchexec() {
  log_step "watching and relaunching diffy with watchexec"
  exec watchexec \
    --restart \
    --watch "${repo_root}/src" \
    --watch "${repo_root}/scripts" \
    --watch "${repo_root}/Cargo.toml" \
    --watch "${repo_root}/Cargo.lock" \
    --watch "${repo_root}/README.md" \
    --watch "${repo_root}/flake.nix" \
    --watch "${repo_root}/devenv.nix" \
    --watch "${repo_root}/.gitignore" \
    --exts rs,toml,md,sh,nix \
    -- "${script_dir}/dev-loop.sh" open
}

run_watch() {
  if command_exists watchexec; then
    watch_with_watchexec
  else
    watch_with_polling
  fi
}

run_watch_open() {
  if command_exists watchexec; then
    watch_open_with_watchexec
  else
    log_step "watch-open requires watchexec for automatic restart"
    exit 1
  fi
}

usage() {
  cat <<EOF
Usage: scripts/dev-loop.sh [once|watch|open|watch-open]

Commands:
  once   Build, test, and run the offscreen smoke scenario once.
  watch  Re-run the once workflow whenever Rust/tooling files change.
  open   Build and launch diffy directly into the configured compare.
  watch-open
         Rebuild and relaunch the visible app whenever files change.

Environment overrides:
  DIFFY_DEV_REPO
  DIFFY_DEV_LEFT
  DIFFY_DEV_RIGHT
  DIFFY_DEV_COMPARE_MODE
  DIFFY_DEV_LAYOUT
  DIFFY_DEV_RENDERER
  DIFFY_DEV_FILE_INDEX
  DIFFY_DEV_EXIT_AFTER_MS
  DIFFY_DEV_SKIP_TESTS
  DIFFY_DEV_SKIP_SMOKE
  DIFFY_DEV_RELEASE
  DIFFY_DEV_ENV_FILE
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
    open)
      run_open
      ;;
    watch-open)
      run_watch_open
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
