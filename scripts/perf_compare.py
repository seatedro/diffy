#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import math
import pathlib
from typing import Tuple


PRIMARY_METRICS: list[Tuple[str, str, bool]] = [
    ("action_last_paint_ms", "action paint", False),
    ("action_last_rows_rebuild_ms", "action rows", False),
    ("action_last_display_rows_rebuild_ms", "action display rows", False),
    ("action_last_metrics_ms", "action metrics", False),
    ("action_texture_uploads_delta", "action uploads delta", False),
    ("action_tile_cache_misses_delta", "action misses delta", False),
]


def resolve_run(path_or_ident: str, repo_root: pathlib.Path) -> pathlib.Path:
    candidate = pathlib.Path(path_or_ident)
    if candidate.exists():
        return candidate
    run_path = repo_root / ".perf-runs" / f"{path_or_ident}.diffy.json"
    if run_path.exists():
        return run_path
    raise FileNotFoundError(f"Could not resolve perf run '{path_or_ident}'")


def load_run(path: pathlib.Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def format_delta(new: float, old: float, lower_is_better: bool) -> str:
    if math.isclose(old, 0.0, abs_tol=1e-9):
        if math.isclose(new, 0.0, abs_tol=1e-9):
            return "0.0%"
        return "n/a"
    pct = ((new - old) / old) * 100.0
    if lower_is_better:
        pct *= -1.0
    sign = "+" if pct >= 0 else ""
    return f"{sign}{pct:.1f}%"


def metric_value(run: dict, scenario: str, metric: str) -> float | None:
    return run.get("scenarios", {}).get(scenario, {}).get("metrics", {}).get(metric, {}).get("median")


def markdown_compare(new_run: dict, old_run: dict, new_label: str, old_label: str) -> str:
    scenarios = sorted(set(new_run.get("scenarios", {})) & set(old_run.get("scenarios", {})))
    lines = [
        f"# Diffy Perf Compare",
        "",
        f"- new: `{new_label}`",
        f"- old: `{old_label}`",
        "",
    ]

    for scenario in scenarios:
        lines.append(f"## {scenario}")
        lines.append("")
        lines.append("| metric | new | old | delta |")
        lines.append("| --- | ---: | ---: | ---: |")
        for metric, label, lower_is_better in PRIMARY_METRICS:
            new_value = metric_value(new_run, scenario, metric)
            old_value = metric_value(old_run, scenario, metric)
            if new_value is None or old_value is None:
                continue
            lines.append(
                f"| {label} | {new_value:.3f} | {old_value:.3f} | {format_delta(new_value, old_value, lower_is_better)} |"
            )
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description="Compare two Diffy perf runs")
    parser.add_argument("new_run")
    parser.add_argument("old_run")
    parser.add_argument("--save")
    args = parser.parse_args()

    repo_root = pathlib.Path(__file__).resolve().parents[1]
    new_path = resolve_run(args.new_run, repo_root)
    old_path = resolve_run(args.old_run, repo_root)

    markdown = markdown_compare(load_run(new_path), load_run(old_path), new_path.name, old_path.name)
    if args.save:
        pathlib.Path(args.save).write_text(markdown, encoding="utf-8")
    print(markdown, end="")


if __name__ == "__main__":
    main()
