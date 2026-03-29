use crate::render::Rect;
use crate::ui::actions::Action;
use crate::ui::design::Sp;
use crate::ui::shell::UiFrame;
use crate::ui::theme::Theme;

use super::Button;

pub struct SegmentedControl<const N: usize> {
    items: [(&'static str, Action, bool); N],
}

impl<const N: usize> SegmentedControl<N> {
    pub fn new(items: [(&'static str, Action, bool); N]) -> Self {
        Self { items }
    }

    pub fn paint(self, frame: &mut UiFrame, rect: Rect, theme: &Theme) {
        let gap = Sp::MD;
        let total_gap = gap * (N as f32 - 1.0);
        let item_width = (rect.width - total_gap) / N as f32;

        for (i, (label, action, selected)) in self.items.into_iter().enumerate() {
            let item_rect = Rect {
                x: rect.x + i as f32 * (item_width + gap),
                y: rect.y,
                width: item_width,
                height: rect.height,
            };
            Button::new(label, action)
                .selected(selected)
                .paint(frame, item_rect, theme);
        }
    }
}
