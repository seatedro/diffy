# Carbon Projection Strategy

Carbon projection is streaming-first. The core API emits `ProjectionRow` values
through a callback so callers can count, filter, layout, or render rows without
allocating a row vector.

When rows do need to be materialized, use `ProjectionBuffer`. It owns a reusable
`Vec<ProjectionRow>` and keeps capacity across rebuilds. This is the intended
storage path for UI caches, viewport windows, and render prep. Avoid allocating
a fresh `Vec<ProjectionRow>` per frame or per scroll tick.

Current guidance:

- Use `project_file` / `project_window` for one-pass consumers.
- Use `ProjectionBuffer::rebuild_file` or `rebuild_window` for cached row sets.
- Use `ProjectionBuffer::append_file` for multi-file projections into one row
  buffer.
- Prefer projecting viewport windows over materializing whole files on hot UI
  paths.
- Keep annotations and inline/syntax spans as overlays keyed by source
  coordinates, not embedded row payloads.

The benchmark split between count-only, reusable-buffer, and fresh-`Vec`
projection is intentional. It lets us see whether a regression is in projection
math, retained row materialization, or allocation churn.
