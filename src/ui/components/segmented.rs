use crate::ui::actions::Action;
use crate::ui::design::{Rad, Sp};
use crate::ui::element::*;
use crate::ui::shell::CursorHint;
use crate::ui::style::Styled;
use crate::ui::theme::Color;

pub struct SegmentedItem {
    pub label: String,
    pub action: Action,
    pub selected: bool,
}

impl SegmentedItem {
    pub fn new(label: impl Into<String>, action: Action, selected: bool) -> Self {
        Self {
            label: label.into(),
            action,
            selected,
        }
    }
}

pub struct SegmentedControl {
    items: Vec<SegmentedItem>,
}

impl SegmentedControl {
    pub fn new(items: Vec<SegmentedItem>) -> Self {
        Self { items }
    }
}

impl RenderOnce for SegmentedControl {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let scale = (cx.theme.metrics.ui_font_size / 16.0).max(0.7);

        let mut row = div()
            .flex_row()
            .flex_shrink_0()
            .rounded_md()
            .bg(tc.element_background)
            .p((Sp::XXS * scale).round() + 1.0)
            .gap((Sp::XXS * scale).round());

        for item in self.items {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .px((Sp::MD * scale).round())
                    .py((Sp::XS * scale).round() + 1.0)
                    .rounded((Rad::LG * scale).round())
                    .when(item.selected, |d| {
                        d.bg(tc.surface)
                            .shadow(2.0, 1.0, Color::rgba(0, 0, 0, 40))
                    })
                    .when(!item.selected, |d| d.hover_bg(tc.ghost_element_hover))
                    .on_click(item.action)
                    .cursor(CursorHint::Pointer)
                    .child(
                        text(&item.label)
                            .text_sm()
                            .medium()
                            .color(if item.selected { tc.text } else { tc.text_muted }),
                    ),
            );
        }

        row.into_any()
    }
}
