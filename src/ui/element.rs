//! Core element model for declarative UI layout.
//!
//! Elements describe what they want (size, flex, padding) and a layout engine
//! (Taffy) resolves concrete pixel coordinates. The lifecycle is:
//!
//! 1. **request_layout** — declare Taffy style and children.
//! 2. **prepaint** — register hitboxes, resolve interaction state.
//! 3. **paint** — emit scene primitives using resolved hover/hit state.

use crate::ui::design::{Alpha, Sz};
use crate::render::Scene;
use crate::render::scene::{BlurRegionPrimitive, EffectQuadPrimitive, EffectType, Rect};
use crate::ui::actions::Action;
use crate::ui::shell::CursorHint;
use crate::ui::signals::{Signal, SignalStore};
use crate::ui::theme::Theme;

pub use taffy::NodeId as LayoutId;

// ---------------------------------------------------------------------------
// Bounds — the resolved rectangle for a laid-out element
// ---------------------------------------------------------------------------

pub type Bounds = Rect;

// ---------------------------------------------------------------------------
// HitRegion — clickable area registered during paint
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct HitRegion {
    pub rect: Rect,
    pub action: Action,
    pub cursor: CursorHint,
}

// ---------------------------------------------------------------------------
// HitboxId / Hitbox — hitbox system for prepaint-phase interaction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HitboxId(usize);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitboxBehavior {
    /// Normal hitbox — participates in hover detection.
    Normal,
    /// Blocks mouse events from reaching hitboxes painted earlier (behind it).
    BlockMouse,
}

#[derive(Debug, Clone)]
pub struct Hitbox {
    pub id: HitboxId,
    pub bounds: Bounds,
    pub behavior: HitboxBehavior,
    pub z_index: i32,
}

// ---------------------------------------------------------------------------
// ElementContext — shared state available during layout, prepaint, and paint
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// ScrollRegion — registered during prepaint for scroll wheel dispatch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ScrollRegion {
    pub bounds: Bounds,
    pub action_builder: ScrollActionBuilder,
}

/// A scrollbar track region — registered during paint for click-to-scroll and drag.
#[derive(Debug, Clone)]
pub struct ScrollbarTrack {
    pub track_rect: Rect,
    pub thumb_top: f32,
    pub thumb_height: f32,
    pub content_height: f32,
    pub viewport_height: f32,
    pub action_builder: ScrollActionBuilder,
}

/// How to convert a scroll delta (in lines) into an Action.
#[derive(Debug, Clone)]
pub enum ScrollActionBuilder {
    /// Emit `Action::ScrollFileList(delta)`.
    FileList,
    /// Emit `Action::ScrollViewportLines(delta)`.
    ViewportLines,
    /// Use a custom action constructor.
    Custom(fn(i32) -> Action),
}

impl ScrollActionBuilder {
    pub fn build(&self, delta: i32) -> Action {
        match self {
            Self::FileList => Action::ScrollFileList(delta),
            Self::ViewportLines => Action::ScrollViewportLines(delta),
            Self::Custom(f) => f(delta),
        }
    }
}

// ---------------------------------------------------------------------------
// ElementContext
// ---------------------------------------------------------------------------

pub struct ElementContext<'a> {
    pub theme: &'a Theme,
    pub scale_factor: f32,
    pub font_system: &'a mut glyphon::FontSystem,
    pub mouse_position: Option<(f32, f32)>,
    pub hits: Vec<HitRegion>,
    pub scroll_regions: Vec<ScrollRegion>,
    pub focus: Option<crate::ui::state::FocusTarget>,
    pub signal_store: &'a mut SignalStore,
    pub clock_ms: u64,
    pub ui_signals: Option<crate::ui::ui_signals::UiSignals>,
    pub debug_wireframe: bool,
    pub text_input_hit_areas: Vec<TextInputHitArea>,
    pub scrollbar_tracks: Vec<ScrollbarTrack>,
    hitboxes: Vec<Hitbox>,
    hovered_hitboxes: Vec<HitboxId>,
    next_hitbox_id: usize,
    z_index_stack: Vec<i32>,
    element_offset_stack: Vec<(f32, f32)>,
}

impl<'a> ElementContext<'a> {
    pub fn new(
        theme: &'a Theme,
        scale_factor: f32,
        font_system: &'a mut glyphon::FontSystem,
        mouse_position: Option<(f32, f32)>,
        signal_store: &'a mut SignalStore,
    ) -> Self {
        Self {
            theme,
            scale_factor,
            font_system,
            mouse_position,
            hits: Vec::new(),
            scroll_regions: Vec::new(),
            focus: None,
            signal_store,
            clock_ms: 0,
            ui_signals: None,
            debug_wireframe: false,
            text_input_hit_areas: Vec::new(),
            scrollbar_tracks: Vec::new(),
            hitboxes: Vec::new(),
            hovered_hitboxes: Vec::new(),
            next_hitbox_id: 0,
            z_index_stack: vec![0],
            element_offset_stack: vec![(0.0, 0.0)],
        }
    }

    /// Read a signal's value (clones it out). Tracked by the current observer scope.
    pub fn read<T: 'static + Clone>(&self, signal: Signal<T>) -> T {
        self.signal_store.read(signal)
    }

    /// Read a signal's value without registering a dependency.
    pub fn read_untracked<T: 'static + Clone>(&self, signal: Signal<T>) -> T {
        self.signal_store.read_untracked(signal)
    }

    /// Access a signal's value by reference without cloning.
    pub fn with_signal<T: 'static, R>(&self, signal: Signal<T>, f: impl FnOnce(&T) -> R) -> R {
        self.signal_store.with(signal, f)
    }

    /// Replace a signal's value.
    pub fn write<T: 'static>(&mut self, signal: Signal<T>, value: T) {
        self.signal_store.write(signal, value);
    }

    /// Mutate a signal's value in place.
    pub fn update<T: 'static>(&mut self, signal: Signal<T>, f: impl FnOnce(&mut T)) {
        self.signal_store.update(signal, f);
    }

    pub fn with_focus(mut self, focus: Option<crate::ui::state::FocusTarget>) -> Self {
        self.focus = focus;
        self
    }

    pub fn with_clock(mut self, clock_ms: u64) -> Self {
        self.clock_ms = clock_ms;
        self
    }

    pub fn with_ui_signals(mut self, signals: crate::ui::ui_signals::UiSignals) -> Self {
        self.ui_signals = Some(signals);
        self
    }

    pub fn is_focused(&self, target: crate::ui::state::FocusTarget) -> bool {
        self.focus == Some(target)
    }

    pub fn current_z_index(&self) -> i32 {
        *self.z_index_stack.last().unwrap_or(&0)
    }

    pub fn push_z_index(&mut self, z: i32) {
        self.z_index_stack.push(z);
    }

    pub fn pop_z_index(&mut self) {
        if self.z_index_stack.len() > 1 {
            self.z_index_stack.pop();
        }
    }

    pub fn current_element_offset(&self) -> (f32, f32) {
        *self.element_offset_stack.last().unwrap_or(&(0.0, 0.0))
    }

    pub fn push_element_offset(&mut self, offset_x: f32, offset_y: f32) {
        let (base_x, base_y) = self.current_element_offset();
        self.element_offset_stack
            .push((base_x + offset_x, base_y + offset_y));
    }

    pub fn pop_element_offset(&mut self) {
        if self.element_offset_stack.len() > 1 {
            self.element_offset_stack.pop();
        }
    }

    /// Register a hitbox during prepaint. Returns an ID for later hover queries.
    pub fn insert_hitbox(&mut self, bounds: Bounds, behavior: HitboxBehavior) -> HitboxId {
        let id = HitboxId(self.next_hitbox_id);
        self.next_hitbox_id += 1;
        self.hitboxes.push(Hitbox {
            id,
            bounds,
            behavior,
            z_index: self.current_z_index(),
        });
        id
    }

    /// Returns true if the given hitbox is hovered (determined after `run_hit_test`).
    pub fn is_hovered(&self, id: HitboxId) -> bool {
        self.hovered_hitboxes.contains(&id)
    }

    /// Run hit-testing: walk hitboxes back-to-front (last registered = topmost).
    /// If a `BlockMouse` hitbox contains the mouse, all hitboxes behind it that
    /// overlap with the blocking hitbox are excluded from hover.
    pub fn run_hit_test(&mut self) {
        self.hovered_hitboxes.clear();
        let mouse = match self.mouse_position {
            Some(pos) => pos,
            None => return,
        };

        // Collect which hitboxes the mouse is inside.
        let mut candidates: Vec<(HitboxId, Bounds, HitboxBehavior, i32)> = Vec::new();
        for hb in &self.hitboxes {
            if hb.bounds.contains(mouse.0, mouse.1) {
                candidates.push((hb.id, hb.bounds, hb.behavior, hb.z_index));
            }
        }

        // Reverse so last-registered (topmost within same paint order) comes first,
        // then stable-sort by z_index descending. Result: highest z first, and
        // within same z, last-registered first.
        candidates.reverse();
        candidates.sort_by(|a, b| b.3.cmp(&a.3));

        let mut blocked_regions: Vec<Bounds> = Vec::new();

        for &(id, bounds, behavior, _z) in &candidates {
            let is_blocked = blocked_regions
                .iter()
                .any(|blocker| blocker.intersection(bounds).is_some());

            if !is_blocked {
                self.hovered_hitboxes.push(id);
            }

            if behavior == HitboxBehavior::BlockMouse {
                blocked_regions.push(bounds);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Element trait
// ---------------------------------------------------------------------------

/// Every UI node implements `Element`. The lifecycle is:
///
/// 1. **request_layout** — declare your Taffy style and children. Returns a
///    `LayoutId` and arbitrary per-element state.
/// 2. **prepaint** — given resolved bounds, register hitboxes and resolve
///    interaction state. Returns arbitrary prepaint state.
/// 3. **paint** — emit scene primitives using resolved bounds and prepaint state.
pub trait Element: 'static {
    type LayoutState: 'static;
    type PrepaintState: 'static;

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        cx: &mut ElementContext,
    ) -> (LayoutId, Self::LayoutState);

    fn prepaint(
        &mut self,
        bounds: Bounds,
        layout_state: &mut Self::LayoutState,
        engine: &LayoutEngine,
        cx: &mut ElementContext,
    ) -> Self::PrepaintState;

    fn paint(
        &mut self,
        bounds: Bounds,
        layout_state: &mut Self::LayoutState,
        prepaint_state: &mut Self::PrepaintState,
        engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
    );
}

// ---------------------------------------------------------------------------
// AnyElement — type-erased element
// ---------------------------------------------------------------------------

pub struct AnyElement {
    inner: Box<dyn AnyElementImpl>,
}

impl AnyElement {
    pub fn new<E: Element>(element: E) -> Self {
        Self {
            inner: Box::new(ElementHolder {
                element,
                layout_state: None,
                prepaint_state: None,
                layout_id: None,
            }),
        }
    }

    pub fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        cx: &mut ElementContext,
    ) -> LayoutId {
        self.inner.request_layout(engine, cx)
    }

    pub fn prepaint(&mut self, engine: &LayoutEngine, cx: &mut ElementContext) {
        self.inner.prepaint(engine, cx, 0.0, 0.0);
    }

    pub fn prepaint_with_offset(
        &mut self,
        engine: &LayoutEngine,
        cx: &mut ElementContext,
        offset_x: f32,
        offset_y: f32,
    ) {
        self.inner.prepaint(engine, cx, offset_x, offset_y);
    }

    pub fn paint(&mut self, engine: &LayoutEngine, scene: &mut Scene, cx: &mut ElementContext) {
        self.inner.paint(engine, scene, cx, 0.0, 0.0);
    }

    pub fn paint_with_offset(
        &mut self,
        engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
        offset_x: f32,
        offset_y: f32,
    ) {
        self.inner.paint(engine, scene, cx, offset_x, offset_y);
    }
}

trait AnyElementImpl {
    fn request_layout(&mut self, engine: &mut LayoutEngine, cx: &mut ElementContext) -> LayoutId;
    fn prepaint(
        &mut self,
        engine: &LayoutEngine,
        cx: &mut ElementContext,
        offset_x: f32,
        offset_y: f32,
    );
    fn paint(
        &mut self,
        engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
        offset_x: f32,
        offset_y: f32,
    );
}

struct ElementHolder<E: Element> {
    element: E,
    layout_state: Option<E::LayoutState>,
    prepaint_state: Option<E::PrepaintState>,
    layout_id: Option<LayoutId>,
}

impl<E: Element> AnyElementImpl for ElementHolder<E> {
    fn request_layout(&mut self, engine: &mut LayoutEngine, cx: &mut ElementContext) -> LayoutId {
        let (id, state) = self.element.request_layout(engine, cx);
        self.layout_id = Some(id);
        self.layout_state = Some(state);
        id
    }

    fn prepaint(
        &mut self,
        engine: &LayoutEngine,
        cx: &mut ElementContext,
        offset_x: f32,
        offset_y: f32,
    ) {
        let id = self
            .layout_id
            .expect("prepaint called before request_layout");
        let mut bounds = engine.layout_bounds(id);
        let (base_offset_x, base_offset_y) = cx.current_element_offset();
        let total_offset_x = base_offset_x + offset_x;
        let total_offset_y = base_offset_y + offset_y;
        bounds.x += total_offset_x;
        bounds.y += total_offset_y;
        let layout_state = self
            .layout_state
            .as_mut()
            .expect("prepaint called before request_layout");
        cx.push_element_offset(offset_x, offset_y);
        let prepaint_state = self.element.prepaint(bounds, layout_state, engine, cx);
        self.prepaint_state = Some(prepaint_state);
        cx.pop_element_offset();
    }

    fn paint(
        &mut self,
        engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
        offset_x: f32,
        offset_y: f32,
    ) {
        let id = self.layout_id.expect("paint called before request_layout");
        let mut bounds = engine.layout_bounds(id);
        let (base_offset_x, base_offset_y) = cx.current_element_offset();
        let total_offset_x = base_offset_x + offset_x;
        let total_offset_y = base_offset_y + offset_y;
        bounds.x += total_offset_x;
        bounds.y += total_offset_y;
        let layout_state = self
            .layout_state
            .as_mut()
            .expect("paint called before request_layout");
        let prepaint_state = self
            .prepaint_state
            .as_mut()
            .expect("paint called before prepaint");
        cx.push_element_offset(offset_x, offset_y);
        self.element
            .paint(bounds, layout_state, prepaint_state, engine, scene, cx);
        cx.pop_element_offset();
    }
}

// ---------------------------------------------------------------------------
// IntoAnyElement — conversion trait
// ---------------------------------------------------------------------------

pub trait IntoAnyElement {
    fn into_any(self) -> AnyElement;
}

impl IntoAnyElement for AnyElement {
    fn into_any(self) -> AnyElement {
        self
    }
}

// ---------------------------------------------------------------------------
// RenderOnce — component-level trait
// ---------------------------------------------------------------------------

/// Components implement `RenderOnce` to produce a tree of elements.
/// The component is consumed (moved) when rendered.
pub trait RenderOnce: 'static + Sized {
    fn render(self, cx: &ElementContext) -> AnyElement;
}

/// Adapter that wraps a `RenderOnce` component into an `Element`.
struct ComponentElement<C: RenderOnce> {
    component: Option<C>,
    rendered: Option<AnyElement>,
}

impl<C: RenderOnce> Element for ComponentElement<C> {
    type LayoutState = ();
    type PrepaintState = ();

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        cx: &mut ElementContext,
    ) -> (LayoutId, ()) {
        let component = self
            .component
            .take()
            .expect("ComponentElement rendered twice");
        let mut any = component.render(cx);
        let id = any.request_layout(engine, cx);
        self.rendered = Some(any);
        (id, ())
    }

    fn prepaint(
        &mut self,
        _bounds: Bounds,
        _layout_state: &mut (),
        engine: &LayoutEngine,
        cx: &mut ElementContext,
    ) -> () {
        if let Some(ref mut rendered) = self.rendered {
            rendered.prepaint(engine, cx);
        }
    }

    fn paint(
        &mut self,
        _bounds: Bounds,
        _layout_state: &mut (),
        _prepaint_state: &mut (),
        engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
    ) {
        if let Some(ref mut rendered) = self.rendered {
            rendered.paint(engine, scene, cx);
        }
    }
}

/// Blanket impl: any `RenderOnce` can be converted into an `AnyElement`.
impl<C: RenderOnce> IntoAnyElement for C {
    fn into_any(self) -> AnyElement {
        AnyElement::new(ComponentElement {
            component: Some(self),
            rendered: None,
        })
    }
}

/// Helper to wrap any `Element` implementor into an `AnyElement`.
/// Use this for types that implement `Element` directly (not `RenderOnce`).
fn element_into_any<E: Element>(element: E) -> AnyElement {
    AnyElement::new(element)
}

// ---------------------------------------------------------------------------
// MeasureFunc — stored per-node for intrinsic sizing (text)
// ---------------------------------------------------------------------------

type MeasureFn = Box<
    dyn Fn(taffy::Size<Option<f32>>, taffy::Size<taffy::AvailableSpace>) -> taffy::Size<f32>
        + Send
        + Sync,
>;

enum NodeMeasure {
    /// Leaf with no measure — sized by Taffy style alone.
    None,
    /// Leaf with an intrinsic measure function (e.g. text).
    Measure(MeasureFn),
}

// ---------------------------------------------------------------------------
// LayoutEngine — wraps TaffyTree
// ---------------------------------------------------------------------------

pub struct LayoutEngine {
    tree: taffy::TaffyTree<NodeMeasure>,
}

impl LayoutEngine {
    pub fn new() -> Self {
        Self {
            tree: taffy::TaffyTree::new(),
        }
    }

    /// Create a layout node with the given style and children.
    pub fn request_layout(&mut self, style: taffy::Style, children: &[LayoutId]) -> LayoutId {
        if children.is_empty() {
            self.tree
                .new_leaf_with_context(style, NodeMeasure::None)
                .expect("taffy new_leaf failed")
        } else {
            self.tree
                .new_with_children(style, children)
                .expect("taffy new_with_children failed")
        }
    }

    /// Create a leaf node that uses a measure function for intrinsic sizing.
    pub fn request_measured_layout(
        &mut self,
        style: taffy::Style,
        measure: impl Fn(
            taffy::Size<Option<f32>>,
            taffy::Size<taffy::AvailableSpace>,
        ) -> taffy::Size<f32>
        + Send
        + Sync
        + 'static,
    ) -> LayoutId {
        self.tree
            .new_leaf_with_context(style, NodeMeasure::Measure(Box::new(measure)))
            .expect("taffy new_leaf_with_context failed")
    }

    /// Compute layout for the entire tree rooted at `root`.
    pub fn compute_layout(&mut self, root: LayoutId, width: f32, height: f32) {
        self.tree
            .compute_layout_with_measure(
                root,
                taffy::Size {
                    width: taffy::AvailableSpace::Definite(width),
                    height: taffy::AvailableSpace::Definite(height),
                },
                |known, available, _node_id, context, _style| {
                    if let Some(NodeMeasure::Measure(f)) = context {
                        f(known, available)
                    } else {
                        taffy::Size::ZERO
                    }
                },
            )
            .expect("taffy compute_layout failed");
    }

    /// Get the resolved bounds for a layout node, in absolute coordinates.
    pub fn layout_bounds(&self, id: LayoutId) -> Bounds {
        let mut x = 0.0_f32;
        let mut y = 0.0_f32;

        // Walk up the tree to accumulate parent offsets.
        let mut current = id;
        loop {
            let layout = self.tree.layout(current).expect("invalid layout id");
            x += layout.location.x;
            y += layout.location.y;
            match self.tree.parent(current) {
                Some(parent) => current = parent,
                None => break,
            }
        }

        let layout = self.tree.layout(id).expect("invalid layout id");
        Bounds {
            x,
            y,
            width: layout.size.width,
            height: layout.size.height,
        }
    }

    /// Clear all nodes for the next frame.
    pub fn clear(&mut self) {
        self.tree.clear();
    }
}

// ---------------------------------------------------------------------------
// render_element — top-level entry point
// ---------------------------------------------------------------------------

/// Lay out, prepaint, hit-test, and paint an element tree into the given scene.
/// Returns the hit regions accumulated during paint.
pub fn render_element(
    root: &mut AnyElement,
    scene: &mut Scene,
    cx: &mut ElementContext,
    width: f32,
    height: f32,
) {
    let mut engine = LayoutEngine::new();
    let root_id = root.request_layout(&mut engine, cx);
    engine.compute_layout(root_id, width, height);
    root.prepaint(&engine, cx);
    cx.run_hit_test();
    root.paint(&engine, scene, cx);
}

// ---------------------------------------------------------------------------
// Div — the fundamental container element
// ---------------------------------------------------------------------------

use crate::render::{BorderPrimitive, FontWeight, RoundedRectPrimitive, ShadowPrimitive};
use crate::ui::style::{ElementStyle, StyleOverride, Styled, apply_override};
use crate::ui::theme::Color;

// ---------------------------------------------------------------------------
// BackgroundEffect — procedural GPU-computed backgrounds
// ---------------------------------------------------------------------------

/// A procedural background effect rendered by the GPU effect shader.
#[derive(Debug, Clone, Copy)]
pub enum BackgroundEffect {
    /// Simplex noise blended between two colors. `scale` controls noise
    /// frequency (try 0.01–0.05 for subtle, 0.1+ for coarse).
    NoiseGradient {
        scale: f32,
        color_a: Color,
        color_b: Color,
    },
    /// Linear gradient between two colors at the given angle (radians).
    /// 0 = left→right, π/2 = top→bottom.
    LinearGradient {
        angle: f32,
        color_a: Color,
        color_b: Color,
    },
    /// Radial gradient — `color_a` at center, `color_b` at edge.
    RadialGradient { color_a: Color, color_b: Color },
    /// Animated diagonal shimmer sweep (loading skeleton).
    /// `speed` controls animation speed (try 1.0–3.0).
    Shimmer {
        base: Color,
        highlight: Color,
        speed: f32,
    },
    /// Edge darkening/tinting. `intensity` controls falloff (try 0.3–0.8).
    Vignette { color: Color, intensity: f32 },
    /// Flat semi-transparent color overlay.
    ColorTint { color: Color },
}

/// Convenience: create a noise gradient background effect.
pub fn noise_gradient(scale: f32, color_a: Color, color_b: Color) -> BackgroundEffect {
    BackgroundEffect::NoiseGradient {
        scale,
        color_a,
        color_b,
    }
}

/// Convenience: create a linear gradient background effect.
pub fn linear_gradient(angle: f32, color_a: Color, color_b: Color) -> BackgroundEffect {
    BackgroundEffect::LinearGradient {
        angle,
        color_a,
        color_b,
    }
}

/// Convenience: create a radial gradient (center → edge).
pub fn radial_gradient(center: Color, edge: Color) -> BackgroundEffect {
    BackgroundEffect::RadialGradient {
        color_a: center,
        color_b: edge,
    }
}

/// Convenience: create an animated shimmer (loading skeleton effect).
pub fn shimmer(base: Color, highlight: Color, speed: f32) -> BackgroundEffect {
    BackgroundEffect::Shimmer {
        base,
        highlight,
        speed,
    }
}

/// Convenience: create a vignette (edge darkening).
pub fn vignette(color: Color, intensity: f32) -> BackgroundEffect {
    BackgroundEffect::Vignette { color, intensity }
}

/// Convenience: create a flat color tint overlay.
pub fn color_tint(color: Color) -> BackgroundEffect {
    BackgroundEffect::ColorTint { color }
}

/// A flexbox container. The core building block.
pub struct Div {
    base_style: ElementStyle,
    hover_style: Option<StyleOverride>,
    bg_effect: Option<BackgroundEffect>,
    blur_radius: Option<f32>,
    children: Vec<AnyElement>,
    on_click: Option<Action>,
    on_scroll: Option<ScrollActionBuilder>,
    cursor: CursorHint,
    scroll_y: f32,
    scroll_total_height: f32,
    hide_scrollbar: bool,
    clips: bool,
    focus_target: Option<crate::ui::state::FocusTarget>,
}

/// Create a new div element.
pub fn div() -> Div {
    Div {
        base_style: ElementStyle::default(),
        hover_style: None,
        bg_effect: None,
        blur_radius: None,
        children: Vec::new(),
        on_click: None,
        on_scroll: None,
        cursor: CursorHint::Default,
        scroll_y: 0.0,
        scroll_total_height: 0.0,
        hide_scrollbar: false,
        clips: false,
        focus_target: None,
    }
}

impl Styled for Div {
    fn element_style_mut(&mut self) -> &mut ElementStyle {
        &mut self.base_style
    }
}

impl Div {
    // -- Children --

    pub fn child(mut self, child: impl IntoAnyElement) -> Self {
        self.children.push(child.into_any());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = AnyElement>) -> Self {
        self.children.extend(children);
        self
    }

    pub fn optional_child(mut self, child: Option<impl IntoAnyElement>) -> Self {
        if let Some(c) = child {
            self.children.push(c.into_any());
        }
        self
    }

    pub fn children_from<I, E>(mut self, iter: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: IntoAnyElement,
    {
        for item in iter {
            self.children.push(item.into_any());
        }
        self
    }

    // -- Interaction --

    pub fn on_click(mut self, action: Action) -> Self {
        self.on_click = Some(action);
        self.cursor = CursorHint::Pointer;
        self
    }

    pub fn cursor(mut self, cursor: CursorHint) -> Self {
        self.cursor = cursor;
        self
    }

    /// Register a scroll action for this div. Scroll wheel events inside
    /// this div's bounds will dispatch through the action builder.
    pub fn on_scroll(mut self, builder: ScrollActionBuilder) -> Self {
        self.on_scroll = Some(builder);
        self
    }

    /// Full style override on hover.
    pub fn hover(mut self, f: impl FnOnce(StyleOverride) -> StyleOverride) -> Self {
        self.hover_style = Some(f(StyleOverride::default()));
        self
    }

    /// Convenience: set only the hover background.
    pub fn hover_bg(self, color: Color) -> Self {
        self.hover(|s| s.bg(color))
    }

    /// Conditionally apply style/config changes.
    pub fn when(self, condition: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if condition { f(self) } else { self }
    }

    // -- Scroll / clip --

    pub fn scroll_y(mut self, offset: f32) -> Self {
        self.scroll_y = offset;
        self.clips = true;
        // Tell taffy the element is a scroll container so it constrains to
        // the available space instead of expanding to fit all children.
        self.base_style.layout.overflow.y = taffy::Overflow::Hidden;
        self
    }

    pub fn scroll_total(mut self, total_height: f32) -> Self {
        self.scroll_total_height = total_height;
        self
    }

    pub fn hide_scrollbar(mut self) -> Self {
        self.hide_scrollbar = true;
        self
    }

    pub fn focus_ring(mut self, target: crate::ui::state::FocusTarget) -> Self {
        self.focus_target = Some(target);
        self
    }

    pub fn clip(mut self) -> Self {
        self.clips = true;
        self
    }

    /// Set a procedural GPU background effect (noise gradient, linear gradient).
    /// This replaces the solid `bg()` color for the background pass.
    pub fn bg_effect(mut self, effect: BackgroundEffect) -> Self {
        self.bg_effect = Some(effect);
        self
    }

    /// Apply a frosted-glass Gaussian blur backdrop to this div.
    /// Everything rendered behind this div will be blurred within its bounds.
    /// Typical radius: 8–20 pixels.
    pub fn blur(mut self, radius: f32) -> Self {
        self.blur_radius = Some(radius);
        self
    }

    // -- Internal: resolve style with overrides --

    fn resolve_style(&self, hovered: bool) -> ElementStyle {
        let mut resolved = self.base_style.clone();
        if hovered {
            if let Some(ref ov) = self.hover_style {
                apply_override(&mut resolved, ov);
            }
        }
        resolved
    }
}

/// Div's prepaint state: an optional hitbox ID (registered when on_click is set).
pub struct DivPrepaintState {
    hitbox_id: Option<HitboxId>,
}

impl Element for Div {
    type LayoutState = Vec<LayoutId>;
    type PrepaintState = DivPrepaintState;

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        cx: &mut ElementContext,
    ) -> (LayoutId, Self::LayoutState) {
        // Layout children first, collecting their IDs.
        let child_ids: Vec<LayoutId> = self
            .children
            .iter_mut()
            .map(|child| child.request_layout(engine, cx))
            .collect();

        let id = engine.request_layout(self.base_style.layout.clone(), &child_ids);
        (id, child_ids)
    }

    fn prepaint(
        &mut self,
        bounds: Bounds,
        _layout_state: &mut Self::LayoutState,
        engine: &LayoutEngine,
        cx: &mut ElementContext,
    ) -> DivPrepaintState {
        let z = self.base_style.z_index;
        if z != 0 {
            cx.push_z_index(z);
        }

        let hitbox_id = if self.on_click.is_some() || self.hover_style.is_some() {
            Some(cx.insert_hitbox(bounds, HitboxBehavior::Normal))
        } else {
            None
        };

        if let Some(ref builder) = self.on_scroll {
            cx.scroll_regions.push(ScrollRegion {
                bounds,
                action_builder: builder.clone(),
            });
        }

        if self.scroll_y != 0.0 {
            for child in &mut self.children {
                child.prepaint_with_offset(engine, cx, 0.0, -self.scroll_y);
            }
        } else {
            for child in &mut self.children {
                child.prepaint(engine, cx);
            }
        }

        if z != 0 {
            cx.pop_z_index();
        }

        DivPrepaintState { hitbox_id }
    }

    fn paint(
        &mut self,
        bounds: Bounds,
        _layout_state: &mut Self::LayoutState,
        prepaint_state: &mut DivPrepaintState,
        engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
    ) {
        let hovered = prepaint_state
            .hitbox_id
            .map_or(false, |id| cx.is_hovered(id));
        let mut style = self.resolve_style(hovered);
        let r = style.corner_radius;
        let z = style.z_index;
        let opacity = style.opacity;

        if opacity < 1.0 {
            if let Some(ref mut bg) = style.background {
                bg.a = (bg.a as f32 * opacity) as u8;
            }
        }

        if z != 0 {
            scene.push_z_index(z);
        }

        if let Some(radius) = self.blur_radius {
            scene.blur_region(BlurRegionPrimitive {
                rect: bounds,
                blur_radius: radius,
                corner_radius: r,
            });
        }

        // Shadows
        for s in &style.shadows {
            scene.shadow(ShadowPrimitive {
                rect: bounds,
                blur_radius: s.blur_radius,
                corner_radius: s.corner_radius.max(r),
                offset: s.offset,
                color: s.color,
            });
        }

        // Background — effect quad takes priority over solid color.
        if let Some(effect) = self.bg_effect {
            let (effect_type, params, color_a, color_b) = match effect {
                BackgroundEffect::NoiseGradient {
                    scale,
                    color_a,
                    color_b,
                } => (EffectType::NoiseGradient, [scale, 0.0], color_a, color_b),
                BackgroundEffect::LinearGradient {
                    angle,
                    color_a,
                    color_b,
                } => (EffectType::LinearGradient, [angle, 0.0], color_a, color_b),
                BackgroundEffect::RadialGradient { color_a, color_b } => {
                    (EffectType::RadialGradient, [0.0, 0.0], color_a, color_b)
                }
                BackgroundEffect::Shimmer {
                    base,
                    highlight,
                    speed,
                } => (EffectType::Shimmer, [speed, 0.0], base, highlight),
                BackgroundEffect::Vignette { color, intensity } => (
                    EffectType::Vignette,
                    [intensity, 0.0],
                    color,
                    Color::TRANSPARENT,
                ),
                BackgroundEffect::ColorTint { color } => {
                    (EffectType::ColorTint, [0.0, 0.0], color, Color::TRANSPARENT)
                }
            };
            scene.effect_quad(EffectQuadPrimitive {
                rect: bounds,
                effect_type,
                color_a,
                color_b,
                params,
                corner_radius: r,
            });
        } else if let Some(bg) = style.background {
            scene.rounded_rect(RoundedRectPrimitive::uniform(bounds, r, bg));
        }

        // Border
        if let Some(border) = style.border_color {
            if style.border_widths != [0.0; 4] {
                scene.border(BorderPrimitive {
                    rect: bounds,
                    widths: style.border_widths,
                    corner_radii: [r; 4],
                    color: border,
                });
            }
        }

        if let Some(target) = self.focus_target {
            if cx.is_focused(target) {
                let ring_inset = -2.0;
                let ring_bounds = Rect {
                    x: bounds.x + ring_inset,
                    y: bounds.y + ring_inset,
                    width: bounds.width - ring_inset * 2.0,
                    height: bounds.height - ring_inset * 2.0,
                };
                scene.border(BorderPrimitive {
                    rect: ring_bounds,
                    widths: [2.0; 4],
                    corner_radii: [(r + 2.0); 4],
                    color: cx.theme.colors.focus_border,
                });
            }
        }

        // Register parent hit BEFORE children so that children's hit regions
        // (pushed later) are found first by the reverse search in handle_left_click.
        // This gives correct z-order: child clicks take priority over parent clicks.
        if let Some(action) = self.on_click.take() {
            cx.hits.push(HitRegion {
                rect: bounds,
                action,
                cursor: self.cursor,
            });
        }

        // Clip + scroll children
        if self.clips {
            scene.clip(bounds);
        }

        if self.scroll_y != 0.0 {
            for child in &mut self.children {
                child.paint_with_offset(engine, scene, cx, 0.0, -self.scroll_y);
            }
        } else {
            for child in &mut self.children {
                child.paint(engine, scene, cx);
            }
        }

        if self.scroll_total_height > bounds.height && !self.hide_scrollbar {
            let track_h = bounds.height;
            let content_h = self.scroll_total_height;
            let max_scroll = content_h - track_h;
            let thumb_h = (track_h / content_h * track_h).max(32.0).min(track_h);
            let thumb_y = if max_scroll > 0.0 {
                (self.scroll_y / max_scroll) * (track_h - thumb_h)
            } else {
                0.0
            };
            let thumb_w = 8.0;
            let track_pad = 3.0;
            let thumb_x = bounds.x + bounds.width - thumb_w - track_pad;

            // Track background (subtle)
            scene.rounded_rect(RoundedRectPrimitive::uniform(
                Rect {
                    x: thumb_x - 1.0,
                    y: bounds.y,
                    width: thumb_w + 2.0,
                    height: track_h,
                },
                5.0,
                Color::rgba(128, 128, 128, 10),
            ));

            // Thumb
            let thumb_inset = 1.0;
            scene.rounded_rect(RoundedRectPrimitive::uniform(
                Rect {
                    x: thumb_x + thumb_inset,
                    y: bounds.y + thumb_y + thumb_inset,
                    width: thumb_w - thumb_inset * 2.0,
                    height: thumb_h - thumb_inset * 2.0,
                },
                3.0,
                cx.theme.colors.scrollbar_thumb,
            ));

            // Register scrollbar track for click-to-scroll and drag.
            if let Some(ref builder) = self.on_scroll {
                let hit_w = thumb_w + 12.0; // wider hit area for easy grabbing
                let track_rect = Rect {
                    x: thumb_x - 6.0,
                    y: bounds.y,
                    width: hit_w,
                    height: track_h,
                };
                cx.scrollbar_tracks.push(ScrollbarTrack {
                    track_rect,
                    thumb_top: bounds.y + thumb_y,
                    thumb_height: thumb_h,
                    content_height: content_h,
                    viewport_height: track_h,
                    action_builder: builder.clone(),
                });
            }
        }

        if self.clips {
            scene.pop_clip();
        }

        // Debug wireframe: 1px outline around every div
        if cx.debug_wireframe {
            // Cycle colors by depth using bounds position as a hash
            let hash = ((bounds.x as u32).wrapping_mul(7) ^ (bounds.y as u32).wrapping_mul(13)) % 6;
            let wire_color = match hash {
                0 => Color::rgba(255, 80, 80, 120),  // red
                1 => Color::rgba(80, 255, 80, 120),  // green
                2 => Color::rgba(80, 80, 255, 120),  // blue
                3 => Color::rgba(255, 255, 80, 120), // yellow
                4 => Color::rgba(255, 80, 255, 120), // magenta
                _ => Color::rgba(80, 255, 255, 120), // cyan
            };
            scene.border(BorderPrimitive {
                rect: bounds,
                widths: [1.0; 4],
                corner_radii: [r; 4],
                color: wire_color,
            });
        }

        if z != 0 {
            scene.pop_z_index();
        }
    }
}

impl IntoAnyElement for Div {
    fn into_any(self) -> AnyElement {
        element_into_any(self)
    }
}

// ---------------------------------------------------------------------------
// Spacer — flexible empty space
// ---------------------------------------------------------------------------

pub struct Spacer;

pub fn spacer() -> Spacer {
    Spacer
}

impl Element for Spacer {
    type LayoutState = ();
    type PrepaintState = ();

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        _cx: &mut ElementContext,
    ) -> (LayoutId, ()) {
        let id = engine.request_layout(
            taffy::Style {
                flex_grow: 1.0,
                ..Default::default()
            },
            &[],
        );
        (id, ())
    }

    fn prepaint(
        &mut self,
        _bounds: Bounds,
        _layout_state: &mut (),
        _engine: &LayoutEngine,
        _cx: &mut ElementContext,
    ) -> () {
    }

    fn paint(
        &mut self,
        _bounds: Bounds,
        _layout_state: &mut (),
        _prepaint_state: &mut (),
        _engine: &LayoutEngine,
        _scene: &mut Scene,
        _cx: &mut ElementContext,
    ) {
    }
}

impl IntoAnyElement for Spacer {
    fn into_any(self) -> AnyElement {
        element_into_any(self)
    }
}

// ---------------------------------------------------------------------------
// TextElement — text with intrinsic sizing
// ---------------------------------------------------------------------------

use crate::render::{FontKind, TextPrimitive};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

pub struct TextElement {
    content: String,
    font_size: f32,
    line_height_factor: f32,
    color: Option<Color>,
    font_kind: FontKind,
    font_weight: FontWeight,
    align: TextAlign,
    truncate: bool,
}

pub fn text(content: impl Into<String>) -> TextElement {
    TextElement {
        content: content.into(),
        font_size: 0.0,
        line_height_factor: 1.5,
        color: None,
        font_kind: FontKind::Ui,
        font_weight: FontWeight::Normal,
        align: TextAlign::Left,
        truncate: false,
    }
}

impl TextElement {
    pub fn size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    pub fn text_sm(mut self) -> Self {
        self.font_size = -1.0; // sentinel: use ui_small_font_size
        self
    }

    pub fn text_xs(mut self) -> Self {
        self.font_size = -2.0; // sentinel: use caption size
        self
    }

    pub fn text_lg(mut self) -> Self {
        self.font_size = -3.0; // sentinel: use heading_font_size
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn mono(mut self) -> Self {
        self.font_kind = FontKind::Mono;
        self
    }

    pub fn line_height(mut self, factor: f32) -> Self {
        self.line_height_factor = factor;
        self
    }

    pub fn bold(mut self) -> Self {
        self.font_weight = FontWeight::Bold;
        self
    }

    pub fn semibold(mut self) -> Self {
        self.font_weight = FontWeight::Semibold;
        self
    }

    pub fn medium(mut self) -> Self {
        self.font_weight = FontWeight::Medium;
        self
    }

    pub fn text_center(mut self) -> Self {
        self.align = TextAlign::Center;
        self
    }

    pub fn text_right(mut self) -> Self {
        self.align = TextAlign::Right;
        self
    }

    pub fn truncate(mut self) -> Self {
        self.truncate = true;
        self
    }

    fn resolve_font_size(&self, theme: &Theme) -> f32 {
        match self.font_size.to_bits() {
            x if self.font_size > 0.0 => self.font_size,
            _ if self.font_size == -1.0 => theme.metrics.ui_small_font_size,
            _ if self.font_size == -2.0 => theme.metrics.ui_small_font_size - 1.0,
            _ if self.font_size == -3.0 => theme.metrics.heading_font_size,
            _ => theme.metrics.ui_font_size, // 0 or anything else
        }
    }
}

impl Element for TextElement {
    type LayoutState = (f32, f32, f32); // (resolved_font_size, line_height, natural_width)
    type PrepaintState = ();

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        cx: &mut ElementContext,
    ) -> (LayoutId, Self::LayoutState) {
        let font_size = self.resolve_font_size(cx.theme);
        let line_height = font_size * self.line_height_factor;

        let text_width = measure_text_width(
            cx.font_system,
            &self.content,
            font_size,
            self.font_kind,
            self.font_weight,
        );

        // Only allow shrinking when `.truncate()` is set; otherwise the text
        // holds its natural width so it isn't crushed next to flex_shrink:0 siblings
        // like SvgIcon.
        let shrink = if self.truncate { 1.0 } else { 0.0 };

        let id = engine.request_layout(
            taffy::Style {
                size: taffy::Size {
                    width: taffy::Dimension::length(text_width),
                    height: taffy::Dimension::length(line_height),
                },
                flex_shrink: shrink,
                ..Default::default()
            },
            &[],
        );
        (id, (font_size, line_height, text_width))
    }

    fn prepaint(
        &mut self,
        _bounds: Bounds,
        _layout_state: &mut Self::LayoutState,
        _engine: &LayoutEngine,
        _cx: &mut ElementContext,
    ) -> () {
    }

    fn paint(
        &mut self,
        bounds: Bounds,
        state: &mut (f32, f32, f32),
        _prepaint_state: &mut (),
        _engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
    ) {
        let (font_size, _line_height, natural_width) = *state;
        let color = self.color.unwrap_or(cx.theme.colors.text);

        let mut content = std::mem::take(&mut self.content);
        let mut text_width = natural_width;

        if self.truncate && bounds.width > 0.0 {
            let (truncated, truncated_width) = truncate_text_to_fit(
                cx.font_system,
                &content,
                font_size,
                self.font_kind,
                self.font_weight,
                bounds.width,
            );
            content = truncated;
            text_width = truncated_width;
        }

        if matches!(self.align, TextAlign::Center | TextAlign::Right) && !self.truncate {
            text_width = natural_width;
        }

        let x_offset = match self.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => ((bounds.width - text_width) * 0.5).max(0.0),
            TextAlign::Right => (bounds.width - text_width).max(0.0),
        };

        scene.text(TextPrimitive {
            rect: Rect {
                x: bounds.x + x_offset,
                ..bounds
            },
            text: content.clone().into(),
            color,
            font_size,
            font_kind: self.font_kind,
            font_weight: self.font_weight,
        });

        // Debug wireframe: red rect around text + log measurement vs bounds
        if cx.debug_wireframe {
            let measured = measure_text_width(
                cx.font_system,
                &content,
                font_size,
                self.font_kind,
                self.font_weight,
            );
            let crushed = bounds.width > 0.0 && bounds.width < measured * 0.9;
            let wire_color = if crushed {
                Color::rgba(255, 40, 40, 200) // red = text is crushed
            } else {
                Color::rgba(40, 200, 40, 120) // green = text fits
            };
            scene.border(BorderPrimitive {
                rect: bounds,
                widths: [1.0; 4],
                corner_radii: [0.0; 4],
                color: wire_color,
            });
            // Log short strings (likely button labels) to stderr
            if content.len() < 30 {
                eprintln!(
                    "[wireframe] text={:20} measured={:6.1} bounds_w={:6.1} scale={:.2} {}",
                    format!("{:?}", content),
                    measured,
                    bounds.width,
                    cx.scale_factor,
                    if crushed { "CRUSHED" } else { "ok" },
                );
            }
        }
    }
}

impl IntoAnyElement for TextElement {
    fn into_any(self) -> AnyElement {
        element_into_any(self)
    }
}

/// Allow `"string literal"` as a child element directly.
impl IntoAnyElement for &str {
    fn into_any(self) -> AnyElement {
        element_into_any(text(self))
    }
}

impl IntoAnyElement for String {
    fn into_any(self) -> AnyElement {
        element_into_any(text(self))
    }
}

// ---------------------------------------------------------------------------
// TextInput — text field with cursor, selection, and editing support
// ---------------------------------------------------------------------------

pub struct TextInput {
    label: String,
    value: String,
    placeholder: String,
    focused: bool,
    on_click: Option<Action>,
    base_style: ElementStyle,
    cursor: usize,
    anchor: usize,
    cursor_moved_at_ms: u64,
    focus_target: Option<crate::ui::state::FocusTarget>,
    bare: bool,
}

pub fn text_input(label: impl Into<String>, value: impl Into<String>) -> TextInput {
    TextInput {
        label: label.into(),
        value: value.into(),
        placeholder: String::new(),
        focused: false,
        on_click: None,
        base_style: ElementStyle::default(),
        cursor: 0,
        anchor: 0,
        cursor_moved_at_ms: 0,
        focus_target: None,
        bare: false,
    }
}

impl TextInput {
    pub fn placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = p.into();
        self
    }

    pub fn focused(mut self, f: bool) -> Self {
        self.focused = f;
        self
    }

    pub fn on_click(mut self, action: Action) -> Self {
        self.on_click = Some(action);
        self
    }

    pub fn cursor(mut self, offset: usize) -> Self {
        self.cursor = offset;
        self
    }

    pub fn anchor(mut self, offset: usize) -> Self {
        self.anchor = offset;
        self
    }

    pub fn cursor_moved_at(mut self, ms: u64) -> Self {
        self.cursor_moved_at_ms = ms;
        self
    }

    pub fn focus_target(mut self, target: crate::ui::state::FocusTarget) -> Self {
        self.focus_target = Some(target);
        self
    }

    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }
}

impl Styled for TextInput {
    fn element_style_mut(&mut self) -> &mut ElementStyle {
        &mut self.base_style
    }
}

/// Describes a text input hit area for click-to-position cursor placement.
#[derive(Debug, Clone)]
pub struct TextInputHitArea {
    pub bounds: Rect,
    pub text_x: f32,
    pub text_y: f32,
    pub text_width: f32,
    pub text_height: f32,
    pub value: String,
    pub font_size: f32,
    pub focus_target: crate::ui::state::FocusTarget,
}

impl Element for TextInput {
    type LayoutState = ();
    type PrepaintState = Option<HitboxId>;

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        _cx: &mut ElementContext,
    ) -> (LayoutId, ()) {
        let id = engine.request_layout(self.base_style.layout.clone(), &[]);
        (id, ())
    }

    fn prepaint(
        &mut self,
        bounds: Bounds,
        _layout_state: &mut (),
        _engine: &LayoutEngine,
        cx: &mut ElementContext,
    ) -> Option<HitboxId> {
        if self.on_click.is_some() || self.focus_target.is_some() {
            Some(cx.insert_hitbox(bounds, HitboxBehavior::Normal))
        } else {
            None
        }
    }

    fn paint(
        &mut self,
        bounds: Bounds,
        _layout_state: &mut (),
        _prepaint_state: &mut Option<HitboxId>,
        _engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
    ) {
        let theme = cx.theme;
        let radius = theme.metrics.control_radius;
        let value_size = if self.bare {
            theme.metrics.ui_small_font_size
        } else {
            theme.metrics.ui_font_size
        };
        let value_lh = value_size * 1.5;

        let (text_x, text_y, text_area_w);

        if self.bare {
            let pad = 0.0;
            text_x = bounds.x + pad;
            text_y = bounds.y + ((bounds.height - value_lh) * 0.5).max(0.0);
            text_area_w = (bounds.width - pad * 2.0).max(0.0);
        } else {
            let fill = if self.focused {
                theme.colors.surface
            } else {
                theme.colors.element_background
            };
            let border = if self.focused {
                theme.colors.focus_border
            } else {
                theme.colors.border
            };

            scene.rounded_rect(RoundedRectPrimitive::uniform(bounds, radius, fill));
            scene.border(BorderPrimitive::uniform(bounds, 1.0, radius, border));

            let scale = theme.metrics.ui_scale();
            let label_size = theme.metrics.ui_small_font_size - 1.0;
            let label_lh = label_size * 1.4;
            let pad = (Sz::INPUT_SIDE_PAD * scale).round();
            let top_pad = (Sz::INPUT_TOP_PAD * scale).round();

            scene.text(TextPrimitive {
                rect: Rect {
                    x: bounds.x + pad,
                    y: bounds.y + top_pad,
                    width: bounds.width - pad * 2.0,
                    height: label_lh,
                },
                text: std::mem::take(&mut self.label).into(),
                color: theme.colors.text_muted,
                font_size: label_size,
                font_kind: FontKind::Ui,
                font_weight: FontWeight::Medium,
            });

            text_x = bounds.x + pad;
            text_y = bounds.y + top_pad + label_lh + 2.0;
            text_area_w = bounds.width - pad * 2.0;
        }

        let is_placeholder = self.value.is_empty();
        let display = if is_placeholder {
            std::mem::take(&mut self.placeholder)
        } else {
            self.value.clone()
        };
        let text_color = if is_placeholder {
            theme.colors.text_muted.with_alpha(Alpha::PLACEHOLDER)
        } else {
            theme.colors.text
        };

        // Selection highlight (render before text so it appears behind)
        if self.focused && !is_placeholder {
            let sel_start = self.cursor.min(self.anchor);
            let sel_end = self.cursor.max(self.anchor);
            if sel_start != sel_end && sel_end <= self.value.len() {
                let x_start = measure_text_width(
                    cx.font_system,
                    &self.value[..sel_start],
                    value_size,
                    FontKind::Ui,
                    FontWeight::Normal,
                );
                let x_end = measure_text_width(
                    cx.font_system,
                    &self.value[..sel_end],
                    value_size,
                    FontKind::Ui,
                    FontWeight::Normal,
                );
                scene.rounded_rect(RoundedRectPrimitive::uniform(
                    Rect {
                        x: text_x + x_start,
                        y: text_y,
                        width: (x_end - x_start).min(text_area_w),
                        height: value_lh,
                    },
                    2.0,
                    theme.colors.accent.with_alpha(Alpha::SOFT),
                ));
            }
        }

        // Value text
        scene.text(TextPrimitive {
            rect: Rect {
                x: text_x,
                y: text_y,
                width: text_area_w,
                height: value_lh,
            },
            text: display.into(),
            color: text_color,
            font_size: value_size,
            font_kind: FontKind::Ui,
            font_weight: FontWeight::Normal,
        });

        // Cursor caret
        if self.focused && !is_placeholder || (self.focused && self.value.is_empty()) {
            let elapsed = cx.clock_ms.saturating_sub(self.cursor_moved_at_ms);
            let cursor_visible = elapsed < 530 || (elapsed / 530) % 2 == 0;
            if cursor_visible {
                let cursor_x = if self.cursor > 0 && !self.value.is_empty() {
                    let offset = self.cursor.min(self.value.len());
                    measure_text_width(
                        cx.font_system,
                        &self.value[..offset],
                        value_size,
                        FontKind::Ui,
                        FontWeight::Normal,
                    )
                } else {
                    0.0
                };
                scene.rounded_rect(RoundedRectPrimitive::uniform(
                    Rect {
                        x: text_x + cursor_x,
                        y: text_y + 1.0,
                        width: Sz::CURSOR_WIDTH,
                        height: value_lh - Sz::CURSOR_WIDTH,
                    },
                    1.0,
                    theme.colors.text,
                ));
            }
        }

        // Register click hit region
        if let Some(action) = self.on_click.take() {
            cx.hits.push(HitRegion {
                rect: bounds,
                action,
                cursor: CursorHint::Text,
            });
        }

        // Register hit area for click-to-position (stored in cx for app.rs to use)
        if let Some(target) = self.focus_target {
            let value = std::mem::take(&mut self.value);
            cx.text_input_hit_areas.push(TextInputHitArea {
                bounds,
                text_x,
                text_y,
                text_width: text_area_w,
                text_height: value_lh,
                value,
                font_size: value_size,
                focus_target: target,
            });
        }
    }
}

impl IntoAnyElement for TextInput {
    fn into_any(self) -> AnyElement {
        element_into_any(self)
    }
}

// ---------------------------------------------------------------------------
// Canvas — custom painting element
// ---------------------------------------------------------------------------

/// A leaf element that delegates painting to a caller-provided closure.
/// Participates in layout via its Taffy style.
pub struct Canvas {
    style: taffy::Style,
    paint_fn: Option<Box<dyn FnOnce(Bounds, &mut Scene, &mut ElementContext)>>,
}

/// Create a canvas element that calls `paint` with its resolved bounds.
pub fn canvas(paint: impl FnOnce(Bounds, &mut Scene, &mut ElementContext) + 'static) -> Canvas {
    Canvas {
        style: taffy::Style::default(),
        paint_fn: Some(Box::new(paint)),
    }
}

impl Canvas {
    pub fn w(mut self, v: f32) -> Self {
        self.style.size.width = taffy::Dimension::length(v);
        self
    }

    pub fn h(mut self, v: f32) -> Self {
        self.style.size.height = taffy::Dimension::length(v);
        self
    }

    pub fn flex_1(mut self) -> Self {
        self.style.flex_grow = 1.0;
        self.style.flex_shrink = 1.0;
        self.style.flex_basis = taffy::Dimension::percent(0.0);
        self
    }
}

impl Element for Canvas {
    type LayoutState = ();
    type PrepaintState = ();

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        _cx: &mut ElementContext,
    ) -> (LayoutId, ()) {
        let id = engine.request_layout(self.style.clone(), &[]);
        (id, ())
    }

    fn prepaint(
        &mut self,
        _bounds: Bounds,
        _layout_state: &mut (),
        _engine: &LayoutEngine,
        _cx: &mut ElementContext,
    ) -> () {
    }

    fn paint(
        &mut self,
        bounds: Bounds,
        _layout_state: &mut (),
        _prepaint_state: &mut (),
        _engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
    ) {
        if let Some(f) = self.paint_fn.take() {
            f(bounds, scene, cx);
        }
    }
}

impl IntoAnyElement for Canvas {
    fn into_any(self) -> AnyElement {
        element_into_any(self)
    }
}
// ---------------------------------------------------------------------------
// SvgIcon — renders an SVG string as a rasterized image
// ---------------------------------------------------------------------------

pub struct SvgIcon {
    svg: &'static str,
    size: f32,
    color: Option<Color>,
}

pub fn svg_icon(svg: &'static str, size: f32) -> SvgIcon {
    SvgIcon {
        svg,
        size,
        color: None,
    }
}

impl SvgIcon {
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }

    pub fn size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }
}

impl Element for SvgIcon {
    type LayoutState = ();
    type PrepaintState = ();

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        cx: &mut ElementContext,
    ) -> (LayoutId, ()) {
        let scale = cx.theme.metrics.ui_scale();
        let effective = self.size * scale;
        let id = engine.request_layout(
            taffy::Style {
                size: taffy::Size {
                    width: taffy::Dimension::length(effective),
                    height: taffy::Dimension::length(effective),
                },
                flex_shrink: 0.0,
                ..Default::default()
            },
            &[],
        );
        (id, ())
    }

    fn prepaint(
        &mut self,
        _bounds: Bounds,
        _layout_state: &mut (),
        _engine: &LayoutEngine,
        _cx: &mut ElementContext,
    ) -> () {
    }

    fn paint(
        &mut self,
        bounds: Bounds,
        _layout_state: &mut (),
        _prepaint_state: &mut (),
        _engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
    ) {
        let color = self.color.unwrap_or(cx.theme.colors.icon);
        let scale = cx.theme.metrics.ui_scale();
        let px_size = (self.size * scale).ceil() as u32;
        let key = crate::ui::icons::cache_key(self.svg, px_size, color);
        let (rgba, w, h) = crate::ui::icons::rasterize_svg(self.svg, px_size, color);
        scene.image(crate::render::ImagePrimitive {
            rect: bounds,
            width: w,
            height: h,
            rgba,
            cache_key: key,
        });
    }
}

impl IntoAnyElement for SvgIcon {
    fn into_any(self) -> AnyElement {
        element_into_any(self)
    }
}

// ---------------------------------------------------------------------------
// Text measurement
// ---------------------------------------------------------------------------

/// Measure text width using glyphon's real shaping — the same engine that
/// renders the text on the GPU.  This eliminates the mismatch between layout
/// estimates and actual rendered glyph widths.
pub(crate) fn measure_text_width(
    font_system: &mut glyphon::FontSystem,
    text: &str,
    font_size: f32,
    font_kind: FontKind,
    font_weight: FontWeight,
) -> f32 {
    if text.is_empty() {
        return 0.0;
    }

    let metrics = glyphon::Metrics::new(font_size, font_size * 1.2);
    let mut buffer = glyphon::Buffer::new(font_system, metrics);

    let family = match font_kind {
        FontKind::Ui => glyphon::Family::SansSerif,
        FontKind::Mono => glyphon::Family::Monospace,
    };
    let weight = match font_weight {
        FontWeight::Normal => glyphon::Weight::NORMAL,
        FontWeight::Medium => glyphon::Weight(500),
        FontWeight::Semibold => glyphon::Weight(600),
        FontWeight::Bold => glyphon::Weight::BOLD,
    };
    let attrs = glyphon::Attrs::new().family(family).weight(weight);

    // Set an unbounded width so glyphon shapes onto a single line.
    buffer.set_size(font_system, None, None);
    buffer.set_text(font_system, text, &attrs, glyphon::Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);

    // Use the line advance width rather than the visible ink bounds.
    // Bounding boxes like `glyph.x + glyph.w` can under-measure strings and
    // make glyphon clip button labels into garbled fragments in the live GPU
    // renderer.
    let width = buffer
        .layout_runs()
        .fold(0.0f32, |width, run| width.max(run.line_w));

    // Ceil to avoid sub-pixel truncation.
    width.ceil()
}

fn truncate_text_to_fit(
    font_system: &mut glyphon::FontSystem,
    text: &str,
    font_size: f32,
    font_kind: FontKind,
    font_weight: FontWeight,
    max_width: f32,
) -> (String, f32) {
    const ELLIPSIS: &str = "\u{2026}";

    if text.is_empty() || max_width <= 0.0 {
        return (String::new(), 0.0);
    }

    let full_width = measure_text_width(font_system, text, font_size, font_kind, font_weight);
    if full_width <= max_width {
        return (text.to_owned(), full_width);
    }

    let ellipsis_width =
        measure_text_width(font_system, ELLIPSIS, font_size, font_kind, font_weight);
    if ellipsis_width > max_width {
        return (String::new(), 0.0);
    }

    let mut boundaries = Vec::with_capacity(text.chars().count() + 1);
    boundaries.push(0);
    boundaries.extend(text.char_indices().skip(1).map(|(idx, _)| idx));
    boundaries.push(text.len());

    let char_count = boundaries.len() - 1;
    let mut low = 0usize;
    let mut high = char_count;
    let mut best_width = ellipsis_width;

    while low < high {
        let mid = (low + high + 1) / 2;
        let prefix = &text[..boundaries[mid]];
        let mut candidate = String::with_capacity(prefix.len() + ELLIPSIS.len());
        candidate.push_str(prefix);
        candidate.push_str(ELLIPSIS);
        let candidate_width =
            measure_text_width(font_system, &candidate, font_size, font_kind, font_weight);
        if candidate_width <= max_width {
            low = mid;
            best_width = candidate_width;
        } else {
            high = mid - 1;
        }
    }

    let prefix = &text[..boundaries[low]];
    let mut truncated = String::with_capacity(prefix.len() + ELLIPSIS.len());
    truncated.push_str(prefix);
    truncated.push_str(ELLIPSIS);
    let truncated_width = if low == 0 { ellipsis_width } else { best_width };
    (truncated, truncated_width)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;

    fn test_cx<'a>(
        font_system: &'a mut glyphon::FontSystem,
        store: &'a mut SignalStore,
    ) -> ElementContext<'a> {
        crate::fonts::configure_font_system(font_system);
        let theme = Box::leak(Box::new(Theme::default_dark()));
        ElementContext::new(theme, 1.0, font_system, None, store)
    }

    #[test]
    fn div_with_fixed_children_lays_out() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let mut root = div()
            .w(400.0)
            .h(300.0)
            .flex_row()
            .gap(10.0)
            .child(div().w(100.0).h_full())
            .child(div().flex_1().h_full())
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 400.0, 300.0);

        // If we got here without panicking, the layout engine worked.
        // The scene should have no primitives (no bg/border set).
        assert_eq!(scene.len(), 0);
    }

    #[test]
    fn div_with_background_emits_rounded_rect() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let mut root = div()
            .w(200.0)
            .h(100.0)
            .bg(Color::rgba(255, 0, 0, 255))
            .rounded(8.0)
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 200.0, 100.0);

        assert_eq!(scene.len(), 1); // one rounded rect
    }

    #[test]
    fn nested_divs_resolve_absolute_positions() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);

        let mut engine = LayoutEngine::new();
        let inner_w = 50.0;
        let padding = 20.0;

        // Outer: 200x100 with 20px padding, inner: 50x50
        let mut outer = div()
            .w(200.0)
            .h(100.0)
            .p(padding)
            .child(div().w(inner_w).h(inner_w));

        let (root_id, _) = outer.request_layout(&mut engine, &mut cx);
        engine.compute_layout(root_id, 200.0, 100.0);

        // The inner div should be offset by the padding.
        // Get child layout id — it's the first child of root.
        let inner_id = *engine.tree.children(root_id).unwrap().first().unwrap();
        let inner_bounds = engine.layout_bounds(inner_id);

        assert!(
            (inner_bounds.x - padding).abs() < 1.0,
            "inner x={} should be near padding={}",
            inner_bounds.x,
            padding
        );
        assert!(
            (inner_bounds.y - padding).abs() < 1.0,
            "inner y={} should be near padding={}",
            inner_bounds.y,
            padding
        );
        assert!((inner_bounds.width - inner_w).abs() < 1.0);
    }

    #[test]
    fn text_element_emits_text_primitive() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let mut root = div()
            .w(400.0)
            .h(50.0)
            .child(
                text("Hello world")
                    .size(14.0)
                    .color(Color::rgba(255, 255, 255, 255)),
            )
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 400.0, 50.0);

        // Should have exactly one text primitive.
        let text_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::TextRun(_)))
            .count();
        assert_eq!(text_count, 1);
    }

    #[test]
    fn string_as_child_works() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let mut root = div().w(300.0).h(40.0).child("bare string child").into_any();

        render_element(&mut root, &mut scene, &mut cx, 300.0, 40.0);

        let text_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::TextRun(_)))
            .count();
        assert_eq!(text_count, 1);
    }

    #[test]
    fn text_element_has_intrinsic_width() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);

        let mut engine = LayoutEngine::new();
        let mut txt = text("ABCDE").size(10.0);
        let (id, _) = txt.request_layout(&mut engine, &mut cx);
        engine.compute_layout(id, 999.0, 999.0);

        let bounds = engine.layout_bounds(id);
        // 5 chars * 10.0 * 0.55 = 27.5
        assert!(
            bounds.width > 20.0 && bounds.width < 40.0,
            "text width {} should be roughly 27.5",
            bounds.width
        );
        // line height = 10.0 * 1.5 = 15.0
        assert!(
            (bounds.height - 15.0).abs() < 1.0,
            "text height {} should be ~15.0",
            bounds.height
        );
    }

    #[test]
    fn on_click_registers_hit_region() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let mut root = div()
            .w(200.0)
            .h(50.0)
            .on_click(Action::OpenCompareSheet)
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 200.0, 50.0);

        assert_eq!(cx.hits.len(), 1);
        assert_eq!(cx.hits[0].action, Action::OpenCompareSheet);
        assert_eq!(cx.hits[0].cursor, CursorHint::Pointer);
        assert!(cx.hits[0].rect.width > 0.0);
    }

    #[test]
    fn hover_bg_applies_when_mouse_inside() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        cx.mouse_position = Some((100.0, 25.0)); // inside the 200x50 div

        let mut scene = Scene::default();
        let red = Color::rgba(255, 0, 0, 255);
        let blue = Color::rgba(0, 0, 255, 255);

        let mut root = div()
            .w(200.0)
            .h(50.0)
            .bg(red)
            .hover_bg(blue)
            .on_click(Action::Bootstrap)
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 200.0, 50.0);

        // Should have painted blue (hover) not red (default)
        let bg_prim = scene
            .primitives
            .iter()
            .find(|p| matches!(p, crate::render::Primitive::RoundedRect(_)));
        assert!(bg_prim.is_some());
        if let crate::render::Primitive::RoundedRect(rr) = bg_prim.unwrap() {
            assert_eq!(rr.color, blue, "hover bg should be blue");
        }
    }

    #[test]
    fn hover_bg_does_not_apply_when_mouse_outside() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        cx.mouse_position = Some((999.0, 999.0)); // outside

        let mut scene = Scene::default();
        let red = Color::rgba(255, 0, 0, 255);
        let blue = Color::rgba(0, 0, 255, 255);

        let mut root = div()
            .w(200.0)
            .h(50.0)
            .bg(red)
            .hover_bg(blue)
            .on_click(Action::Bootstrap)
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 200.0, 50.0);

        if let crate::render::Primitive::RoundedRect(rr) = &scene.primitives[0] {
            assert_eq!(rr.color, red, "should use normal bg when not hovered");
        }
    }

    #[test]
    fn realistic_title_bar_layout() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let theme = cx.theme;
        let mut root = div()
            .flex_row()
            .items_center()
            .w(1200.0)
            .h(52.0)
            .px(20.0)
            .bg(theme.colors.title_bar_background)
            .child(text("diffy").text_lg().color(theme.colors.text_strong))
            .child(spacer())
            .child(
                div()
                    .flex_row()
                    .gap(8.0)
                    .child(
                        div()
                            .px(14.0)
                            .py(6.0)
                            .rounded(7.0)
                            .bg(theme.colors.element_background)
                            .hover_bg(theme.colors.element_hover)
                            .on_click(Action::OpenCompareSheet)
                            .child(text("Compare").text_sm().color(theme.colors.text)),
                    )
                    .child(
                        div()
                            .px(14.0)
                            .py(6.0)
                            .rounded(7.0)
                            .hover_bg(theme.colors.ghost_element_hover)
                            .on_click(Action::OpenPullRequestModal)
                            .child(text("PR").text_sm().color(theme.colors.text_muted)),
                    ),
            )
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 1200.0, 52.0);

        // Should have: title bar bg + "Compare" button bg + 3 text primitives
        let rect_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::RoundedRect(_)))
            .count();
        assert!(
            rect_count >= 2,
            "should have title bar bg + button bg, got {}",
            rect_count
        );

        let text_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::TextRun(_)))
            .count();
        assert_eq!(text_count, 3, "should have 3 text labels");

        // Should have 2 hit regions (Compare + PR buttons)
        assert_eq!(cx.hits.len(), 2);
        assert_eq!(cx.hits[0].action, Action::OpenCompareSheet);
        assert_eq!(cx.hits[1].action, Action::OpenPullRequestModal);
    }

    #[test]
    fn measure_text_width_matches_layout_run_width() {
        let mut font_system = glyphon::FontSystem::new();
        let text = "Open Compare";
        let font_size = 12.0;

        let measured = measure_text_width(
            &mut font_system,
            text,
            font_size,
            FontKind::Ui,
            FontWeight::Normal,
        );

        let metrics = glyphon::Metrics::new(font_size, font_size * 1.2);
        let mut buffer = glyphon::Buffer::new(&mut font_system, metrics);
        let attrs = glyphon::Attrs::new().family(glyphon::Family::SansSerif);
        buffer.set_size(&mut font_system, None, None);
        buffer.set_text(
            &mut font_system,
            text,
            &attrs,
            glyphon::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);

        let line_width = buffer
            .layout_runs()
            .fold(0.0f32, |width, run| width.max(run.line_w))
            .ceil();

        assert!(
            (measured - line_width).abs() < 1.0,
            "measured width {measured} should track glyphon line width {line_width}",
        );
    }

    #[test]
    fn measure_text_width_accounts_for_font_weight() {
        let mut font_system = glyphon::FontSystem::new();
        crate::fonts::configure_font_system(&mut font_system);
        let text = "Open Compare";
        let font_size = 12.0;

        let measured = measure_text_width(
            &mut font_system,
            text,
            font_size,
            FontKind::Ui,
            FontWeight::Medium,
        );

        let metrics = glyphon::Metrics::new(font_size, font_size * 1.2);
        let mut buffer = glyphon::Buffer::new(&mut font_system, metrics);
        let attrs = glyphon::Attrs::new()
            .family(glyphon::Family::SansSerif)
            .weight(glyphon::Weight(500));
        buffer.set_size(&mut font_system, None, None);
        buffer.set_text(
            &mut font_system,
            text,
            &attrs,
            glyphon::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);

        let line_width = buffer
            .layout_runs()
            .fold(0.0f32, |width, run| width.max(run.line_w))
            .ceil();

        assert!(
            (measured - line_width).abs() < 1.0,
            "measured width {measured} should track medium-weight glyphon line width {line_width}",
        );
    }

    #[test]
    fn truncate_text_to_fit_accounts_for_font_weight() {
        let mut font_system = glyphon::FontSystem::new();
        crate::fonts::configure_font_system(&mut font_system);
        let text = "Open Compare With Repository";
        let font_size = 12.0;
        let max_width = measure_text_width(
            &mut font_system,
            "Open Compare",
            font_size,
            FontKind::Ui,
            FontWeight::Medium,
        );

        let (truncated, truncated_width) = truncate_text_to_fit(
            &mut font_system,
            text,
            font_size,
            FontKind::Ui,
            FontWeight::Medium,
            max_width,
        );

        assert_ne!(
            truncated, text,
            "text should truncate when width is constrained"
        );
        assert!(
            truncated.ends_with('\u{2026}'),
            "truncated text should end with an ellipsis: {truncated:?}",
        );
        assert!(
            truncated_width <= max_width + 1.0,
            "truncated width {truncated_width} should fit max width {max_width}",
        );
    }

    #[test]
    fn truncated_text_primitive_fits_bounds_for_medium_weight() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let mut root = div()
            .w(110.0)
            .h(28.0)
            .flex_row()
            .child(
                text("Open Compare With Repository")
                    .text_sm()
                    .medium()
                    .truncate(),
            )
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 110.0, 28.0);

        let text = scene
            .primitives
            .iter()
            .find_map(|primitive| match primitive {
                crate::render::Primitive::TextRun(text) => Some(text.clone()),
                _ => None,
            })
            .expect("expected a text primitive");

        assert!(
            text.text.ends_with('\u{2026}'),
            "rendered text should be truncated: {:?}",
            text.text,
        );

        let measured = measure_text_width(
            cx.font_system,
            &text.text,
            text.font_size,
            text.font_kind,
            text.font_weight,
        );
        assert!(
            measured <= text.rect.width + 1.0,
            "measured width {measured} should fit rendered bounds {} for {:?}",
            text.rect.width,
            text.text,
        );
    }

    #[test]
    fn realistic_file_list_with_scroll() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let theme = cx.theme;
        let files = vec!["src/main.rs", "src/lib.rs", "Cargo.toml", "README.md"];

        let mut root = div()
            .flex_col()
            .w(260.0)
            .h(400.0)
            .bg(theme.colors.sidebar_background)
            .child(
                div().px(12.0).py(12.0).child(
                    text(format!("Files  ·  {}", files.len()))
                        .text_sm()
                        .color(theme.colors.text_muted),
                ),
            )
            .child(div().flex_1().flex_col().scroll_y(0.0).children_from(
                files.iter().enumerate().map(|(i, path)| {
                    div()
                        .w_full()
                        .h(36.0)
                        .px(12.0)
                        .items_center()
                        .flex_row()
                        .rounded(7.0)
                        .hover_bg(theme.colors.sidebar_row_hover)
                        .on_click(Action::SelectFile(i))
                        .child(text(*path).text_sm().color(theme.colors.text))
                        .into_any()
                }),
            ))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 260.0, 400.0);

        // 4 file items should generate 4 hit regions
        assert_eq!(cx.hits.len(), 4);
        assert_eq!(cx.hits[2].action, Action::SelectFile(2));

        // Should have text for header + 4 files = 5 text primitives
        let text_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::TextRun(_)))
            .count();
        assert_eq!(text_count, 5);
    }

    #[test]
    fn scroll_y_offsets_nested_descendant_text() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);

        let build_scene = |scroll_y: f32, cx: &mut ElementContext<'_>| {
            let mut scene = Scene::default();
            let mut root = div()
                .w(220.0)
                .h(80.0)
                .scroll_y(scroll_y)
                .child(
                    div().w_full().h(36.0).child(
                        div()
                            .px(12.0)
                            .py(8.0)
                            .child(text("nested file row").text_sm()),
                    ),
                )
                .into_any();

            render_element(&mut root, &mut scene, cx, 220.0, 80.0);

            scene
                .primitives
                .iter()
                .find_map(|primitive| match primitive {
                    crate::render::Primitive::TextRun(text) => Some(text.rect.y),
                    _ => None,
                })
                .expect("expected nested text primitive")
        };

        let unscrolled_y = build_scene(0.0, &mut cx);
        let scrolled_y = build_scene(20.0, &mut cx);

        assert!(
            (scrolled_y - (unscrolled_y - 20.0)).abs() < 1.0,
            "nested text did not move with scroll: unscrolled_y={unscrolled_y}, scrolled_y={scrolled_y}"
        );
    }

    #[test]
    fn scroll_y_clips_and_offsets_children() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let red = Color::rgba(255, 0, 0, 255);

        // Container 100px tall, child 50px tall, scrolled down 20px.
        // Child should paint at y = -20 (shifted up), and be clipped.
        let mut root = div()
            .w(200.0)
            .h(100.0)
            .scroll_y(20.0)
            .child(div().w(200.0).h(50.0).bg(red))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 200.0, 100.0);

        // Should have: ClipStart, RoundedRect (child bg), ClipEnd
        let clip_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::ClipStart(_)))
            .count();
        assert_eq!(clip_count, 1, "scroll container should clip");

        // The child's bg rect should be offset by -20 in y
        let bg = scene
            .primitives
            .iter()
            .find_map(|p| {
                if let crate::render::Primitive::RoundedRect(rr) = p {
                    Some(rr)
                } else {
                    None
                }
            })
            .expect("should have child bg");
        assert!(
            (bg.rect.y - (-20.0)).abs() < 1.0,
            "child y={} should be ~-20 (scrolled)",
            bg.rect.y
        );
    }

    // -- New tests --

    #[test]
    fn canvas_element_emits_custom_primitives() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let green = Color::rgba(0, 255, 0, 255);

        let mut root = div()
            .w(400.0)
            .h(300.0)
            .child(
                canvas(move |bounds, scene, _cx| {
                    // Draw a custom rect using the resolved bounds.
                    scene.rounded_rect(RoundedRectPrimitive::uniform(bounds, 0.0, green));
                })
                .w(100.0)
                .h(50.0),
            )
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 400.0, 300.0);

        // The canvas closure should have emitted exactly one rounded rect.
        let rr_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::RoundedRect(_)))
            .count();
        assert_eq!(rr_count, 1, "canvas should emit one rounded rect");

        if let crate::render::Primitive::RoundedRect(rr) = &scene.primitives[0] {
            assert_eq!(rr.color, green, "canvas rect should be green");
            assert!(
                (rr.rect.width - 100.0).abs() < 1.0,
                "canvas width should be ~100"
            );
            assert!(
                (rr.rect.height - 50.0).abs() < 1.0,
                "canvas height should be ~50"
            );
        } else {
            panic!("expected RoundedRect primitive from canvas");
        }
    }

    #[test]
    fn hitbox_blocking_modal_prevents_hover_behind() {
        // Scenario: a background div and a modal div that blocks mouse events.
        // The mouse is at a position inside both. The background div should NOT
        // be hovered because the modal's BlockMouse hitbox blocks it.
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let theme = Box::leak(Box::new(Theme::default_dark()));
        let mut cx = ElementContext::new(
            theme,
            1.0,
            &mut font_system,
            Some((100.0, 100.0)),
            &mut store,
        );

        // Register a "background" hitbox at (0,0)-(200,200).
        let bg_id = cx.insert_hitbox(
            Bounds {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 200.0,
            },
            HitboxBehavior::Normal,
        );

        // Register a "modal" hitbox at (50,50)-(150,150) that blocks mouse.
        let modal_id = cx.insert_hitbox(
            Bounds {
                x: 50.0,
                y: 50.0,
                width: 100.0,
                height: 100.0,
            },
            HitboxBehavior::BlockMouse,
        );

        cx.run_hit_test();

        // The modal should be hovered (mouse at 100,100 is inside it).
        assert!(cx.is_hovered(modal_id), "modal should be hovered");
        // The background should NOT be hovered because the modal blocks it.
        assert!(
            !cx.is_hovered(bg_id),
            "background should be blocked by modal"
        );
    }

    #[test]
    fn render_once_component_renders_correctly() {
        // Define a simple component that produces a div with text.
        struct MyButton {
            label: String,
            color: Color,
        }

        impl RenderOnce for MyButton {
            fn render(self, _cx: &ElementContext) -> AnyElement {
                div()
                    .w(120.0)
                    .h(40.0)
                    .bg(self.color)
                    .child(text(self.label).size(14.0))
                    .into_any()
            }
        }

        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let blue = Color::rgba(0, 0, 255, 255);

        let button = MyButton {
            label: "Click me".into(),
            color: blue,
        };

        // Use the RenderOnce component as a child via IntoAnyElement.
        let mut root = div().w(400.0).h(200.0).child(button.into_any()).into_any();

        render_element(&mut root, &mut scene, &mut cx, 400.0, 200.0);

        // Should have the button's background rect and text.
        let rr_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::RoundedRect(_)))
            .count();
        assert_eq!(rr_count, 1, "button should emit one background rect");

        let text_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::TextRun(_)))
            .count();
        assert_eq!(text_count, 1, "button should emit one text primitive");

        if let crate::render::Primitive::RoundedRect(rr) = &scene.primitives[0] {
            assert_eq!(rr.color, blue, "button bg should be blue");
        }
    }

    #[test]
    fn hover_style_override_changes_border() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = ElementContext::new(
            Box::leak(Box::new(Theme::default_dark())),
            1.0,
            &mut font_system,
            Some((100.0, 25.0)), // inside
            &mut store,
        );
        let mut scene = Scene::default();

        let red = Color::rgba(255, 0, 0, 255);
        let blue = Color::rgba(0, 0, 255, 255);
        let green = Color::rgba(0, 255, 0, 255);

        let mut root = div()
            .w(200.0)
            .h(50.0)
            .bg(red)
            .border_b(blue)
            .hover(|s| s.bg(green).border_color(green))
            .on_click(Action::Bootstrap)
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 200.0, 50.0);

        // Should use green bg and green border (hover override)
        let bg = scene
            .primitives
            .iter()
            .find_map(|p| {
                if let crate::render::Primitive::RoundedRect(rr) = p {
                    Some(rr)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(bg.color, green, "hover should override bg to green");

        let border = scene
            .primitives
            .iter()
            .find_map(|p| {
                if let crate::render::Primitive::Border(b) = p {
                    Some(b)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(border.color, green, "hover should override border to green");
    }

    #[test]
    fn when_conditional_applies() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let red = Color::rgba(255, 0, 0, 255);
        let blue = Color::rgba(0, 0, 255, 255);

        // .when(true, ...) should apply
        let mut root = div()
            .w(100.0)
            .h(50.0)
            .bg(red)
            .when(true, |d| d.bg(blue))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 100.0, 50.0);

        if let crate::render::Primitive::RoundedRect(rr) = &scene.primitives[0] {
            assert_eq!(rr.color, blue, "when(true) should apply bg override");
        }
    }

    #[test]
    fn on_scroll_registers_scroll_region() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let mut root = div()
            .w(260.0)
            .h(400.0)
            .scroll_y(0.0)
            .on_scroll(ScrollActionBuilder::FileList)
            .child(div().w_full().h(1000.0))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 260.0, 400.0);

        assert_eq!(cx.scroll_regions.len(), 1);
        let action = cx.scroll_regions[0].action_builder.build(3);
        assert_eq!(action, Action::ScrollFileList(3));
    }

    #[test]
    fn focus_tracking_query() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let cx = ElementContext::new(
            Box::leak(Box::new(Theme::default_dark())),
            1.0,
            &mut font_system,
            None,
            &mut store,
        )
        .with_focus(Some(crate::ui::state::FocusTarget::FileList));

        assert!(cx.is_focused(crate::ui::state::FocusTarget::FileList));
        assert!(!cx.is_focused(crate::ui::state::FocusTarget::Editor));
    }

    #[test]
    fn text_input_renders_label_and_value() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let mut root = text_input("Branch", "main")
            .w(200.0)
            .h(56.0)
            .on_click(Action::OpenRefPicker(crate::ui::state::CompareField::Left))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 200.0, 56.0);

        // Should have: bg rect + border + 2 text primitives (label + value)
        let text_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::TextRun(_)))
            .count();
        assert_eq!(text_count, 2, "should have label + value text");

        // Should have a hit region
        assert_eq!(cx.hits.len(), 1);
        assert_eq!(cx.hits[0].cursor, CursorHint::Text);
    }

    #[test]
    fn when_conditional_skips() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let red = Color::rgba(255, 0, 0, 255);
        let blue = Color::rgba(0, 0, 255, 255);

        // .when(false, ...) should NOT apply
        let mut root = div()
            .w(100.0)
            .h(50.0)
            .bg(red)
            .when(false, |d| d.bg(blue))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 100.0, 50.0);

        if let crate::render::Primitive::RoundedRect(rr) = &scene.primitives[0] {
            assert_eq!(rr.color, red, "when(false) should keep original bg");
        }
    }

    #[test]
    fn bg_effect_noise_gradient_emits_effect_quad() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let a = Color::rgba(255, 0, 0, 255);
        let b = Color::rgba(0, 0, 255, 255);

        let mut root = div()
            .w(300.0)
            .h(200.0)
            .rounded(10.0)
            .bg_effect(noise_gradient(0.02, a, b))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 300.0, 200.0);

        let effect_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::EffectQuad(_)))
            .count();
        assert_eq!(effect_count, 1, "should emit one effect quad");

        // Should NOT emit a RoundedRect bg (effect replaces it).
        let rr_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::RoundedRect(_)))
            .count();
        assert_eq!(rr_count, 0, "effect should replace solid bg");

        if let crate::render::Primitive::EffectQuad(eq) = &scene.primitives[0] {
            assert_eq!(eq.effect_type, crate::render::EffectType::NoiseGradient);
            assert_eq!(eq.color_a, a);
            assert_eq!(eq.color_b, b);
            assert!((eq.params[0] - 0.02).abs() < 0.001);
            assert!((eq.corner_radius - 10.0).abs() < 0.1);
        } else {
            panic!("expected EffectQuad primitive");
        }
    }

    #[test]
    fn bg_effect_linear_gradient_emits_effect_quad() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let a = Color::rgba(0, 255, 0, 255);
        let b = Color::rgba(255, 255, 0, 255);
        let angle = std::f32::consts::FRAC_PI_2;

        let mut root = div()
            .w(200.0)
            .h(100.0)
            .bg_effect(linear_gradient(angle, a, b))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 200.0, 100.0);

        let effect_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::EffectQuad(_)))
            .count();
        assert_eq!(effect_count, 1);

        if let crate::render::Primitive::EffectQuad(eq) = &scene.primitives[0] {
            assert_eq!(eq.effect_type, crate::render::EffectType::LinearGradient);
            assert!((eq.params[0] - angle).abs() < 0.001);
        } else {
            panic!("expected EffectQuad primitive");
        }
    }

    #[test]
    fn bg_effect_replaces_solid_bg() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let red = Color::rgba(255, 0, 0, 255);
        let blue = Color::rgba(0, 0, 255, 255);

        // Setting both bg() and bg_effect() — effect should win.
        let mut root = div()
            .w(100.0)
            .h(100.0)
            .bg(red)
            .bg_effect(linear_gradient(0.0, red, blue))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 100.0, 100.0);

        let effect_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::EffectQuad(_)))
            .count();
        let rr_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::RoundedRect(_)))
            .count();

        assert_eq!(effect_count, 1, "effect should be emitted");
        assert_eq!(
            rr_count, 0,
            "solid bg should not be emitted when effect is set"
        );
    }

    #[test]
    fn blur_emits_blur_region_primitive() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let red = Color::rgba(255, 0, 0, 255);

        let mut root = div()
            .w(400.0)
            .h(300.0)
            .blur(12.0)
            .bg(red)
            .rounded(14.0)
            .child(text("Frosted glass"))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 400.0, 300.0);

        // Should have a BlurRegion primitive before the background.
        let blur_count = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, crate::render::Primitive::BlurRegion(_)))
            .count();
        assert_eq!(blur_count, 1, "should emit one blur region");

        // The BlurRegion should come before the RoundedRect (background).
        let blur_idx = scene
            .primitives
            .iter()
            .position(|p| matches!(p, crate::render::Primitive::BlurRegion(_)))
            .unwrap();
        let bg_idx = scene
            .primitives
            .iter()
            .position(|p| matches!(p, crate::render::Primitive::RoundedRect(_)))
            .unwrap();
        assert!(blur_idx < bg_idx, "blur should precede background");

        if let crate::render::Primitive::BlurRegion(br) = &scene.primitives[blur_idx] {
            assert!((br.blur_radius - 12.0).abs() < 0.1);
            assert!((br.corner_radius - 14.0).abs() < 0.1);
            assert!((br.rect.width - 400.0).abs() < 1.0);
        } else {
            panic!("expected BlurRegion");
        }
    }

    #[test]
    fn radial_gradient_emits_correct_effect_type() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let a = Color::rgba(255, 255, 255, 255);
        let b = Color::rgba(0, 0, 0, 255);

        let mut root = div()
            .w(200.0)
            .h(200.0)
            .bg_effect(radial_gradient(a, b))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 200.0, 200.0);

        if let crate::render::Primitive::EffectQuad(eq) = &scene.primitives[0] {
            assert_eq!(eq.effect_type, crate::render::EffectType::RadialGradient);
        } else {
            panic!("expected EffectQuad");
        }
    }

    #[test]
    fn shimmer_emits_correct_effect_type() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let base = Color::rgba(40, 40, 40, 255);
        let highlight = Color::rgba(60, 60, 60, 255);

        let mut root = div()
            .w(300.0)
            .h(20.0)
            .bg_effect(shimmer(base, highlight, 2.0))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 300.0, 20.0);

        if let crate::render::Primitive::EffectQuad(eq) = &scene.primitives[0] {
            assert_eq!(eq.effect_type, crate::render::EffectType::Shimmer);
            assert!((eq.params[0] - 2.0).abs() < 0.01, "speed should be 2.0");
        } else {
            panic!("expected EffectQuad");
        }
    }

    #[test]
    fn vignette_emits_correct_effect_type() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let dark = Color::rgba(0, 0, 0, 128);

        let mut root = div()
            .w(800.0)
            .h(600.0)
            .bg_effect(vignette(dark, 0.5))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 800.0, 600.0);

        if let crate::render::Primitive::EffectQuad(eq) = &scene.primitives[0] {
            assert_eq!(eq.effect_type, crate::render::EffectType::Vignette);
            assert!((eq.params[0] - 0.5).abs() < 0.01, "intensity should be 0.5");
        } else {
            panic!("expected EffectQuad");
        }
    }

    #[test]
    fn color_tint_emits_correct_effect_type() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let tint = Color::rgba(0, 100, 255, 80);

        let mut root = div()
            .w(400.0)
            .h(300.0)
            .bg_effect(color_tint(tint))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 400.0, 300.0);

        if let crate::render::Primitive::EffectQuad(eq) = &scene.primitives[0] {
            assert_eq!(eq.effect_type, crate::render::EffectType::ColorTint);
            assert_eq!(eq.color_a, tint);
        } else {
            panic!("expected EffectQuad");
        }
    }

    #[test]
    fn glow_adds_shadow_with_zero_offset() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let accent = Color::rgba(0, 128, 255, 200);

        let mut root = div()
            .w(100.0)
            .h(40.0)
            .rounded(8.0)
            .bg(Color::rgba(30, 30, 30, 255))
            .glow(accent, 10.0)
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 100.0, 40.0);

        // Glow should produce a ShadowPrimitive with offset [0, 0].
        let shadow = scene.primitives.iter().find_map(|p| {
            if let crate::render::Primitive::Shadow(s) = p {
                Some(s)
            } else {
                None
            }
        });
        assert!(shadow.is_some(), "glow should produce a shadow");
        let s = shadow.unwrap();
        assert_eq!(s.color, accent);
        assert!((s.offset[0]).abs() < 0.01, "glow x offset should be 0");
        assert!((s.offset[1]).abs() < 0.01, "glow y offset should be 0");
        assert!(
            (s.blur_radius - 10.0).abs() < 0.1,
            "blur radius should be 10"
        );
    }

    #[test]
    fn z_index_emits_push_pop_primitives() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let red = Color::rgba(255, 0, 0, 255);

        let mut root = div().w(200.0).h(100.0).z_index(10).bg(red).into_any();

        render_element(&mut root, &mut scene, &mut cx, 200.0, 100.0);

        let has_push = scene
            .primitives
            .iter()
            .any(|p| matches!(p, crate::render::Primitive::ZIndexPush(10)));
        let has_pop = scene
            .primitives
            .iter()
            .any(|p| matches!(p, crate::render::Primitive::ZIndexPop));
        assert!(has_push, "z_index(10) should emit ZIndexPush(10)");
        assert!(has_pop, "z_index(10) should emit ZIndexPop");
    }

    #[test]
    fn z_index_zero_emits_no_push_pop() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let mut cx = test_cx(&mut font_system, &mut store);
        let mut scene = Scene::default();

        let mut root = div()
            .w(200.0)
            .h(100.0)
            .bg(Color::rgba(255, 0, 0, 255))
            .into_any();

        render_element(&mut root, &mut scene, &mut cx, 200.0, 100.0);

        let has_push = scene
            .primitives
            .iter()
            .any(|p| matches!(p, crate::render::Primitive::ZIndexPush(_)));
        assert!(!has_push, "z_index 0 should not emit ZIndexPush");
    }

    #[test]
    fn z_index_hitbox_priority() {
        let mut font_system = glyphon::FontSystem::new();
        let mut store = SignalStore::new();
        let theme = Box::leak(Box::new(Theme::default_dark()));
        let mut cx =
            ElementContext::new(theme, 1.0, &mut font_system, Some((50.0, 50.0)), &mut store);

        // Register a z=0 hitbox covering (0,0)-(100,100).
        cx.push_z_index(0);
        let low_id = cx.insert_hitbox(
            Bounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            HitboxBehavior::Normal,
        );
        cx.pop_z_index();

        // Register a z=10 BlockMouse hitbox covering (0,0)-(100,100).
        cx.push_z_index(10);
        let high_id = cx.insert_hitbox(
            Bounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            HitboxBehavior::BlockMouse,
        );
        cx.pop_z_index();

        cx.run_hit_test();

        assert!(cx.is_hovered(high_id), "z=10 should be hovered");
        assert!(!cx.is_hovered(low_id), "z=0 should be blocked by z=10");
    }
}
