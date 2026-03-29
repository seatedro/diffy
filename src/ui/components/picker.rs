use crate::render::{FontKind, Rect, RoundedRectPrimitive, TextPrimitive};
use crate::ui::actions::Action;
use crate::ui::design::Sp;
use crate::ui::shell::{CursorHint, HitRegion, UiFrame};
use crate::ui::state::{PaletteEntry, PickerEntry};
use crate::ui::theme::Theme;

pub trait PickerItem {
    fn label(&self) -> &str;
    fn detail(&self) -> &str;
}

impl PickerItem for PickerEntry {
    fn label(&self) -> &str {
        &self.label
    }
    fn detail(&self) -> &str {
        &self.detail
    }
}

impl PickerItem for PaletteEntry {
    fn label(&self) -> &str {
        &self.label
    }
    fn detail(&self) -> &str {
        &self.detail
    }
}

pub struct PickerList<'a, T: PickerItem> {
    entries: &'a [T],
    selected_index: usize,
    row_height: f32,
}

impl<'a, T: PickerItem> PickerList<'a, T> {
    pub fn new(entries: &'a [T], selected_index: usize) -> Self {
        Self {
            entries,
            selected_index,
            row_height: 36.0,
        }
    }

    pub fn row_height(mut self, h: f32) -> Self {
        self.row_height = h;
        self
    }

    pub fn paint(self, frame: &mut UiFrame, rect: Rect, theme: &Theme) {
        frame.scene.clip(rect);
        let mut y = rect.y + Sp::XS;

        for (index, entry) in self.entries.iter().enumerate() {
            let row = Rect {
                x: rect.x + Sp::XS,
                y,
                width: rect.width - Sp::MD,
                height: self.row_height - 2.0,
            };

            if index == self.selected_index {
                frame.scene.rounded_rect(RoundedRectPrimitive {
                    rect: row,
                    radius: theme.metrics.control_radius,
                    color: theme.colors.ghost_element_selected,
                });
            }

            let title_lh = theme.metrics.ui_font_size * 1.35;
            let detail_lh = theme.metrics.ui_small_font_size * 1.35;
            frame.scene.text(TextPrimitive {
                rect: Rect {
                    x: row.x + Sp::LG,
                    y: row.y + Sp::XS,
                    width: row.width - Sp::XXL,
                    height: title_lh,
                },
                text: entry.label().to_owned(),
                color: theme.colors.text,
                font_size: theme.metrics.ui_font_size,
                font_kind: FontKind::Ui,
            });

            frame.scene.text(TextPrimitive {
                rect: Rect {
                    x: row.x + Sp::LG,
                    y: row.y + Sp::XS + title_lh,
                    width: row.width - Sp::XXL,
                    height: detail_lh,
                },
                text: entry.detail().to_owned(),
                color: theme.colors.text_muted,
                font_size: theme.metrics.ui_small_font_size,
                font_kind: FontKind::Ui,
            });

            frame.hits.push(HitRegion {
                rect: row,
                action: Action::SelectOverlayEntry(index),
                hover_file_index: None,
                hover_toast_index: None,
                cursor: CursorHint::Pointer,
            });

            y += self.row_height;
        }
        frame.scene.pop_clip();
    }
}
