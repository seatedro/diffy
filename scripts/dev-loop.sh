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

: "${DIFFY_DEV_BUILD_DIR:=build/dev}"
: "${DIFFY_PREVIEW_BUILD_DIR:=build/preview}"
: "${DIFFY_DEV_REPO:=${HOME}/exa/monorepo-master}"
: "${DIFFY_DEV_LEFT:=master}"
: "${DIFFY_DEV_RIGHT:=rohit/apollo-servecontents-cutover}"
: "${DIFFY_DEV_LAYOUT:=split}"
: "${DIFFY_DEV_RENDERER:=}"
: "${DIFFY_DEV_FILE_INDEX:=0}"
: "${DIFFY_DEV_EXIT_AFTER_MS:=1400}"
: "${DIFFY_DEV_CAPTURE_DELAY_MS:=420}"
: "${DIFFY_DEV_PRINT_STATE_DELAY_MS:=260}"
: "${DIFFY_DEV_CAPTURE_PATH:=/tmp/diffy-dev/latest.png}"
: "${DIFFY_DEV_POLL_SECONDS:=0.5}"
: "${DIFFY_DEV_QMLLINT:=0}"
: "${DIFFY_DEV_SKIP_TESTS:=0}"
: "${DIFFY_DEV_SKIP_SMOKE:=0}"

state_dir="$(dirname "${DIFFY_DEV_CAPTURE_PATH}")"
capture_path="${DIFFY_DEV_CAPTURE_PATH}"
previous_capture_path="${state_dir}/previous.png"
stdout_log="${state_dir}/smoke.stdout.log"
stderr_log="${state_dir}/smoke.stderr.log"

resolve_path() {
  local value="$1"
  if [[ "${value}" == /* ]]; then
    printf '%s\n' "${value}"
  else
    printf '%s/%s\n' "${repo_root}" "${value}"
  fi
}

dev_build_dir="$(resolve_path "${DIFFY_DEV_BUILD_DIR}")"
preview_build_dir="$(resolve_path "${DIFFY_PREVIEW_BUILD_DIR}")"

mkdir -p "${dev_build_dir}" "${preview_build_dir}" "${state_dir}"

timestamp() {
  date +"%H:%M:%S"
}

log_step() {
  printf '[%s] %s\n' "$(timestamp)" "$*"
}

command_exists() {
  command -v "$1" >/dev/null 2>&1
}

configure_build() {
  local target_dir="$1"
  local qml_debug="$2"
  log_step "configuring ${target_dir}"
  cmake -S "${repo_root}" -B "${target_dir}" -G Ninja \
    -DCMAKE_BUILD_TYPE=Debug \
    -DDIFFY_ENABLE_QML_DEBUG="${qml_debug}" \
    -DDIFFY_FAST_QML_DEV=ON
}

run_qmllint() {
  if [[ "${DIFFY_DEV_QMLLINT}" != "1" ]]; then
    return
  fi
  log_step "running all_qmllint"
  cmake --build "${dev_build_dir}" --target all_qmllint
}

build_project() {
  local target_dir="$1"
  local target_name="${2:-}"
  log_step "building diffy"
  if [[ -n "${target_name}" ]]; then
    cmake --build "${target_dir}" --target "${target_name}"
  else
    cmake --build "${target_dir}"
  fi
}

run_tests() {
  if [[ "${DIFFY_DEV_SKIP_TESTS}" == "1" ]]; then
    return
  fi
  log_step "running ctest"
  ctest --test-dir "${dev_build_dir}" --output-on-failure
}

print_smoke_summary() {
  local state_line frame_hash previous_hash frame_status
  state_line="$(grep '^DIFFY_STATE ' "${stdout_log}" | tail -n 1 || true)"
  frame_hash=""
  previous_hash=""
  frame_status="missing"

  if [[ -f "${capture_path}" ]]; then
    frame_hash="$(sha256sum "${capture_path}" | awk '{print $1}')"
    frame_status="new"
    if [[ -f "${previous_capture_path}" ]]; then
      previous_hash="$(sha256sum "${previous_capture_path}" | awk '{print $1}')"
      if [[ "${frame_hash}" == "${previous_hash}" ]]; then
        frame_status="unchanged"
      else
        frame_status="changed"
      fi
    fi
  fi

  if [[ -n "${state_line}" ]]; then
    printf '%s\n' "${state_line}"
  else
    printf 'DIFFY_STATE missing\n'
  fi

  printf 'DIFFY_CAPTURE path=%s status=%s\n' "${capture_path}" "${frame_status}"
  printf 'DIFFY_LOGS stdout=%s stderr=%s\n' "${stdout_log}" "${stderr_log}"
}

run_smoke() {
  if [[ "${DIFFY_DEV_SKIP_SMOKE}" == "1" ]]; then
    return
  fi

  log_step "running smoke"
  local config_dir smoke_status
  config_dir="$(mktemp -d)"

  if [[ -f "${capture_path}" ]]; then
    cp "${capture_path}" "${previous_capture_path}"
  else
    rm -f "${previous_capture_path}"
  fi

  set +e
  (
    export QT_QPA_PLATFORM=offscreen
    export QT_QUICK_BACKEND=software
    export XDG_CONFIG_HOME="${config_dir}"
    export XDG_DATA_HOME="${config_dir}/data"
    export XDG_CACHE_HOME="${config_dir}/cache"
    export DIFFY_QML_SOURCE=1
    export DIFFY_START_REPO="${DIFFY_DEV_REPO}"
    export DIFFY_START_LEFT="${DIFFY_DEV_LEFT}"
    export DIFFY_START_RIGHT="${DIFFY_DEV_RIGHT}"
    export DIFFY_START_COMPARE=1
    export DIFFY_REQUIRE_RESULTS=1
    export DIFFY_PRINT_STATE=1
    export DIFFY_PRINT_STATE_DELAY_MS="${DIFFY_DEV_PRINT_STATE_DELAY_MS}"
    export DIFFY_FATAL_RUNTIME_WARNINGS=1
    export DIFFY_START_LAYOUT="${DIFFY_DEV_LAYOUT}"
    export DIFFY_START_FILE_INDEX="${DIFFY_DEV_FILE_INDEX}"
    export DIFFY_CAPTURE_DELAY_MS="${DIFFY_DEV_CAPTURE_DELAY_MS}"
    export DIFFY_CAPTURE_PATH="${capture_path}"
    export DIFFY_EXIT_AFTER_MS="${DIFFY_DEV_EXIT_AFTER_MS}"
    if [[ -n "${DIFFY_DEV_RENDERER}" ]]; then
      export DIFFY_START_RENDERER="${DIFFY_DEV_RENDERER}"
    fi
    "${dev_build_dir}/diffy"
  ) >"${stdout_log}" 2>"${stderr_log}"
  smoke_status=$?
  set -e

  rm -rf "${config_dir}"

  print_smoke_summary

  if [[ ${smoke_status} -ne 0 ]]; then
    printf '\nSmoke stdout:\n' >&2
    cat "${stdout_log}" >&2
    printf '\nSmoke stderr:\n' >&2
    cat "${stderr_log}" >&2
    return "${smoke_status}"
  fi
}

run_once() {
  configure_build "${dev_build_dir}" OFF
  run_qmllint
  build_project "${dev_build_dir}"
  run_tests
  run_smoke
}

watch_fingerprint() {
  (
    fd --type f . "${repo_root}/src" "${repo_root}/qml" "${repo_root}/tests" "${repo_root}/scripts"
    printf '%s\n' \
      "${repo_root}/CMakeLists.txt" \
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
    --watch "${repo_root}/tests" \
    --watch "${repo_root}/scripts" \
    --watch "${repo_root}/CMakeLists.txt" \
    --watch "${repo_root}/README.md" \
    --watch "${repo_root}/flake.nix" \
    --watch "${repo_root}/devenv.nix" \
    --watch "${repo_root}/.gitignore" \
    --exts cpp,h,hpp,c,qml,cmake,md,sh,nix \
    -- "${script_dir}/dev-loop.sh" once
}

run_watch() {
  if command_exists watchexec; then
    watch_with_watchexec
  else
    watch_with_polling
  fi
}

run_preview() {
  if ! command_exists qmlpreview; then
    printf 'qmlpreview is not available in PATH\n' >&2
    return 1
  fi

  configure_build "${preview_build_dir}" ON
  build_project "${preview_build_dir}" diffy

  log_step "starting qmlpreview"
  exec env \
    XDG_CONFIG_HOME="${state_dir}/preview-config" \
    XDG_DATA_HOME="${state_dir}/preview-data" \
    XDG_CACHE_HOME="${state_dir}/preview-cache" \
    DIFFY_QML_SOURCE=1 \
    DIFFY_START_REPO="${DIFFY_DEV_REPO}" \
    DIFFY_START_LEFT="${DIFFY_DEV_LEFT}" \
    DIFFY_START_RIGHT="${DIFFY_DEV_RIGHT}" \
    DIFFY_START_COMPARE=1 \
    DIFFY_REQUIRE_RESULTS=1 \
    DIFFY_START_LAYOUT="${DIFFY_DEV_LAYOUT}" \
    DIFFY_START_FILE_INDEX="${DIFFY_DEV_FILE_INDEX}" \
    qmlpreview "${preview_build_dir}/diffy"
}

print_help() {
  cat <<EOF
Usage: scripts/dev-loop.sh [once|watch|preview]

Commands:
  once     Configure, build, test, smoke, and capture one iteration.
  watch    Re-run the full loop on file changes.
  preview  Launch qmlpreview against a debug build with source-loaded QML.

Local overrides:
  ${env_file}

Useful variables:
  DIFFY_DEV_REPO=${DIFFY_DEV_REPO}
  DIFFY_DEV_BUILD_DIR=${DIFFY_DEV_BUILD_DIR}
  DIFFY_PREVIEW_BUILD_DIR=${DIFFY_PREVIEW_BUILD_DIR}
  DIFFY_DEV_LEFT=${DIFFY_DEV_LEFT}
  DIFFY_DEV_RIGHT=${DIFFY_DEV_RIGHT}
  DIFFY_DEV_LAYOUT=${DIFFY_DEV_LAYOUT}
  DIFFY_DEV_FILE_INDEX=${DIFFY_DEV_FILE_INDEX}
  DIFFY_DEV_CAPTURE_PATH=${DIFFY_DEV_CAPTURE_PATH}
  DIFFY_DEV_QMLLINT=${DIFFY_DEV_QMLLINT}
  DIFFY_DEV_SKIP_TESTS=${DIFFY_DEV_SKIP_TESTS}
  DIFFY_DEV_SKIP_SMOKE=${DIFFY_DEV_SKIP_SMOKE}
EOF
}

case "${1:-watch}" in
  once)
    run_once
    ;;
  watch)
    run_watch
    ;;
  preview)
    run_preview
    ;;
  help|-h|--help)
    print_help
    ;;
  *)
    print_help >&2
    exit 1
    ;;
esac
