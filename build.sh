#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${script_dir}"

action="build"
preset="Release"
fresh=()

usage() {
  cat <<EOF
Usage:
  ./build.sh [--preset Debug|Release] [--fresh]
  ./build.sh configure [--preset Debug|Release] [--fresh]
  ./build.sh test [--preset Debug|Release] [--fresh]
  ./build.sh run [--preset Debug|Release] [--fresh]

Notes:
  - Defaults to Release.
  - Uses the existing CMake presets in this repo.
  - Expects your Unix build dependencies to already be available in the environment.
EOF
}

while (($#)); do
  case "$1" in
    build|configure|test|run)
      action="$1"
      shift
      ;;
    --preset)
      if (($# < 2)); then
        echo "--preset requires a value." >&2
        echo >&2
        usage >&2
        exit 1
      fi
      preset="$2"
      shift 2
      ;;
    --fresh)
      fresh=(--fresh)
      shift
      ;;
    -h|--help|help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      echo >&2
      usage >&2
      exit 1
      ;;
  esac
done

do_configure() {
  echo "[diffy] Configuring ${preset}..."
  cmake --preset "${preset}" "${fresh[@]}"
}

do_build() {
  do_configure
  echo "[diffy] Building ${preset}..."
  cmake --build --preset "${preset}"
}

do_test() {
  do_build
  echo "[diffy] Testing ${preset}..."
  ctest --preset "${preset}"
}

do_run() {
  local diffy_exe="build/${preset}/diffy"
  if [[ ! -x "${diffy_exe}" ]]; then
    echo "Could not find ${diffy_exe}" >&2
    echo "Run ./build.sh --preset ${preset} first." >&2
    exit 1
  fi

  echo "[diffy] Running ${diffy_exe}..."
  "./${diffy_exe}"
}

case "${action}" in
  configure)
    do_configure
    ;;
  build)
    do_build
    ;;
  test)
    do_test
    ;;
  run)
    do_run
    ;;
  *)
    echo "Unknown action: ${action}" >&2
    exit 1
    ;;
esac
