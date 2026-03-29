//! Core element model for declarative UI layout.
//!
//! Elements describe what they want (size, flex, padding) and a layout engine
//! (Taffy) resolves concrete pixel coordinates. The renderer then paints.

use crate::render::scene::Rect;
use crate::render::Scene;
use crate::ui::theme::Theme;

pub use taffy::NodeId as LayoutId;

// ---------------------------------------------------------------------------
// Bounds — the resolved rectangle for a laid-out element
// ---------------------------------------------------------------------------

pub type Bounds = Rect;

// ---------------------------------------------------------------------------
// ElementContext — shared state available during layout and paint
// ---------------------------------------------------------------------------

pub struct ElementContext<'a> {
    pub theme: &'a Theme,
    pub scale_factor: f32,
    pub font_system: &'a mut glyphon::FontSystem,
}

// ---------------------------------------------------------------------------
// Element trait
// ---------------------------------------------------------------------------

/// Every UI node implements `Element`. The lifecycle is:
///
/// 1. **request_layout** — declare your Taffy style and children. Returns a
///    `LayoutId` and arbitrary per-element state.
/// 2. **paint** — given your resolved bounds, emit scene primitives.
pub trait Element: 'static {
    type LayoutState: 'static;

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        cx: &mut ElementContext,
    ) -> (LayoutId, Self::LayoutState);

    fn paint(
        &mut self,
        bounds: Bounds,
        state: &mut Self::LayoutState,
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

    pub fn paint(
        &mut self,
        engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
    ) {
        self.inner.paint(engine, scene, cx);
    }
}

trait AnyElementImpl {
    fn request_layout(&mut self, engine: &mut LayoutEngine, cx: &mut ElementContext) -> LayoutId;
    fn paint(&mut self, engine: &LayoutEngine, scene: &mut Scene, cx: &mut ElementContext);
}

struct ElementHolder<E: Element> {
    element: E,
    layout_state: Option<E::LayoutState>,
    layout_id: Option<LayoutId>,
}

impl<E: Element> AnyElementImpl for ElementHolder<E> {
    fn request_layout(&mut self, engine: &mut LayoutEngine, cx: &mut ElementContext) -> LayoutId {
        let (id, state) = self.element.request_layout(engine, cx);
        self.layout_id = Some(id);
        self.layout_state = Some(state);
        id
    }

    fn paint(&mut self, engine: &LayoutEngine, scene: &mut Scene, cx: &mut ElementContext) {
        let id = self.layout_id.expect("paint called before request_layout");
        let bounds = engine.layout_bounds(id);
        let state = self.layout_state.as_mut().expect("paint called before request_layout");
        self.element.paint(bounds, state, engine, scene, cx);
    }
}

// ---------------------------------------------------------------------------
// IntoAnyElement — conversion trait
// ---------------------------------------------------------------------------

pub trait IntoAnyElement {
    fn into_any(self) -> AnyElement;
}

impl<E: Element> IntoAnyElement for E {
    fn into_any(self) -> AnyElement {
        AnyElement::new(self)
    }
}

// ---------------------------------------------------------------------------
// MeasureFunc — stored per-node for intrinsic sizing (text)
// ---------------------------------------------------------------------------

type MeasureFn = Box<
    dyn Fn(
        taffy::Size<Option<f32>>,
        taffy::Size<taffy::AvailableSpace>,
    ) -> taffy::Size<f32>
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
    pub fn request_layout(
        &mut self,
        style: taffy::Style,
        children: &[LayoutId],
    ) -> LayoutId {
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
        measure: impl Fn(taffy::Size<Option<f32>>, taffy::Size<taffy::AvailableSpace>) -> taffy::Size<f32>
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

/// Lay out and paint an element tree into the given scene.
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
    root.paint(&engine, scene, cx);
}

// ---------------------------------------------------------------------------
// Div — the fundamental container element
// ---------------------------------------------------------------------------

use crate::render::{
    BorderPrimitive, RoundedRectPrimitive, ShadowPrimitive,
};
use crate::ui::theme::Color;

/// A flexbox container. The core building block.
pub struct Div {
    style: taffy::Style,
    children: Vec<AnyElement>,
    background: Option<Color>,
    border_color: Option<Color>,
    border_width: f32,
    corner_radius: f32,
    shadows: Vec<ShadowSpec>,
}

struct ShadowSpec {
    blur_radius: f32,
    offset: [f32; 2],
    corner_radius: f32,
    color: Color,
}

/// Create a new div element.
pub fn div() -> Div {
    Div {
        style: taffy::Style {
            display: taffy::Display::Flex,
            ..Default::default()
        },
        children: Vec::new(),
        background: None,
        border_color: None,
        border_width: 0.0,
        corner_radius: 0.0,
        shadows: Vec::new(),
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

    // -- Layout --

    pub fn flex_row(mut self) -> Self {
        self.style.flex_direction = taffy::FlexDirection::Row;
        self
    }

    pub fn flex_col(mut self) -> Self {
        self.style.flex_direction = taffy::FlexDirection::Column;
        self
    }

    pub fn flex_1(mut self) -> Self {
        self.style.flex_grow = 1.0;
        self.style.flex_shrink = 1.0;
        self.style.flex_basis = taffy::Dimension::percent(0.0);
        self
    }

    pub fn flex_grow(mut self) -> Self {
        self.style.flex_grow = 1.0;
        self
    }

    pub fn flex_shrink_0(mut self) -> Self {
        self.style.flex_shrink = 0.0;
        self
    }

    pub fn gap(mut self, v: f32) -> Self {
        self.style.gap = taffy::Size {
            width: taffy::LengthPercentage::length(v),
            height: taffy::LengthPercentage::length(v),
        };
        self
    }

    pub fn gap_x(mut self, v: f32) -> Self {
        self.style.gap.width = taffy::LengthPercentage::length(v);
        self
    }

    pub fn gap_y(mut self, v: f32) -> Self {
        self.style.gap.height = taffy::LengthPercentage::length(v);
        self
    }

    pub fn p(mut self, v: f32) -> Self {
        let l = taffy::LengthPercentage::length(v);
        self.style.padding = taffy::Rect { left: l, right: l, top: l, bottom: l };
        self
    }

    pub fn px(mut self, v: f32) -> Self {
        let l = taffy::LengthPercentage::length(v);
        self.style.padding.left = l;
        self.style.padding.right = l;
        self
    }

    pub fn py(mut self, v: f32) -> Self {
        let l = taffy::LengthPercentage::length(v);
        self.style.padding.top = l;
        self.style.padding.bottom = l;
        self
    }

    pub fn w(mut self, v: f32) -> Self {
        self.style.size.width = taffy::Dimension::length(v);
        self
    }

    pub fn h(mut self, v: f32) -> Self {
        self.style.size.height = taffy::Dimension::length(v);
        self
    }

    pub fn w_full(mut self) -> Self {
        self.style.size.width = taffy::Dimension::percent(1.0);
        self
    }

    pub fn h_full(mut self) -> Self {
        self.style.size.height = taffy::Dimension::percent(1.0);
        self
    }

    pub fn min_w(mut self, v: f32) -> Self {
        self.style.min_size.width = taffy::Dimension::length(v);
        self
    }

    pub fn min_h(mut self, v: f32) -> Self {
        self.style.min_size.height = taffy::Dimension::length(v);
        self
    }

    pub fn items_center(mut self) -> Self {
        self.style.align_items = Some(taffy::AlignItems::Center);
        self
    }

    pub fn items_start(mut self) -> Self {
        self.style.align_items = Some(taffy::AlignItems::FlexStart);
        self
    }

    pub fn items_end(mut self) -> Self {
        self.style.align_items = Some(taffy::AlignItems::FlexEnd);
        self
    }

    pub fn justify_center(mut self) -> Self {
        self.style.justify_content = Some(taffy::JustifyContent::Center);
        self
    }

    pub fn justify_between(mut self) -> Self {
        self.style.justify_content = Some(taffy::JustifyContent::SpaceBetween);
        self
    }

    pub fn justify_end(mut self) -> Self {
        self.style.justify_content = Some(taffy::JustifyContent::FlexEnd);
        self
    }

    pub fn overflow_hidden(mut self) -> Self {
        self.style.overflow = taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Hidden,
        };
        self
    }

    pub fn overflow_y_scroll(mut self) -> Self {
        self.style.overflow.y = taffy::Overflow::Scroll;
        self
    }

    // -- Visual --

    pub fn bg(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub fn border_b(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self.border_width = 1.0;
        self
    }

    pub fn rounded(mut self, r: f32) -> Self {
        self.corner_radius = r;
        self
    }

    pub fn shadow(mut self, blur: f32, offset_y: f32, color: Color) -> Self {
        self.shadows.push(ShadowSpec {
            blur_radius: blur,
            offset: [0.0, offset_y],
            corner_radius: self.corner_radius,
            color,
        });
        self
    }
}

impl Element for Div {
    type LayoutState = Vec<LayoutId>;

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

        let id = engine.request_layout(self.style.clone(), &child_ids);
        (id, child_ids)
    }

    fn paint(
        &mut self,
        bounds: Bounds,
        _state: &mut Self::LayoutState,
        engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
    ) {
        let r = self.corner_radius;

        // Shadows
        for s in &self.shadows {
            scene.shadow(ShadowPrimitive {
                rect: bounds,
                blur_radius: s.blur_radius,
                corner_radius: s.corner_radius.max(r),
                offset: s.offset,
                color: s.color,
            });
        }

        // Background
        if let Some(bg) = self.background {
            scene.rounded_rect(RoundedRectPrimitive::uniform(bounds, r, bg));
        }

        // Border
        if let Some(border) = self.border_color {
            scene.border(BorderPrimitive::uniform(bounds, self.border_width, r, border));
        }

        // Paint children
        for child in &mut self.children {
            child.paint(engine, scene, cx);
        }
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

    fn paint(
        &mut self,
        _bounds: Bounds,
        _state: &mut (),
        _engine: &LayoutEngine,
        _scene: &mut Scene,
        _cx: &mut ElementContext,
    ) {
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;

    fn test_cx(font_system: &mut glyphon::FontSystem) -> ElementContext<'_> {
        // We leak the theme for test convenience — it's a test.
        let theme = Box::leak(Box::new(Theme::default_dark()));
        ElementContext {
            theme,
            scale_factor: 1.0,
            font_system,
        }
    }

    #[test]
    fn div_with_fixed_children_lays_out() {
        let mut font_system = glyphon::FontSystem::new();
        let mut cx = test_cx(&mut font_system);
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
        let mut cx = test_cx(&mut font_system);
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
        let mut cx = test_cx(&mut font_system);

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

        assert!((inner_bounds.x - padding).abs() < 1.0,
            "inner x={} should be near padding={}", inner_bounds.x, padding);
        assert!((inner_bounds.y - padding).abs() < 1.0,
            "inner y={} should be near padding={}", inner_bounds.y, padding);
        assert!((inner_bounds.width - inner_w).abs() < 1.0);
    }
}
