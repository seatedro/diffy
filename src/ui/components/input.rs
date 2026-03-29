use crate::render::{BorderPrimitive, FontKind, Rect, RoundedRectPrimitive, TextPrimitive};
use crate::ui::actions::Action;
use crate::ui::design::Sp;
use crate::ui::shell::{CursorHint, HitRegion, UiFrame};
use crate::ui::theme::Theme;

pub struct TextInput<'a> {
    label: &'a str,
    value: &'a str,
    placeholder: &'a str,
    focused: bool,
    on_click: Action,
}

impl<'a> TextInput<'a> {
    pub fn new(label: &'a str, value: &'a str) -> Self {
        Self {
            label,
            value,
            placeholder: "",
            focused: false,
            on_click: Action::Bootstrap,
        }
    }

    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn on_click(mut self, action: Action) -> Self {
        self.on_click = action;
        self
    }

    pub fn paint(self, frame: &mut UiFrame, rect: Rect, theme: &Theme) {
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

        frame.scene.rounded_rect(RoundedRectPrimitive {
            rect,
            radius: theme.metrics.control_radius,
            color: fill,
        });
        frame.scene.border(BorderPrimitive {
            rect,
            width: 1.0,
            radius: theme.metrics.control_radius,
            color: border,
        });

        let label_size = theme.metrics.ui_small_font_size;
        let value_size = theme.metrics.ui_font_size;
        let label_lh = label_size * 1.35;
        let value_lh = value_size * 1.35;

        frame.scene.text(TextPrimitive {
            rect: Rect {
                x: rect.x + Sp::LG,
                y: rect.y + Sp::SM,
                width: rect.width - Sp::XXL,
                height: label_lh,
            },
            text: self.label.to_owned(),
            color: theme.colors.text_muted,
            font_size: label_size,
            font_kind: FontKind::Ui,
        });

        let display = if self.value.is_empty() {
            self.placeholder
        } else {
            self.value
        };
        let text_color = if self.value.is_empty() {
            theme.colors.text_muted.with_alpha(180)
        } else {
            theme.colors.text
        };
        frame.scene.text(TextPrimitive {
            rect: Rect {
                x: rect.x + Sp::LG,
                y: rect.y + Sp::SM + label_lh,
                width: rect.width - Sp::XXL,
                height: value_lh,
            },
            text: display.to_owned(),
            color: text_color,
            font_size: value_size,
            font_kind: FontKind::Ui,
        });

        frame.hits.push(HitRegion {
            rect,
            action: self.on_click,
            hover_file_index: None,
            hover_toast_index: None,
            cursor: CursorHint::Text,
        });
    }
}
