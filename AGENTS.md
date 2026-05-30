# AGENTS.md

Guidance for agents working in Diffy. These instructions apply to the whole
repository unless a more specific `AGENTS.md` is added in a subdirectory.

## Product Priorities

Diffy is a native GPU-accelerated Git diff viewer. Optimize every change for:

1. User experience: fast startup, responsive interaction, clear errors, polished
   visual behavior, and no surprising pauses on the UI thread.
2. Performance: keep redraw, layout, diff parsing, syntax highlighting, and Git
   operations cheap and incremental.
3. Minimal binary size: avoid new dependencies, feature bloat, embedded assets,
   and broad default features unless they are clearly justified.
4. Clean Rust: small modules, explicit ownership, idiomatic error handling, and
   simple data flow over clever abstractions.

## Repository Map

- `src/main.rs` starts the native app through `ui::run()`.
- `src/ui/app.rs` owns the `winit` application loop, window lifecycle,
  renderer setup, input dispatch, frame building, and redraw scheduling.
- `src/ui/state/` contains `AppState`; user actions mutate state and return
  `Effect`s. Keep state transitions deterministic and easy to test.
- `src/actions.rs`, `src/effects.rs`, and `src/events.rs` define the
  action/effect/event boundary. Prefer extending these contracts over wiring
  work directly through UI code.
- `src/apprt/` runs services and worker threads. Blocking Git, filesystem,
  network, settings, AI, and watcher work belongs here, not on the UI thread.
- `src/core/` holds product logic: diff parsing, compare backends, text/token
  buffers, syntax, fuzzy search, rendering prep, themes, and VCS integrations.
- `src/render/` is the low-level `wgpu`/`glyphon` renderer. Treat per-frame
  allocation, texture churn, shader changes, and text cache behavior as
  performance-sensitive.
- `src/ui/components/`, `src/ui/overlays/`, and `src/ui/editor/` are reusable
  UI surfaces. Use existing design constants from `src/ui/design.rs` and theme
  tokens from `src/ui/theme.rs`.
- `crates/carbon/` is Diffy's data-oriented diff substrate: patch parsing,
  durable diff coordinates, text stores, inline diffs, projections, and review
  anchors.
- `crates/halogen/` is Diffy's local UI/reactivity toolkit. Read
  `crates/halogen/ARCHITECTURE.md` before changing signals, scene primitives, or
  the `view!` macro contract.
- `crates/halogen-macros/` owns the proc-macro parsing and lowering for
  `#[derive(Store)]` and `view!`.
- `crates/phosphor/` is the local tree-sitter-backed syntax highlighting crate.
- `crates/difftastic/` is vendored and excluded from the main workspace. Do not
  casually refactor it; changes there should be intentional vendor or integration
  work.
- `.docs/` contains reference material and experiments. Do not treat large
  reference checkouts there as app source.

## Crate Intent Nodes

Use these crate-level `AGENTS.md` files as progressive-disclosure downlinks.
Load the relevant node before editing that crate, and keep shared facts at the
shallowest node that always applies instead of duplicating them.

- Diff substrate: `crates/carbon/AGENTS.md`
- UI/reactivity toolkit: `crates/halogen/AGENTS.md`
- UI proc macros: `crates/halogen-macros/AGENTS.md`
- Syntax highlighting and packs: `crates/phosphor/AGENTS.md`

Treat these files as an intent layer, not general documentation. Each node
should compress the code it covers, surface hidden contracts and anti-patterns,
and point to deeper context instead of copying it. Add new nodes only at
semantic boundaries where responsibility, invariants, or failure modes change.
When a fact applies to multiple areas, keep it in the least common ancestor
node. When behavior changes, update the affected leaf node first and then revise
parent summaries only if their guidance changed.

## Architecture Rules

- Do not block `winit` event handling or rendering. Route expensive work through
  `Effect`s, `AppRuntime`, and services/workers.
- Use generation IDs or equivalent stale-result guards when async work can race
  with newer state.
- Preserve the action -> state/effect -> runtime -> event -> state loop. Avoid
  ad hoc callbacks that bypass this flow.
- Prefer shared buffers and ranges over duplicated strings. The existing
  `TextBuffer` and `TokenBuffer` patterns are there to reduce memory churn.
- Defer expensive syntax annotation, layout, measurement, and rendering prep
  until the user-visible file or viewport needs it.
- Keep renderer changes batch-friendly. Reuse pools and caches instead of
  allocating GPU resources or glyph buffers every frame.

## UX Rules

- Treat keyboard, mouse, scrolling, resizing, HiDPI scaling, and small windows as
  first-class behavior.
- Surface recoverable failures through app state, toasts, or visible UI. Avoid
  panics outside tests.
- Preserve focus handling and text input/IME behavior when changing overlays,
  search, commit editing, or settings fields.
- Use existing spacing, radius, icon, color, typography, and theme primitives.
  Do not introduce one-off visual constants unless the design system needs a new
  named token.
- Keep copy short and concrete. Diffy should feel like a tool, not a marketing
  page.

## Performance And Size Rules

- Before adding a crate, ask whether `std`, an existing dependency, or a small
  local helper is enough. If adding a crate, disable default features when
  possible and explain the size/perf tradeoff.
- Avoid clones in hot paths, especially diff rows, file lists, token spans,
  render primitives, and frame construction.
- Avoid unbounded per-frame work. Cache measurements, flattening, prepared rows,
  icon rasterization, avatars, textures, and derived UI data when practical.
- Keep release settings size-conscious. `Cargo.toml` currently uses LTO and one
  codegen unit for release builds.
- Be careful with embedded assets. Fonts, icons, generated packaging assets, and
  syntax grammars directly affect distribution size.

## Rust Style

- Use `Result` and project error types for fallible operations. Keep `unwrap` and
  `expect` to tests or genuinely impossible invariants with clear context.
- Prefer explicit, boring Rust over macro magic unless working inside
  `halogen-macros`.
- Keep public types compact and derive useful traits (`Debug`, `Clone`,
  `PartialEq`, `Eq`, `Serialize`) when they match nearby patterns.
- Use `Arc`/`Rc` intentionally for shared UI/runtime data; keep thread-crossing
  data `Send` and owned.
- Add comments only where they explain non-obvious invariants, performance
  choices, or platform behavior.

## Validation

Use the smallest meaningful validation for the change, then broaden when the
risk crosses module boundaries.

- Do not add regression tests by default. Only add new tests when the user asks
  for them or when the behavior is high-risk enough that you explicitly call out
  why the test is necessary.
- Format: `cargo fmt --all`
- Test: `cargo test`
- Focused tests: `cargo test <name>`
- Lints when practical: `cargo clippy --workspace --all-targets --all-features`
- Run locally: `cargo run`
- macOS Computer Use dev app: `scripts/dev-loop.sh app` builds and launches
  `/Users/ro/Applications/Diffy Dev.app` with bundle id
  `io.github.seatedro.diffy.dev`. Prefer this app path or bundle id for
  `$computer-use` validation so the dev build is targeted instead of any
  globally installed `/Applications/Diffy.app`.
- Hot reload UI iteration: `dx serve --hot-patch --features hot-reload`
- Packaging/release smoke check: `cargo build --release`

`Diffy Dev.app` is intentionally ad-hoc signed and wrapped for local
automation: no release cert is required, the window title prefix is `diffy dev`,
and keyring is disabled by default in the wrapper. Pass startup args after `--`,
for example `scripts/dev-loop.sh app -- --repo /Users/ro/personal/diffy`.

The native accessibility tree is built with AccessKit. Shared controls should
expose stable roles, labels, and actions so Computer Use can inspect and operate
them through platform accessibility APIs. Current coverage includes buttons,
recent repo rows, file rows/tree rows, picker rows, tabs, segmented controls,
checkboxes/toggles, dropdown/menu rows, modals, text inputs, editor focus,
scrolling, click/focus/set-value actions, and the GPU-rendered editor document
itself: the editor viewport, visible file headers, hunk headers, diff line
text, review comment blocks, and context-expansion controls should all be
represented in the AccessKit tree.

For GUI validation, use the global `$cua` skill and CLI when available. Prefer
the one-shot path for screenshots:

```bash
cua capture --window "diffy native" --path /tmp/cua/diffy.png --upload -- \
  env WINIT_UNIX_BACKEND=x11 WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1 \
  target/debug/diffy --repo <repo-with-diff> --file-path <path>
```

Use `cua start`, `cua launch`, `cua wait-window`, `cua shot-window`, and
`cua upload` for interactive UI loops. Keep captures under `/tmp/cua/` so
validation does not dirty the repository.

### Headless UI verification loop

For UI surfaces with a headless fixture (currently the review-thread card via
`src/ui/harness.rs`), iterate without launching the app. Four legs, each
answering a different question:

- See it (layout + color): `cargo run --example render_fixture --features
  headless-render` writes `target/ui/review_card.png`. `CARD_WIDTH` and
  `CARD_SCALE` env vars vary the geometry; the scene bakes the scale, so the PNG
  is full-resolution. Read the image to judge spacing, alignment, weight, and
  color.
- Read the text (content ground truth): `dump_accessibility(frame)` emits one
  line per node (`author_id | role | label | value | x,y,w,h`). This is the
  source of truth for what a string says and where its bounds are. Do not read
  text content off rendered glyphs — small text in a PNG is misread-prone, and
  that has produced phantom "bugs" before. Confirm content and clipping against
  the dump, use the image only for visual quality.
- Measure it (geometry ground truth): `dump_text_layout(scene, font_system)` emits
  one line per painted text piece — `y x adv gap kind/weight "text"` — where `gap`
  is the *measured* distance to the next piece on the same visual line (`text_pieces`
  / `text_lines` expose the same data structurally). This is the source of truth for
  spacing, overlap, and alignment: `+N` is a space N px wide, `+0` means the pieces
  abut, a negative gap means they overlap. NEVER eyeball a PNG to decide whether two
  glyphs touch — antialiased same-color text reads as merged at small sizes and has
  burned hours chasing non-bugs. Compute the gap. (A primitive's `rect.width` is its
  layout box, not the ink, so the advance is measured, not read off the rect.)
- Drive it (behavior, no GPU): the `#[cfg(test)]` `UiHarness` replays
  `.drag(...)` / `.key(...)` against frame-built hit regions and asserts on the
  resulting `AppState` / emitted actions (e.g. drag-select then Cmd+C copies the
  exact substring). The input layer takes `Option<&mut glyphon::FontSystem>`, not
  a `Renderer`, so it runs without a GPU.

One iteration: change, then `cargo test --lib <surface>::` (deterministic, fast,
asserts behavior + invariants), then render the PNG and look at it for what tests
cannot assert (kerning, pill padding, vertical centering), checking the a11y and
text-layout dumps if any text looks off. Lean on the test leg first; treat the
image as the last mile. For spacing/overlap/alignment specifically, the
text-layout dump is the answer, not the image. When a glyph diagnostic is genuinely
visual (does a face render bold? does a pill sit behind the right run?), tint each
piece a distinct color and crop+zoom the boundary (`magick … -crop … -resize`) —
color separation cuts through antialiasing where a same-color zoom cannot. Keep
PNGs under `target/`; never commit them.

If a command cannot be run in the current environment, say so in the final
response and explain what was not verified.

## Change Hygiene

- Use `rg`/`rg --files` for codebase search and avoid searching vendored or
  reference trees unless needed.
- Keep changes scoped to the task. Do not reformat or refactor unrelated files.
- Do not edit generated packaging assets unless the task is packaging-specific.
- Do not log secrets, tokens, API keys, or raw credential material.
- Preserve user work in the tree. Never revert unrelated changes without an
  explicit request.
- For routine checks or small edits, do not send progress updates. Only send an
  update when blocked, requesting approval, or surfacing a user-visible decision.
  When an update is necessary, keep it plain and minimal.
