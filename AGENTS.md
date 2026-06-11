# AGENTS.md

Guidance for agents working in Diffy, a native GPU-accelerated Git diff viewer.
Applies repo-wide unless a more specific `AGENTS.md` exists in a subdirectory.

Assume you will be fired if you make mistakes.

Optimize every change for, in order: user experience (responsive, polished, no
UI-thread pauses), performance (cheap incremental redraw/diff/highlighting),
minimal binary size (no new deps, features, or embedded assets without clear
justification), and clean, boring Rust.

## Repository Map

- `src/ui/app.rs` — winit loop, window lifecycle, input dispatch, redraw scheduling.
- `src/ui/state/` — `AppState`; actions mutate state and return `Effect`s.
- `src/actions.rs` / `src/effects.rs` / `src/events.rs` — the action/effect/event
  boundary; extend these contracts rather than wiring work through UI code.
- `src/apprt/` — services and worker threads; all blocking Git/fs/network/AI work.
- `src/core/` — product logic: diff parsing, compare backends, text/token buffers,
  syntax, fuzzy search, themes, VCS.
- `src/render/` — low-level wgpu/glyphon renderer; per-frame allocation, texture
  churn, and text caches are performance-sensitive.
- `src/ui/components|overlays|editor/` — UI surfaces; use design constants from
  `src/ui/design.rs` and theme tokens from `src/ui/theme.rs`.
- `crates/carbon/` — diff substrate. `crates/halogen/` — UI/reactivity toolkit;
  read its `ARCHITECTURE.md` before touching signals, scene primitives, or `view!`.
  `crates/halogen-macros/` — `#[derive(Store)]` and `view!` proc macros.
  `crates/phosphor/` — tree-sitter syntax highlighting.
- `crates/difftastic/` is vendored and out-of-workspace; don't casually refactor it.
- `.docs/` is reference material, not app source.

Before editing a crate, load its `AGENTS.md`
(`crates/{carbon,halogen,halogen-macros,phosphor}/AGENTS.md`). Keep shared facts
at the shallowest node that always applies; when behavior changes, update the
leaf node first.

## Rules

- Never block winit event handling or rendering — route expensive work through
  `Effect`s, `AppRuntime`, and workers. Preserve the action -> state/effect ->
  runtime -> event -> state loop; no ad hoc callbacks that bypass it.
- Guard async results against newer state with generation IDs.
- Prefer shared buffers/ranges (`TextBuffer`, `TokenBuffer`) over duplicated
  strings; avoid clones and unbounded work in hot paths; cache derived UI data.
- Defer syntax/layout/measurement work until the viewport needs it. Keep
  renderer changes batch-friendly: reuse pools and caches.
- Keyboard, mouse, scrolling, resizing, HiDPI, and small windows are first-class.
  Preserve focus and IME behavior when touching overlays or text fields. Shared
  controls expose stable AccessKit roles/labels/actions so Computer Use can
  inspect and drive them.
- Surface recoverable failures via state/toasts; no panics outside tests.
- Use existing spacing/radius/icon/color/typography tokens; no one-off visual
  constants. Keep copy short and concrete.
- Before adding a crate, prefer std, an existing dep, or a small local helper;
  if added anyway, disable default features and justify the size tradeoff.
  Embedded assets (fonts, icons, grammars) directly affect distribution size.
- Use `Result` + project error types; keep `unwrap`/`expect` to tests or
  impossible invariants. Boring Rust over macro magic outside `halogen-macros`.
- Comment only non-obvious invariants, performance choices, or platform behavior.

## Validation

Smallest meaningful check first; broaden when risk crosses module boundaries.
Don't add regression tests by default — only when asked, or when the risk is
high enough that you explicitly call out why.

- `cargo fmt --all`, `cargo test`, and when practical
  `cargo clippy --workspace --all-targets --all-features`.
- Hot reload UI iteration: `dx serve --hot-patch --features hot-reload`.
- macOS Computer Use dev app: `scripts/dev-loop.sh app` builds and launches
  `Diffy Dev.app` (bundle id `io.github.seatedro.diffy.dev`; ad-hoc signed,
  keyring disabled, dev token file for GitHub auth, startup args after `--`).
  Target it, not any installed `/Applications/Diffy.app`.
- GUI validation: use the global `$cua` skill/CLI; keep captures under `/tmp/cua/`.

Headless UI loop, for surfaces with a fixture (review card via
`src/ui/harness.rs`): run `cargo test --lib <surface>::` first, then
`cargo run --example render_fixture --features headless-render` for a PNG.
Ground truth rules: text content comes from `dump_accessibility(frame)`;
spacing/overlap/alignment comes from `dump_text_layout(scene, font_system)`
gaps. Never read either off the PNG — antialiased glyphs have produced phantom
bugs; the image is only for visual quality (kerning, padding, centering). For
genuinely visual glyph questions, tint pieces distinct colors and crop+zoom.
Keep PNGs under `target/`; never commit them.

If a command can't run in this environment, say so and note what went unverified.

## Change Hygiene

- Search with `rg`; skip vendored/reference trees unless needed.
- Keep changes scoped to the task; don't reformat unrelated files or edit
  generated packaging assets unless the task is packaging-specific.
- Never log secrets, tokens, or credential material.
- Preserve user work in the tree; never revert unrelated changes without an
  explicit request.
- Skip progress updates for routine work; update only when blocked, requesting
  approval, or surfacing a user-visible decision.
