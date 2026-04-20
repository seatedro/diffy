# Performance Tooling Boundary

This document defines where Diffy's future performance tooling belongs after the removal of the
old capture and automation harnesses.

## Non-Goals

The main `diffy` binary is not a benchmark harness.

Do not add:

- hidden-window execution modes
- forced auto-exit flags
- state/file/error dump flags
- in-binary screenshot capture paths
- perf-only startup env var contracts

These features make the product binary harder to reason about and blur the boundary between app
behavior and test instrumentation.

## Allowed Shape

Future performance work should use a separate surface:

- a dedicated perf runner binary
- a benchmark target under `benches/`
- or a standalone tool that drives Diffy through a narrow public interface

That tool should:

- run in `--release`
- define named scripted scenarios
- separate setup, action, and measurement windows
- emit stable machine-readable results
- avoid changing normal app behavior

## Scenario Model

A valid perf scenario should describe:

- setup: repo fixture or synthetic state
- action: one explicit interaction or short scripted sequence
- measurement window: the precise phase being timed
- outputs: timings, counts, and environment metadata

Examples:

- first frame for a ready workspace
- switch selected file
- open command palette
- scroll viewport
- complete a compare operation

## Hot Reload

Hot reload remains a supported development workflow and is not considered part of the removed perf
or capture harness surface.

## Rule Of Thumb

If a feature exists only to help tests, capture, benchmarking, or scripted measurement, it should
not be added to the normal app CLI unless there is a strong product reason.
