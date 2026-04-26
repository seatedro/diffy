# AGENTS.md

Guidance for agents working in `crates/halogen`. The root `AGENTS.md` still
applies; this node is the high-signal map for the local UI/reactivity toolkit.

## Purpose And Scope

Halogen provides Diffy's small UI foundation: fine-grained reactive signals,
pure geometry, hit-testing primitives, style data, immediate-mode scene
primitives, and re-exports for the `view!` and `Store` macros.

Halogen does not own Diffy's design tokens, app action enum, renderer, glyph
cache, OS event loop, overlay policy, or product state. Those live in the app
and adapt Halogen's generic data.

## Related Context

- Root product and UI rules: `../../AGENTS.md`
- Halogen architecture overview: `ARCHITECTURE.md`
- Macro parser and lowering contracts: `../halogen-macros/AGENTS.md`
- Diffy builder/style methods that `view!` calls: `../../src/ui/style.rs`

## Core Contracts

- Read `ARCHITECTURE.md` before changing signals, scene primitives, style data,
  hit-testing, or the `view!` contract.
- `Signal<T>` is a small `Copy` handle into `SignalStore`; values live in arena
  slots indexed by `{ index, generation }`.
- Reactive propagation has three states: `Clean`, `Check`, and `Dirty`. Writes
  mark the source dirty and transitive subscribers for lazy checking; memo reads
  settle the graph.
- `create_memo` relies on `PartialEq` to stop invalidation cascades. Avoid memos
  whose values churn every frame.
- `any_dirty()` is the frame-loop signal that something was written since the
  last `clear_dirty()`. Clear it after rendering, not before consumers observe
  the redraw need.
- Re-entrant signal access is a bug. The store intentionally panics on nested
  mutable/immutable borrow conflicts instead of hiding feedback-loop writes.
- `scene::Scene` is pure data. Halogen emits primitives; Diffy's renderer
  decides how to batch, cache, clip, and draw them.
- `hit` is generic over the host click-result payload. Keep action routing in
  the app; Halogen should only resolve geometry, z-order, blocking, cursor, and
  identity data.
- `style::ElementStyle` is pure layout/visual data. Diffy's fluent `Styled`
  helpers and token systems live outside this crate.
- `class="..."` in `view!` is not CSS. It lowers to Rust builder method calls
  such as `.flex_row()` or mapped aliases from `halogen-macros`.

## Usage Patterns

- Fix shared UI contract bugs in Halogen when the bug is in the primitive
  semantics, not at one app call site.
- Keep primitive structs compact, cloneable when useful, and renderer-agnostic.
- Prefer generic payloads and identities at the Halogen boundary so the app can
  own policy and actions.
- Add tests around behavior that affects layout, clipping, hit ordering,
  reactivity propagation, or macro-generated store access.

## Anti-Patterns

- Do not reason from browser CSS behavior. Verify the actual Taffy/style/paint
  path and the macro lowering before claiming semantics.
- Do not add wgpu, glyphon, winit, app `Action`, or theme-token dependencies to
  Halogen.
- Do not patch only a Diffy component if the underlying Halogen primitive
  contract is wrong.
- Do not introduce reactive effects that write during their own execution
  without a deferred-write design.
- Do not let scene or hit-test construction allocate unbounded data per frame
  without a clear app-level reuse story.

## Validation

- Focused crate tests: `cargo test -p halogen`
- Macro contract fallout: `cargo test -p halogen-macros` and then
  `cargo test -p halogen`
- UI behavior changes that cross into Diffy: run the smallest relevant app test
  or smoke path in addition to crate tests.

## Maintenance

Keep cross-cutting UI rules at the root or in `ARCHITECTURE.md` when they apply
beyond this crate. Keep this node focused on the contracts an agent needs before
editing Halogen itself.
