# Diffy Repo Guidance

Diffy keeps product code and developer tooling separate.

## App Binary Rules

- Keep the main `diffy` binary product-focused.
- Do not add hidden automation modes, dump-state flags, capture modes, or perf-only control paths to the app CLI.
- Do not reintroduce in-process harness hooks such as hidden-window startup, JSON dump flags, or screenshot capture paths.
- Hot reload is an allowed exception. It is part of the active development workflow and remains behind the `hot-reload` feature.

## Performance Tooling Boundary

- Performance tooling must live outside the normal app runtime path.
- New perf work should use a separate runner, benchmark target, or dedicated tool rather than app-startup debug flags.
- Perf scenarios should be scripted, reproducible, and release-only.
- Perf outputs should use a stable machine-readable schema so regressions can be compared without coupling the format to the UI binary.

## Contribution Notes

- Prefer deleting stale tooling over keeping dead compatibility paths.
- Keep docs in sync when development workflows change.
- If a new tool is needed, make its ownership and boundary explicit in `README.md` and related docs before adding it.
