# AGENTS.md

Guidance for agents working in `crates/carbon`. The root `AGENTS.md` still
applies; this node captures the crate-specific contracts that are easy to miss
from code alone.

## Purpose And Scope

Carbon is Diffy's data-oriented diff substrate. It owns durable diff data,
unified patch parsing, side-specific text storage, inline diff spans, projection
rows/windows, and review anchors.

Carbon deliberately does not own UI components, Git access, syntax
highlighting, text shaping, rendering, worker scheduling, or product state.
Callers adapt Carbon's plain data into those layers.

## Related Context

- Root product and performance rules: `../../AGENTS.md`
- Projection strategy and allocation guidance: `PROJECTION.md`
- Syntax consumers of Carbon text ranges: `../phosphor/AGENTS.md`

## Core Contracts

- Coordinates are explicit. `SourceRange`, `LineId`, `old_index`, and
  `new_index` are zero-based. `Hunk` source starts, `ProjectionRow.old_line`,
  `ProjectionRow.new_line`, and `review::LineRange` are one-based.
- `TextByteRange` is byte-based, not character-based. Any span that touches
  UTF-8 text must remain on char boundaries when it claims to be text-derived.
- `TextStore` may contain non-UTF-8 bytes. Use `as_str()` only when the caller
  can handle `None`; syntax highlighting is one such UTF-8-only consumer.
- `TextStore::line_range` strips LF and CRLF terminators from line payload
  ranges. Do not re-add phantom final lines for trailing newlines.
- `FileDiff.blocks` and `FileDiff.hunks` use compact IDs backed by vector
  positions. Keep IDs stable and check by ID after indexing.
- `FileDiff.is_partial` means Carbon only knows hunk text, not the whole file.
  Projection must not invent collapsed context gaps for partial files.
- Projection is streaming-first. `project_file` and `project_window` emit rows
  through callbacks; `ProjectionBuffer` is the reusable materialized storage
  path when callers need retained rows.
- Projection is pure and non-mutating. Expanding context changes
  `ExpansionState`, not the underlying hunks or blocks.
- Inline diffs are bounded by `InlineOptions::max_line_len`. Do not remove this
  guard on hot paths; very long lines should degrade to no inline spans.
- Review annotations are overlays keyed by file/side/source coordinates. Keep
  annotations and syntax/inline spans out of `ProjectionRow` payloads unless the
  core coordinate model itself changes.

## Usage Patterns

- Prefer one-pass projection callbacks for counting, scanning, layout sizing, or
  render prep that can stream rows.
- Use `ProjectionBuffer::rebuild_*` or `append_*` when the caller keeps a row
  cache across frames or scroll ticks.
- Use `projected_row_byte_range` to bridge projection rows back to side-specific
  `TextStore` ranges for syntax, inline overlays, or review affordances.
- Add coordinate features at the Carbon model layer first, then adapt the app
  layer to them. Avoid parallel coordinate systems in UI code.
- When changing row semantics, update invariants and property tests before
  broad app integration.

## Anti-Patterns

- Do not add UI, renderer, syntax, Git, async runtime, or app action types to
  Carbon.
- Do not allocate a fresh `Vec<ProjectionRow>` per frame or per scroll tick when
  a callback or `ProjectionBuffer` would do.
- Do not treat `ProjectionRow.old_line` / `new_line` as zero-based just because
  the paired indexes are zero-based.
- Do not assume all diffs are full-file. Partial diffs are a real input shape.
- Do not clone text into new `String`s for derived views when ranges into
  `TextStore` are enough.

## Validation

- Focused crate tests: `cargo test -p carbon`
- Projection or coordinate changes: include `tests/invariants.rs` and
  `tests/properties.rs`
- Allocation-sensitive projection changes: compare
  `cargo bench -p carbon --bench carbon_core` when practical

## Maintenance

Keep this node compact. If a rule applies to all crates, move it to the root
node. If a fact only applies to projection internals, keep it in `PROJECTION.md`
and downlink it here rather than duplicating the full explanation.
