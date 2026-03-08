#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/perf-matrix.sh [options]

Run a repeatable Diffy performance scenario matrix and summarize DIFFY_STATE timings.

Options:
  --binary PATH              Diffy binary to execute.
                             Default: ./build/Release/diffy if present, otherwise ./build/diffy
  --runs N                   Number of repetitions per scenario. Default: 5
  --repo PATH                Repository for real-repo scenarios.
                             Default: ~/exa/monorepo-master
  --left REF                 Left revision. Default: master
  --right REF                Right revision. Default: rohit/apollo-servecontents-cutover
  --compare-mode MODE        Compare mode. Default: three-dot
  --unified-file PATH        File for cold unified scenario.
  --scroll-file PATH         File for unified vertical scroll scenario.
  --split-file PATH          File for split horizontal and layout switch scenarios.
  --switch-from-file PATH    Starting file for file-switch scenario.
  --switch-to-file PATH      Target file for file-switch scenario.
  --scenario NAME            Run a single scenario instead of the full matrix.
  --keep-workdir             Keep raw stdout/stderr/state files under /tmp.
  -h, --help                 Show this help.

Scenarios:
  cold-unified
  cold-split
  warm-vertical-scroll
  warm-split-horizontal
  file-switch
  layout-switch
  long-line-split-scroll
EOF
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ -x "${repo_root}/build/Release/diffy" ]]; then
  binary="${repo_root}/build/Release/diffy"
else
  binary="${repo_root}/build/diffy"
fi

runs=5
repo="${HOME}/exa/monorepo-master"
left_ref="master"
right_ref="rohit/apollo-servecontents-cutover"
compare_mode="three-dot"
unified_file="rust/services/apollo/src/main.rs"
scroll_file="rust/services/apollo/src/contents/get_contents.rs"
split_file="rust/services/apollo/crates/e2e-test/src/main.rs"
switch_from_file="rust/services/apollo/src/config.rs"
switch_to_file="rust/services/apollo/src/contents/get_contents.rs"
requested_scenario=""
keep_workdir=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary)
      binary="$2"
      shift 2
      ;;
    --runs)
      runs="$2"
      shift 2
      ;;
    --repo)
      repo="$2"
      shift 2
      ;;
    --left)
      left_ref="$2"
      shift 2
      ;;
    --right)
      right_ref="$2"
      shift 2
      ;;
    --compare-mode)
      compare_mode="$2"
      shift 2
      ;;
    --unified-file)
      unified_file="$2"
      shift 2
      ;;
    --scroll-file)
      scroll_file="$2"
      shift 2
      ;;
    --split-file)
      split_file="$2"
      shift 2
      ;;
    --switch-from-file)
      switch_from_file="$2"
      shift 2
      ;;
    --switch-to-file)
      switch_to_file="$2"
      shift 2
      ;;
    --scenario)
      requested_scenario="$2"
      shift 2
      ;;
    --keep-workdir)
      keep_workdir=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ ! -x "$binary" ]]; then
  echo "Diffy binary is not executable: $binary" >&2
  exit 1
fi

if ! [[ "$runs" =~ ^[1-9][0-9]*$ ]]; then
  echo "--runs must be a positive integer" >&2
  exit 1
fi

work_dir="$(mktemp -d /tmp/diffy-perf-matrix-XXXXXX)"
cleanup() {
  if [[ "$keep_workdir" != true ]]; then
    rm -rf "$work_dir"
  fi
}
trap cleanup EXIT

long_repo=""

create_long_line_repo() {
  if [[ -n "$long_repo" ]]; then
    return
  fi

  long_repo="$(mktemp -d /tmp/diffy-perf-longline-XXXXXX)"
  mkdir -p "${long_repo}/src"
  git -C "$long_repo" init >/dev/null

  local initial updated
  initial="$(printf '#include <string>\n\nconst char* payload = "%s";\nint width() {\n  return 1;\n}\n' "$(printf 'a%.0s' $(seq 1 4096))")"
  updated="$(printf '#include <string>\n\nconst char* payload = "%s";\nint width() {\n  return 2;\n}\n' "$(printf 'b%.0s' $(seq 1 4096))")"

  printf '%s' "$initial" > "${long_repo}/src/long.cpp"
  git -C "$long_repo" add src/long.cpp >/dev/null
  git -C "$long_repo" -c user.name=diffy -c user.email=diffy@example.com commit -m initial >/dev/null

  printf '%s' "$updated" > "${long_repo}/src/long.cpp"
  git -C "$long_repo" add src/long.cpp >/dev/null
  git -C "$long_repo" -c user.name=diffy -c user.email=diffy@example.com commit -m update >/dev/null
}

extract_metric() {
  local line="$1"
  local key="$2"
  printf '%s\n' "$line" | sed -n "s/.*${key}=\\([^ ]*\\).*/\\1/p"
}

max_metric_from_log() {
  local log_path="$1"
  local key="$2"
  grep 'DIFFY_STATE' "$log_path" | sed -n "s/.*${key}=\\([^ ]*\\).*/\\1/p" | sort -g | tail -n 1
}

metric_delta() {
  local before="$1"
  local after="$2"
  awk -v before="$before" -v after="$after" 'BEGIN { printf "%.3f", after - before }'
}

median_file() {
  sort -g "$1" | awk '
    { values[++n] = $1 }
    END {
      if (n == 0) exit 1
      if (n % 2 == 1) {
        printf "%.3f", values[(n + 1) / 2]
      } else {
        printf "%.3f", (values[n / 2] + values[n / 2 + 1]) / 2.0
      }
    }'
}

max_file() {
  sort -g "$1" | tail -n 1 | awk '{ printf "%.3f", $1 }'
}

print_summary_table() {
  local title="$1"
  shift
  local columns=("$@")

  printf '\n%s\n' "$title"
  printf '%-26s' "scenario"
  local column
  for column in "${columns[@]}"; do
    printf ' %18s' "$column"
  done
  printf '\n'

  local scenario
  for scenario in "${scenario_names[@]}"; do
    printf '%-26s' "$scenario"
    for column in "${columns[@]}"; do
      printf ' %18s' "$(cat "${work_dir}/${scenario}.${column}")"
    done
    printf '\n'
  done
}

collect_summary() {
  local scenario="$1"
  local metric
  for metric in \
    last_paint_ms last_raster_ms last_upload_ms \
    last_rows_rebuild_ms last_display_rows_rebuild_ms last_metrics_ms \
    texture_uploads tile_cache_hits tile_cache_misses resident_tiles paint_count; do
    local values_file="${work_dir}/${scenario}.${metric}.values"
    printf '%s\n' "$(median_file "$values_file")" > "${work_dir}/${scenario}.${metric}.median"
    printf '%s\n' "$(max_file "$values_file")" > "${work_dir}/${scenario}.${metric}.max"
  done

  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.last_paint_ms.median")" \
    "$(cat "${work_dir}/${scenario}.last_paint_ms.max")" \
    > "${work_dir}/${scenario}.paint_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.last_raster_ms.median")" \
    "$(cat "${work_dir}/${scenario}.last_raster_ms.max")" \
    > "${work_dir}/${scenario}.raster_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.last_upload_ms.median")" \
    "$(cat "${work_dir}/${scenario}.last_upload_ms.max")" \
    > "${work_dir}/${scenario}.upload_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.last_rows_rebuild_ms.median")" \
    "$(cat "${work_dir}/${scenario}.last_rows_rebuild_ms.max")" \
    > "${work_dir}/${scenario}.rows_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.last_display_rows_rebuild_ms.median")" \
    "$(cat "${work_dir}/${scenario}.last_display_rows_rebuild_ms.max")" \
    > "${work_dir}/${scenario}.display_rows_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.last_metrics_ms.median")" \
    "$(cat "${work_dir}/${scenario}.last_metrics_ms.max")" \
    > "${work_dir}/${scenario}.metrics_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.texture_uploads.median")" \
    "$(cat "${work_dir}/${scenario}.texture_uploads.max")" \
    > "${work_dir}/${scenario}.uploads_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.tile_cache_hits.median")" \
    "$(cat "${work_dir}/${scenario}.tile_cache_hits.max")" \
    > "${work_dir}/${scenario}.hits_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.tile_cache_misses.median")" \
    "$(cat "${work_dir}/${scenario}.tile_cache_misses.max")" \
    > "${work_dir}/${scenario}.misses_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.resident_tiles.median")" \
    "$(cat "${work_dir}/${scenario}.resident_tiles.max")" \
    > "${work_dir}/${scenario}.resident_med_max"
}

collect_action_summary() {
  local scenario="$1"
  local metric
  for metric in \
    action_last_paint_ms action_last_raster_ms action_last_upload_ms \
    action_last_rows_rebuild_ms action_last_display_rows_rebuild_ms action_last_metrics_ms \
    action_texture_uploads_delta action_tile_cache_hits_delta action_tile_cache_misses_delta action_resident_tiles_delta; do
    local values_file="${work_dir}/${scenario}.${metric}.values"
    printf '%s\n' "$(median_file "$values_file")" > "${work_dir}/${scenario}.${metric}.median"
    printf '%s\n' "$(max_file "$values_file")" > "${work_dir}/${scenario}.${metric}.max"
  done

  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.action_last_paint_ms.median")" \
    "$(cat "${work_dir}/${scenario}.action_last_paint_ms.max")" \
    > "${work_dir}/${scenario}.action_paint_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.action_last_raster_ms.median")" \
    "$(cat "${work_dir}/${scenario}.action_last_raster_ms.max")" \
    > "${work_dir}/${scenario}.action_raster_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.action_last_upload_ms.median")" \
    "$(cat "${work_dir}/${scenario}.action_last_upload_ms.max")" \
    > "${work_dir}/${scenario}.action_upload_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.action_last_rows_rebuild_ms.median")" \
    "$(cat "${work_dir}/${scenario}.action_last_rows_rebuild_ms.max")" \
    > "${work_dir}/${scenario}.action_rows_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.action_last_display_rows_rebuild_ms.median")" \
    "$(cat "${work_dir}/${scenario}.action_last_display_rows_rebuild_ms.max")" \
    > "${work_dir}/${scenario}.action_display_rows_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.action_last_metrics_ms.median")" \
    "$(cat "${work_dir}/${scenario}.action_last_metrics_ms.max")" \
    > "${work_dir}/${scenario}.action_metrics_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.action_texture_uploads_delta.median")" \
    "$(cat "${work_dir}/${scenario}.action_texture_uploads_delta.max")" \
    > "${work_dir}/${scenario}.action_uploads_delta_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.action_tile_cache_hits_delta.median")" \
    "$(cat "${work_dir}/${scenario}.action_tile_cache_hits_delta.max")" \
    > "${work_dir}/${scenario}.action_hits_delta_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.action_tile_cache_misses_delta.median")" \
    "$(cat "${work_dir}/${scenario}.action_tile_cache_misses_delta.max")" \
    > "${work_dir}/${scenario}.action_misses_delta_med_max"
  printf '%s / %s' \
    "$(cat "${work_dir}/${scenario}.action_resident_tiles_delta.median")" \
    "$(cat "${work_dir}/${scenario}.action_resident_tiles_delta.max")" \
    > "${work_dir}/${scenario}.action_resident_delta_med_max"
}

write_na_action_summary() {
  local scenario="$1"
  local key
  for key in \
    action_paint_med_max action_raster_med_max action_upload_med_max \
    action_rows_med_max action_display_rows_med_max action_metrics_med_max \
    action_uploads_delta_med_max action_hits_delta_med_max action_misses_delta_med_max action_resident_delta_med_max; do
    printf 'n/a' > "${work_dir}/${scenario}.${key}"
  done
}

action_state_pre=""
action_state_post=""

find_action_states() {
  local scenario="$1"
  local log_path="$2"
  mapfile -t scenario_states < <(grep 'DIFFY_STATE' "${log_path}" || true)
  action_state_pre=""
  action_state_post=""
  if [[ "${#scenario_states[@]}" -lt 2 ]]; then
    return 1
  fi

  local index
  case "${scenario}" in
    layout-switch)
      for ((index = 0; index < ${#scenario_states[@]} - 1; ++index)); do
        local current_layout next_layout
        current_layout="$(extract_metric "${scenario_states[index]}" "layout")"
        next_layout="$(extract_metric "${scenario_states[index + 1]}" "layout")"
        if [[ "${current_layout}" != "${next_layout}" && "${next_layout}" == "unified" ]]; then
          action_state_pre="${scenario_states[index]}"
          action_state_post="${scenario_states[index + 1]}"
          return 0
        fi
      done
      ;;
    file-switch)
      for ((index = 0; index < ${#scenario_states[@]} - 1; ++index)); do
        local current_path next_path
        current_path="$(extract_metric "${scenario_states[index]}" "selected_path")"
        next_path="$(extract_metric "${scenario_states[index + 1]}" "selected_path")"
        if [[ -n "${current_path}" && -n "${next_path}" && "${current_path}" != "${next_path}" && "${next_path}" != "none" ]]; then
          action_state_pre="${scenario_states[index]}"
          action_state_post="${scenario_states[index + 1]}"
          return 0
        fi
      done
      ;;
    warm-split-horizontal|long-line-split-scroll)
      for ((index = 0; index < ${#scenario_states[@]} - 1; ++index)); do
        local current_left next_left current_right next_right
        current_left="$(extract_metric "${scenario_states[index]}" "left_viewport_x")"
        next_left="$(extract_metric "${scenario_states[index + 1]}" "left_viewport_x")"
        current_right="$(extract_metric "${scenario_states[index]}" "right_viewport_x")"
        next_right="$(extract_metric "${scenario_states[index + 1]}" "right_viewport_x")"
        if [[ -n "${current_left}" && -n "${next_left}" ]] && \
           ([[ "${current_left}" != "${next_left}" ]] || [[ "${current_right}" != "${next_right}" ]]); then
          action_state_pre="${scenario_states[index]}"
          action_state_post="${scenario_states[index + 1]}"
          return 0
        fi
      done
      ;;
  esac

  return 1
}

run_scenario() {
  local scenario="$1"
  shift
  local extra_env=("$@")
  local run
  local has_action_metrics=false

  printf 'Running %-26s' "${scenario}"
  : > "${work_dir}/${scenario}.states"
  for run in $(seq 1 "$runs"); do
    printf ' %d/%d' "$run" "$runs"
    local config_dir stdout_log stderr_log state_line
    config_dir="$(mktemp -d "${work_dir}/${scenario}.xdg.XXXXXX")"
    stdout_log="${work_dir}/${scenario}.run${run}.stdout"
    stderr_log="${work_dir}/${scenario}.run${run}.stderr"

    if ! (
      export QT_QPA_PLATFORM=offscreen
      export QT_QUICK_BACKEND=software
      export XDG_CONFIG_HOME="${config_dir}"
      export XDG_DATA_HOME="${config_dir}/data"
      export XDG_CACHE_HOME="${config_dir}/cache"
      export DIFFY_START_COMPARE=1
      export DIFFY_REQUIRE_RESULTS=1
      export DIFFY_PRINT_STATE=1
      export DIFFY_PRINT_STATE_DELAY_MS=80
      export DIFFY_PRINT_STATE_REPEAT_MS=120
      export DIFFY_PRINT_STATE_COUNT=8
      export DIFFY_CAPTURE_DELAY_MS=980
      export DIFFY_EXIT_AFTER_MS=1300
      export DIFFY_FATAL_RUNTIME_WARNINGS=1
      export DIFFY_START_REPO="${repo}"
      export DIFFY_START_LEFT="${left_ref}"
      export DIFFY_START_RIGHT="${right_ref}"
      export DIFFY_START_COMPARE_MODE="${compare_mode}"
      local entry
      for entry in "${extra_env[@]}"; do
        export "${entry}"
      done
      "${binary}"
    ) >"${stdout_log}" 2>"${stderr_log}"; then
      printf '\nScenario %s failed on run %d\n' "$scenario" "$run" >&2
      printf '\nstdout:\n' >&2
      cat "${stdout_log}" >&2
      printf '\nstderr:\n' >&2
      cat "${stderr_log}" >&2
      return 1
    fi

    state_line="$(grep 'DIFFY_STATE' "${stdout_log}" | tail -n 1 || true)"
    if [[ -z "${state_line}" ]]; then
      printf '\nScenario %s did not emit DIFFY_STATE on run %d\n' "$scenario" "$run" >&2
      printf '\nstdout:\n' >&2
      cat "${stdout_log}" >&2
      printf '\nstderr:\n' >&2
      cat "${stderr_log}" >&2
      return 1
    fi

    if [[ "${state_line}" != *" error=none" ]]; then
      printf '\nScenario %s reported an error on run %d\n%s\n' "$scenario" "$run" "$state_line" >&2
      return 1
    fi

    grep 'DIFFY_STATE' "${stdout_log}" >> "${work_dir}/${scenario}.states"
    local metric
    for metric in \
      last_paint_ms last_raster_ms last_upload_ms \
      last_rows_rebuild_ms last_display_rows_rebuild_ms last_metrics_ms \
      texture_uploads tile_cache_hits tile_cache_misses resident_tiles paint_count; do
      local value
      value="$(max_metric_from_log "${stdout_log}" "${metric}")"
      if [[ -z "${value}" ]]; then
        printf '\nFailed to parse %s for scenario %s run %d\n' "$metric" "$scenario" "$run" >&2
        cat "${stdout_log}" >&2
        return 1
      fi
      printf '%s\n' "${value}" >> "${work_dir}/${scenario}.${metric}.values"
    done

    if find_action_states "${scenario}" "${stdout_log}"; then
      has_action_metrics=true
      for metric in \
        last_paint_ms last_raster_ms last_upload_ms \
        last_rows_rebuild_ms last_display_rows_rebuild_ms last_metrics_ms; do
        local action_value
        action_value="$(extract_metric "${action_state_post}" "${metric}")"
        printf '%s\n' "${action_value}" >> "${work_dir}/${scenario}.action_${metric}.values"
      done

      local counter
      for counter in texture_uploads tile_cache_hits tile_cache_misses resident_tiles; do
        local before_value after_value
        before_value="$(extract_metric "${action_state_pre}" "${counter}")"
        after_value="$(extract_metric "${action_state_post}" "${counter}")"
        printf '%s\n' "$(metric_delta "${before_value}" "${after_value}")" \
          >> "${work_dir}/${scenario}.action_${counter}_delta.values"
      done
    fi
  done
  printf '\n'
  collect_summary "$scenario"
  if [[ "${has_action_metrics}" == true ]]; then
    collect_action_summary "$scenario"
  else
    write_na_action_summary "$scenario"
  fi
}

scenario_names=(
  "cold-unified"
  "cold-split"
  "warm-vertical-scroll"
  "warm-split-horizontal"
  "file-switch"
  "layout-switch"
  "long-line-split-scroll"
)

if [[ -n "${requested_scenario}" ]]; then
  case "${requested_scenario}" in
    cold-unified|cold-split|warm-vertical-scroll|warm-split-horizontal|file-switch|layout-switch|long-line-split-scroll)
      scenario_names=("${requested_scenario}")
      ;;
    *)
      echo "Unknown scenario: ${requested_scenario}" >&2
      usage >&2
      exit 1
      ;;
  esac
fi

if printf '%s\n' "${scenario_names[@]}" | grep -qx 'long-line-split-scroll'; then
  create_long_line_repo
fi

for scenario in "${scenario_names[@]}"; do
  case "${scenario}" in
    cold-unified)
      run_scenario "${scenario}" \
        "DIFFY_START_LAYOUT=unified" \
        "DIFFY_START_FILE_PATH=${unified_file}"
      ;;
    cold-split)
      run_scenario "${scenario}" \
        "DIFFY_START_LAYOUT=split" \
        "DIFFY_START_FILE_PATH=${unified_file}"
      ;;
    warm-vertical-scroll)
      run_scenario "${scenario}" \
        "DIFFY_START_LAYOUT=unified" \
        "DIFFY_START_FILE_PATH=${scroll_file}" \
        "DIFFY_START_SCROLL_Y=600"
      ;;
    warm-split-horizontal)
      run_scenario "${scenario}" \
        "DIFFY_START_LAYOUT=split" \
        "DIFFY_START_FILE_PATH=${split_file}" \
        "DIFFY_START_SCROLL_Y=2500" \
        "DIFFY_RESET_SURFACE_STATS_AFTER_MS=160" \
        "DIFFY_START_WHEEL_AFTER_MS=220" \
        "DIFFY_START_WHEEL_PIXEL_X=-160"
      ;;
    file-switch)
      run_scenario "${scenario}" \
        "DIFFY_START_LAYOUT=unified" \
        "DIFFY_START_FILE_PATH=${switch_from_file}" \
        "DIFFY_RESET_SURFACE_STATS_AFTER_MS=160" \
        "DIFFY_SWITCH_FILE_TO_PATH=${switch_to_file}" \
        "DIFFY_SWITCH_FILE_AFTER_MS=220"
      ;;
    layout-switch)
      run_scenario "${scenario}" \
        "DIFFY_START_LAYOUT=split" \
        "DIFFY_START_FILE_PATH=${split_file}" \
        "DIFFY_START_SCROLL_Y=1200" \
        "DIFFY_RESET_SURFACE_STATS_AFTER_MS=160" \
        "DIFFY_SWITCH_LAYOUT_TO=unified" \
        "DIFFY_SWITCH_LAYOUT_AFTER_MS=260"
      ;;
    long-line-split-scroll)
      run_scenario "${scenario}" \
        "DIFFY_START_REPO=${long_repo}" \
        "DIFFY_START_LEFT=HEAD~1" \
        "DIFFY_START_RIGHT=HEAD" \
        "DIFFY_START_COMPARE_MODE=two-dot" \
        "DIFFY_START_LAYOUT=split" \
        "DIFFY_START_FILE_PATH=src/long.cpp" \
        "DIFFY_RESET_SURFACE_STATS_AFTER_MS=160" \
        "DIFFY_START_WHEEL_AFTER_MS=220" \
        "DIFFY_START_WHEEL_PIXEL_X=-160"
      ;;
  esac
done

print_summary_table "Render Timings (median / max ms)" \
  "paint_med_max" \
  "raster_med_max" \
  "upload_med_max"

print_summary_table "Layout Timings (median / max ms)" \
  "rows_med_max" \
  "display_rows_med_max" \
  "metrics_med_max"

print_summary_table "Cache Counters (median / max)" \
  "uploads_med_max" \
  "hits_med_max" \
  "misses_med_max" \
  "resident_med_max"

print_summary_table "Action Render Timings (median / max ms)" \
  "action_paint_med_max" \
  "action_raster_med_max" \
  "action_upload_med_max"

print_summary_table "Action Layout Timings (median / max ms)" \
  "action_rows_med_max" \
  "action_display_rows_med_max" \
  "action_metrics_med_max"

print_summary_table "Action Cache Deltas (median / max)" \
  "action_uploads_delta_med_max" \
  "action_hits_delta_med_max" \
  "action_misses_delta_med_max" \
  "action_resident_delta_med_max"

if [[ "$keep_workdir" == true ]]; then
  printf '\nRaw scenario files kept at: %s\n' "${work_dir}"
else
  printf '\nRaw scenario files cleaned up. Re-run with --keep-workdir to keep them.\n'
fi
