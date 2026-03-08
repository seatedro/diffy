#!/usr/bin/env python3

from __future__ import annotations

import argparse
import datetime as dt
import json
import pathlib
import subprocess
from typing import Dict


NUMERIC_METRICS = [
    "last_paint_ms",
    "last_raster_ms",
    "last_upload_ms",
    "last_rows_rebuild_ms",
    "last_display_rows_rebuild_ms",
    "last_metrics_ms",
    "texture_uploads",
    "tile_cache_hits",
    "tile_cache_misses",
    "resident_tiles",
    "paint_count",
]

ACTION_METRICS = [
    "action_last_paint_ms",
    "action_last_raster_ms",
    "action_last_upload_ms",
    "action_last_rows_rebuild_ms",
    "action_last_display_rows_rebuild_ms",
    "action_last_metrics_ms",
    "action_texture_uploads_delta",
    "action_tile_cache_hits_delta",
    "action_tile_cache_misses_delta",
    "action_resident_tiles_delta",
]


def read_text(path: pathlib.Path) -> str:
    return path.read_text(encoding="utf-8").strip()


def read_float(path: pathlib.Path) -> float:
    return float(read_text(path))


def jj_rev(repo_root: pathlib.Path, revset: str) -> str | None:
    try:
        out = subprocess.check_output(
            ["jj", "log", "-r", revset, "--no-graph", "-T", "commit_id.short()"],
            cwd=repo_root,
            text=True,
        ).strip()
        return out or None
    except Exception:
        return None


def git_rev(repo_root: pathlib.Path) -> str | None:
    try:
        out = subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=repo_root,
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
        return out or None
    except Exception:
        return None


def load_scenario(work_dir: pathlib.Path, scenario: str) -> Dict[str, object]:
    metrics: Dict[str, Dict[str, float]] = {}
    for metric in NUMERIC_METRICS + ACTION_METRICS:
        median_path = work_dir / f"{scenario}.{metric}.median"
        max_path = work_dir / f"{scenario}.{metric}.max"
        if median_path.exists() and max_path.exists():
            metrics[metric] = {
                "median": read_float(median_path),
                "max": read_float(max_path),
            }

    strings = {}
    for key in [
        "paint_med_max",
        "raster_med_max",
        "upload_med_max",
        "rows_med_max",
        "display_rows_med_max",
        "metrics_med_max",
        "uploads_med_max",
        "hits_med_max",
        "misses_med_max",
        "resident_med_max",
        "action_paint_med_max",
        "action_raster_med_max",
        "action_upload_med_max",
        "action_rows_med_max",
        "action_display_rows_med_max",
        "action_metrics_med_max",
        "action_uploads_delta_med_max",
        "action_hits_delta_med_max",
        "action_misses_delta_med_max",
        "action_resident_delta_med_max",
    ]:
        path = work_dir / f"{scenario}.{key}"
        if path.exists():
            strings[key] = read_text(path)

    state_lines_path = work_dir / f"{scenario}.states"
    state_lines = state_lines_path.read_text(encoding="utf-8").splitlines() if state_lines_path.exists() else []
    return {
        "metrics": metrics,
        "summary_strings": strings,
        "state_count": len(state_lines),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Export perf-matrix temp files to JSON")
    parser.add_argument("--work-dir", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--repo-root", required=True)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--runs", required=True, type=int)
    parser.add_argument("--scenario", action="append", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--left", required=True)
    parser.add_argument("--right", required=True)
    parser.add_argument("--compare-mode", required=True)
    parser.add_argument("--unified-file", required=True)
    parser.add_argument("--scroll-file", required=True)
    parser.add_argument("--split-file", required=True)
    parser.add_argument("--switch-from-file", required=True)
    parser.add_argument("--switch-to-file", required=True)
    parser.add_argument("--ident")
    args = parser.parse_args()

    work_dir = pathlib.Path(args.work_dir)
    repo_root = pathlib.Path(args.repo_root)
    output_path = pathlib.Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    payload = {
        "schema_version": 1,
        "ident": args.ident,
        "recorded_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "repo_root": str(repo_root),
        "binary": args.binary,
        "runs": args.runs,
        "inputs": {
            "repo": args.repo,
            "left": args.left,
            "right": args.right,
            "compare_mode": args.compare_mode,
            "unified_file": args.unified_file,
            "scroll_file": args.scroll_file,
            "split_file": args.split_file,
            "switch_from_file": args.switch_from_file,
            "switch_to_file": args.switch_to_file,
        },
        "vcs": {
            "jj_at": jj_rev(repo_root, "@"),
            "jj_parent": jj_rev(repo_root, "@-"),
            "git_head": git_rev(repo_root),
        },
        "scenarios": {scenario: load_scenario(work_dir, scenario) for scenario in args.scenario},
    }

    output_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
