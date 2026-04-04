use crate::ui::design::{Sp, Sz};
use crate::ui::element::*;
use crate::ui::style::Styled;

pub struct Toolbar {
    left: Vec<AnyElement>,
    right: Vec<AnyElement>,
}

impl Toolbar {
    pub fn new() -> Self {
        Self {
            left: Vec::new(),
            right: Vec::new(),
        }
    }

    pub fn left_child(mut self, child: impl IntoAnyElement) -> Self {
        self.left.push(child.into_any());
        self
    }

    pub fn right_child(mut self, child: impl IntoAnyElement) -> Self {
        self.right.push(child.into_any());
        self
    }
}

impl RenderOnce for Toolbar {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let scale = (cx.theme.metrics.ui_font_size / 16.0).max(0.7);

        let mut left = div().flex_row().items_center().gap_1();
        for child in self.left {
            left = left.child(child);
        }

        let mut right = div().flex_row().items_center().gap_1();
        for child in self.right {
            right = right.child(child);
        }

        div()
            .h((Sz::ROW * scale).round())
            .w_full()
            .flex_row()
            .items_center()
            .px((Sp::MD * scale).round())
            .border_b(tc.border_variant)
            .child(left)
            .child(spacer())
            .child(right)
            .into_any()
    }
}
