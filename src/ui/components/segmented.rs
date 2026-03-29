use crate::render::{Rect, RoundedRectPrimitive};
use crate::ui::actions::Action;
use crate::ui::shell::UiFrame;
use crate::ui::theme::Theme;

use super::{Button, ButtonStyle};

pub struct SegmentedControl<const N: usize> {
    items: [(&'static str, Action, bool); N],
}

impl<const N: usize> SegmentedControl<N> {
    pub fn new(items: [(&'static str, Action, bool); N]) -> Self {
        Self { items }
    }

    pub fn paint(self, frame: &mut UiFrame, rect: Rect, theme: &Theme) {
        // Container background
        frame.scene.rounded_rect(RoundedRectPrimitive::uniform(
            rect,
            theme.metrics.control_radius + 2.0,
            theme.colors.element_background,
        ));

        let inset = 2.0;
        let inner = rect.pad(inset, inset, inset, inset);
        let gap = 2.0;
        let total_gap = gap * (N as f32 - 1.0);
        let item_width = (inner.width - total_gap) / N as f32;

        for (i, (label, action, selected)) in self.items.into_iter().enumerate() {
            let item_rect = Rect {
                x: inner.x + i as f32 * (item_width + gap),
                y: inner.y,
                width: item_width,
                height: inner.height,
            };
            Button::new(label, action)
                .style(if selected { ButtonStyle::Filled } else { ButtonStyle::Ghost })
                .selected(selected)
                .paint(frame, item_rect, theme);
        }
    }
}
