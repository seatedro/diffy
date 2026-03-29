//! Style system — shared layout + visual properties for elements.

use crate::ui::theme::Color;

// ---------------------------------------------------------------------------
// ShadowStyle
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ShadowStyle {
    pub blur_radius: f32,
    pub offset: [f32; 2],
    pub corner_radius: f32,
    pub color: Color,
}

// ---------------------------------------------------------------------------
// ElementStyle — combined layout + visual
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ElementStyle {
    pub layout: taffy::Style,
    pub background: Option<Color>,
    pub border_color: Option<Color>,
    pub border_widths: [f32; 4],
    pub corner_radius: f32,
    pub opacity: f32,
    pub z_index: i32,
    pub shadows: Vec<ShadowStyle>,
}

impl Default for ElementStyle {
    fn default() -> Self {
        Self {
            layout: taffy::Style {
                display: taffy::Display::Flex,
                ..Default::default()
            },
            background: None,
            border_color: None,
            border_widths: [0.0; 4],
            corner_radius: 0.0,
            opacity: 1.0,
            z_index: 0,
            shadows: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// StyleOverride — partial overlay for hover/active/focus
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct StyleOverride {
    pub background: Option<Color>,
    pub border_color: Option<Color>,
    pub corner_radius: Option<f32>,
    pub opacity: Option<f32>,
}

impl StyleOverride {
    pub fn bg(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub fn border_color(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self
    }

    pub fn rounded(mut self, r: f32) -> Self {
        self.corner_radius = Some(r);
        self
    }

    pub fn opacity(mut self, v: f32) -> Self {
        self.opacity = Some(v);
        self
    }
}

pub fn apply_override(base: &mut ElementStyle, ov: &StyleOverride) {
    if let Some(bg) = ov.background {
        base.background = Some(bg);
    }
    if let Some(bc) = ov.border_color {
        base.border_color = Some(bc);
    }
    if let Some(cr) = ov.corner_radius {
        base.corner_radius = cr;
    }
    if let Some(op) = ov.opacity {
        base.opacity = op;
    }
}

// ---------------------------------------------------------------------------
// Styled trait — fluent setters shared across element types
// ---------------------------------------------------------------------------

pub trait Styled: Sized {
    fn element_style_mut(&mut self) -> &mut ElementStyle;

    // -- Layout --

    fn flex_row(mut self) -> Self {
        self.element_style_mut().layout.flex_direction = taffy::FlexDirection::Row;
        self
    }

    fn flex_col(mut self) -> Self {
        self.element_style_mut().layout.flex_direction = taffy::FlexDirection::Column;
        self
    }

    fn flex_1(mut self) -> Self {
        let l = &mut self.element_style_mut().layout;
        l.flex_grow = 1.0;
        l.flex_shrink = 1.0;
        l.flex_basis = taffy::Dimension::percent(0.0);
        self
    }

    fn flex_grow(mut self) -> Self {
        self.element_style_mut().layout.flex_grow = 1.0;
        self
    }

    fn flex_shrink_0(mut self) -> Self {
        self.element_style_mut().layout.flex_shrink = 0.0;
        self
    }

    fn gap(mut self, v: f32) -> Self {
        self.element_style_mut().layout.gap = taffy::Size {
            width: taffy::LengthPercentage::length(v),
            height: taffy::LengthPercentage::length(v),
        };
        self
    }

    fn gap_x(mut self, v: f32) -> Self {
        self.element_style_mut().layout.gap.width = taffy::LengthPercentage::length(v);
        self
    }

    fn gap_y(mut self, v: f32) -> Self {
        self.element_style_mut().layout.gap.height = taffy::LengthPercentage::length(v);
        self
    }

    // -- Tailwind-style spacing shortcuts (4px base grid) --

    fn p_1(self) -> Self { self.p(4.0) }
    fn p_2(self) -> Self { self.p(8.0) }
    fn p_3(self) -> Self { self.p(12.0) }
    fn p_4(self) -> Self { self.p(16.0) }
    fn p_5(self) -> Self { self.p(20.0) }
    fn p_6(self) -> Self { self.p(24.0) }
    fn p_8(self) -> Self { self.p(32.0) }

    fn px_2(self) -> Self { self.px(8.0) }
    fn px_3(self) -> Self { self.px(12.0) }
    fn px_4(self) -> Self { self.px(16.0) }
    fn px_5(self) -> Self { self.px(20.0) }
    fn px_6(self) -> Self { self.px(24.0) }

    fn py_1(self) -> Self { self.py(4.0) }
    fn py_2(self) -> Self { self.py(8.0) }
    fn py_3(self) -> Self { self.py(12.0) }

    fn gap_1(self) -> Self { self.gap(4.0) }
    fn gap_2(self) -> Self { self.gap(8.0) }
    fn gap_3(self) -> Self { self.gap(12.0) }
    fn gap_4(self) -> Self { self.gap(16.0) }

    fn rounded_sm(self) -> Self { self.rounded(6.0) }
    fn rounded_md(self) -> Self { self.rounded(8.0) }
    fn rounded_lg(self) -> Self { self.rounded(12.0) }
    fn rounded_xl(self) -> Self { self.rounded(16.0) }

    fn h_10(self) -> Self { self.h(40.0) }
    fn h_12(self) -> Self { self.h(48.0) }

    // -- Raw value methods --

    fn p(mut self, v: f32) -> Self {
        let l = taffy::LengthPercentage::length(v);
        self.element_style_mut().layout.padding = taffy::Rect {
            left: l, right: l, top: l, bottom: l,
        };
        self
    }

    fn px(mut self, v: f32) -> Self {
        let l = taffy::LengthPercentage::length(v);
        let p = &mut self.element_style_mut().layout.padding;
        p.left = l;
        p.right = l;
        self
    }

    fn py(mut self, v: f32) -> Self {
        let l = taffy::LengthPercentage::length(v);
        let p = &mut self.element_style_mut().layout.padding;
        p.top = l;
        p.bottom = l;
        self
    }

    fn pt(mut self, v: f32) -> Self {
        self.element_style_mut().layout.padding.top = taffy::LengthPercentage::length(v);
        self
    }

    fn pb(mut self, v: f32) -> Self {
        self.element_style_mut().layout.padding.bottom = taffy::LengthPercentage::length(v);
        self
    }

    fn w(mut self, v: f32) -> Self {
        self.element_style_mut().layout.size.width = taffy::Dimension::length(v);
        self
    }

    fn h(mut self, v: f32) -> Self {
        self.element_style_mut().layout.size.height = taffy::Dimension::length(v);
        self
    }

    fn w_full(mut self) -> Self {
        self.element_style_mut().layout.size.width = taffy::Dimension::percent(1.0);
        self
    }

    fn h_full(mut self) -> Self {
        self.element_style_mut().layout.size.height = taffy::Dimension::percent(1.0);
        self
    }

    fn min_w(mut self, v: f32) -> Self {
        self.element_style_mut().layout.min_size.width = taffy::Dimension::length(v);
        self
    }

    fn min_h(mut self, v: f32) -> Self {
        self.element_style_mut().layout.min_size.height = taffy::Dimension::length(v);
        self
    }

    fn items_center(mut self) -> Self {
        self.element_style_mut().layout.align_items = Some(taffy::AlignItems::Center);
        self
    }

    fn items_start(mut self) -> Self {
        self.element_style_mut().layout.align_items = Some(taffy::AlignItems::FlexStart);
        self
    }

    fn items_end(mut self) -> Self {
        self.element_style_mut().layout.align_items = Some(taffy::AlignItems::FlexEnd);
        self
    }

    fn justify_center(mut self) -> Self {
        self.element_style_mut().layout.justify_content = Some(taffy::JustifyContent::Center);
        self
    }

    fn justify_between(mut self) -> Self {
        self.element_style_mut().layout.justify_content = Some(taffy::JustifyContent::SpaceBetween);
        self
    }

    fn justify_end(mut self) -> Self {
        self.element_style_mut().layout.justify_content = Some(taffy::JustifyContent::FlexEnd);
        self
    }

    fn overflow_hidden(mut self) -> Self {
        self.element_style_mut().layout.overflow = taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Hidden,
        };
        self
    }

    fn overflow_y_scroll(mut self) -> Self {
        self.element_style_mut().layout.overflow.y = taffy::Overflow::Scroll;
        self
    }

    // -- Visual --

    fn bg(mut self, color: Color) -> Self {
        self.element_style_mut().background = Some(color);
        self
    }

    fn border(mut self, color: Color) -> Self {
        let s = self.element_style_mut();
        s.border_color = Some(color);
        s.border_widths = [1.0; 4];
        self
    }

    fn border_t(mut self, color: Color) -> Self {
        let s = self.element_style_mut();
        s.border_color = Some(color);
        s.border_widths[0] = 1.0;
        self
    }

    fn border_r(mut self, color: Color) -> Self {
        let s = self.element_style_mut();
        s.border_color = Some(color);
        s.border_widths[1] = 1.0;
        self
    }

    fn border_b(mut self, color: Color) -> Self {
        let s = self.element_style_mut();
        s.border_color = Some(color);
        s.border_widths[2] = 1.0;
        self
    }

    fn border_l(mut self, color: Color) -> Self {
        let s = self.element_style_mut();
        s.border_color = Some(color);
        s.border_widths[3] = 1.0;
        self
    }

    fn rounded(mut self, r: f32) -> Self {
        self.element_style_mut().corner_radius = r;
        self
    }

    fn opacity(mut self, v: f32) -> Self {
        self.element_style_mut().opacity = v;
        self
    }

    fn shadow(mut self, blur: f32, offset_y: f32, color: Color) -> Self {
        let r = self.element_style_mut().corner_radius;
        self.element_style_mut().shadows.push(ShadowStyle {
            blur_radius: blur,
            offset: [0.0, offset_y],
            corner_radius: r,
            color,
        });
        self
    }

    /// Outer glow — a colored halo around the element (e.g. focus indicator).
    /// Implemented as a zero-offset shadow with the given color and radius.
    fn glow(self, color: Color, radius: f32) -> Self {
        self.shadow(radius, 0.0, color)
    }

    /// Set the z-index for rendering order. Higher values render on top.
    /// Default is 0. Modals typically use 100+, toasts 200+.
    fn z_index(mut self, z: i32) -> Self {
        self.element_style_mut().z_index = z;
        self
    }

    fn absolute(mut self) -> Self {
        self.element_style_mut().layout.position = taffy::Position::Absolute;
        self
    }

    fn top(mut self, v: f32) -> Self {
        self.element_style_mut().layout.inset.top = taffy::LengthPercentageAuto::length(v);
        self
    }

    fn bottom(mut self, v: f32) -> Self {
        self.element_style_mut().layout.inset.bottom = taffy::LengthPercentageAuto::length(v);
        self
    }

    fn left(mut self, v: f32) -> Self {
        self.element_style_mut().layout.inset.left = taffy::LengthPercentageAuto::length(v);
        self
    }

    fn right(mut self, v: f32) -> Self {
        self.element_style_mut().layout.inset.right = taffy::LengthPercentageAuto::length(v);
        self
    }

    fn inset(mut self, v: f32) -> Self {
        let l = taffy::LengthPercentageAuto::length(v);
        self.element_style_mut().layout.inset = taffy::Rect { left: l, right: l, top: l, bottom: l };
        self
    }

    fn max_w(mut self, v: f32) -> Self {
        self.element_style_mut().layout.max_size.width = taffy::Dimension::length(v);
        self
    }

    fn max_h(mut self, v: f32) -> Self {
        self.element_style_mut().layout.max_size.height = taffy::Dimension::length(v);
        self
    }

    fn flex_wrap(mut self) -> Self {
        self.element_style_mut().layout.flex_wrap = taffy::FlexWrap::Wrap;
        self
    }
}
