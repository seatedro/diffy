# Element System — diffy's GPUI

Build a retained-mode element layer on top of our existing wgpu renderer.
Components describe layout declaratively; Taffy resolves coordinates; the
renderer paints. No manual Rect arithmetic.

## Architecture

```
Component (RenderOnce)
    ↓ .render() returns
Element tree (div, text, custom elements)
    ↓ request_layout()
Taffy layout nodes
    ↓ compute_layout()
Bounds<Pixels> per element
    ↓ prepaint() — hitboxes, scroll offsets
    ↓ paint() — emit scene primitives
Renderer (existing wgpu pipelines)
```

## Phases

### Phase 1 — Core Element Model

The foundation. An `Element` trait, a Taffy-backed layout engine, and a
`Div` container that can hold children.

**Files:** `src/ui/element.rs`, `src/ui/layout_engine.rs`

**Element trait:**
```rust
trait Element: 'static {
    type LayoutState;

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        cx: &ElementContext,
    ) -> (LayoutId, Self::LayoutState);

    fn paint(
        &mut self,
        bounds: Bounds,
        state: &mut Self::LayoutState,
        scene: &mut Scene,
        cx: &ElementContext,
    );
}
```

**AnyElement:** type-erased wrapper so containers can hold mixed children.

**LayoutEngine:** wraps `TaffyTree`, provides:
- `request_layout(style, children) -> LayoutId`
- `request_measured_layout(style, measure_fn) -> LayoutId` (for text)
- `compute_layout(root, available_space)`
- `layout_bounds(id) -> Bounds`

**Div:**
- Holds `Vec<AnyElement>` children
- `request_layout`: creates Taffy node with children's layout IDs
- `paint`: paints background/border/shadow, then paints children

**Done when:** you can write `div().child(div())` and it lays out correctly.

---

### Phase 2 — Style System

Fluent Tailwind-like API so components read cleanly.

**Files:** `src/ui/style.rs` (or extend `element.rs`)

**Style struct** — maps 1:1 to Taffy properties plus visual properties:
```rust
struct Style {
    // Layout (→ Taffy)
    display: Display,
    flex_direction: FlexDirection,
    flex_grow: f32,
    flex_shrink: f32,
    gap: Size,
    padding: Edges,
    margin: Edges,
    size: Size,
    min_size: Size,
    max_size: Size,
    overflow: Overflow,
    align_items: AlignItems,
    justify_content: JustifyContent,

    // Visual (→ paint)
    background: Option<Color>,
    border_color: Option<Color>,
    border_widths: Edges,
    corner_radii: Corners,
    shadow: Vec<ShadowStyle>,
    opacity: f32,
    text: TextStyle,
}
```

**Styled trait** — fluent setters:
```rust
trait Styled: Sized {
    fn style(&mut self) -> &mut Style;

    fn flex(self) -> Self;
    fn flex_col(self) -> Self;
    fn flex_row(self) -> Self;
    fn flex_1(self) -> Self;
    fn gap(self, v: f32) -> Self;
    fn p(self, v: f32) -> Self;        // padding all
    fn px(self, v: f32) -> Self;       // padding horizontal
    fn py(self, v: f32) -> Self;       // padding vertical
    fn bg(self, color: Color) -> Self;
    fn border(self, color: Color) -> Self;
    fn rounded(self, r: f32) -> Self;
    fn text_color(self, c: Color) -> Self;
    fn text_sm(self) -> Self;
    fn text_lg(self) -> Self;
    fn w(self, v: impl Into<Length>) -> Self;
    fn h(self, v: impl Into<Length>) -> Self;
    fn items_center(self) -> Self;
    fn justify_center(self) -> Self;
    fn justify_between(self) -> Self;
    fn overflow_hidden(self) -> Self;
    fn overflow_y_scroll(self) -> Self;
}
```

**Done when:** `div().flex().gap(8.0).p(16.0).bg(color).child(...)` works.

---

### Phase 3 — Text Element

Text needs intrinsic sizing — it must tell Taffy how big it is.

**Approach:** Text element uses `request_measured_layout` with a measure
function that calls glyphon to measure the string. Taffy calls this
during layout to determine intrinsic size, and respects max-width for
wrapping.

```rust
fn text(content: impl Into<String>) -> TextElement;
```

Features:
- Single-line and wrapping text
- Ellipsis truncation when overflow is hidden
- Styled spans (bold, color per range) — `StyledText`
- Font size/family from the element's TextStyle

**Done when:** `div().w(200.0).child("Hello world that wraps")` wraps
correctly, and `div().child("Title").text_lg()` sizes itself.

---

### Phase 4 — Interactivity

Hover, click, focus. Elements register hitboxes during prepaint; the
frame runner does hit testing after layout.

**Hitbox system:**
- During `prepaint`, interactive elements call `cx.insert_hitbox(bounds)`
- After all elements prepaint, hit-test mouse position against hitboxes
- Elements query `cx.is_hovered(hitbox_id)` during paint

**InteractiveElement trait:**
```rust
trait InteractiveElement: Sized {
    fn on_click(self, handler: impl Fn(&ClickEvent) + 'static) -> Self;
    fn on_mouse_down(self, handler: impl Fn(&MouseEvent) + 'static) -> Self;
    fn on_scroll(self, handler: impl Fn(&ScrollEvent) + 'static) -> Self;
    fn hover(self, style: impl Fn(Style) -> Style) -> Self;
    fn active(self, style: impl Fn(Style) -> Style) -> Self;
    fn cursor(self, cursor: CursorStyle) -> Self;
}
```

**Done when:** `div().on_click(|_| action).hover(|s| s.bg(hover_color))`
highlights on hover and fires the action on click.

---

### Phase 5 — Scroll Containers

Any div with `overflow_y_scroll()` becomes a scroll container.

**ScrollHandle:**
- Tracks scroll offset, content height, viewport height
- Updated by scroll wheel events
- Applied as element offset during prepaint (children shift up)

**Content masking:**
- Scroll container pushes a clip rect during paint
- Children outside the clip are culled

**Scrollbar rendering:**
- Optional visible scrollbar thumb (auto-hiding)
- Uses ghost element colors

**Done when:** a file list with 100 items scrolls smoothly with a
visible thumb that auto-hides.

---

### Phase 6 — Port shell.rs

Rewrite the entire UI using the element system. This is where all the
manual `Rect` arithmetic, `vstack`/`hstack` helpers, and `Label::paint()`
calls get replaced.

**Before (current):**
```rust
let content = rect.pad(Sp::XL, 0.0, Sp::XL, 0.0);
let label_h = theme.metrics.heading_font_size * 1.35;
Label::new(repo_label)
    .style(TextStyle::Heading)
    .paint(frame, Rect { x: content.x, y: ..., width: ..., height: ... }, theme);
```

**After:**
```rust
div()
    .flex()
    .items_center()
    .px(20.0)
    .h(52.0)
    .bg(theme.colors.title_bar_background)
    .child(
        text(repo_label).text_lg().text_color(theme.colors.text_strong)
    )
    .child(spacer())
    .child(
        div().flex().gap(8.0)
            .child(button("Compare", Action::OpenCompareSheet))
            .child(button("PR", Action::OpenPullRequestModal))
    )
```

**Sub-tasks:**
- [ ] Title bar
- [ ] Sidebar + file list (scroll container)
- [ ] Main surface + viewport toolbar
- [ ] Status bar
- [ ] Empty state / loading state cards
- [ ] Compare sheet modal
- [ ] Repo picker modal
- [ ] Ref picker modal
- [ ] Command palette
- [ ] PR modal
- [ ] Auth modal
- [ ] Toasts

**Done when:** `shell.rs` no longer imports `Rect` or computes pixel
positions manually. All layout flows through the element system.

---

## Execution Order

```
Phase 1 (element + layout) ──→ Phase 2 (styles)
                                    ↓
                               Phase 3 (text)
                                    ↓
                               Phase 4 (interactivity)
                                    ↓
                               Phase 5 (scroll)
                                    ↓
                               Phase 6 (port shell.rs)
```

Each phase is a commit point. Phases 1-3 are the minimum to start
porting. Phase 4 is needed for buttons/hover. Phase 5 for the file list.
Phase 6 is the payoff.
