use crate::render::{BorderPrimitive, Rect, RoundedRectPrimitive, TextPrimitive, FontKind};
use crate::ui::actions::Action;
use crate::ui::design::Sp;
use crate::ui::shell::{CursorHint, HitRegion, UiFrame};
use crate::ui::theme::{Color, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonStyle {
    #[default]
    Subtle,
    Filled,
    Ghost,
    Tinted,
}

pub struct Button {
    label: String,
    action: Action,
    style: ButtonStyle,
    selected: bool,
    focused: bool,
    tint: Option<Color>,
}

impl Button {
    pub fn new(label: impl Into<String>, action: Action) -> Self {
        Self {
            label: label.into(),
            action,
            style: ButtonStyle::default(),
            selected: false,
            focused: false,
            tint: None,
        }
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn tint(mut self, color: Color) -> Self {
        self.tint = Some(color);
        self.style = ButtonStyle::Tinted;
        self
    }

    pub fn paint(self, frame: &mut UiFrame, rect: Rect, theme: &Theme) {
        let (fill, border_color) = self.resolve_colors(theme);

        frame.scene.rounded_rect(RoundedRectPrimitive {
            rect,
            radius: theme.metrics.control_radius,
            color: fill,
        });
        frame.scene.border(BorderPrimitive {
            rect,
            width: 1.0,
            radius: theme.metrics.control_radius,
            color: border_color,
        });

        let font_size = theme.metrics.ui_small_font_size;
        let line_height = font_size * 1.35;
        let text_y = rect.y + (rect.height - line_height) * 0.5;
        frame.scene.text(TextPrimitive {
            rect: Rect {
                x: rect.x + Sp::LG,
                y: text_y,
                width: rect.width - Sp::LG * 2.0,
                height: line_height,
            },
            text: self.label,
            color: theme.colors.text,
            font_size,
            font_kind: FontKind::Ui,
        });

        frame.hits.push(HitRegion {
            rect,
            action: self.action,
            hover_file_index: None,
            hover_toast_index: None,
            cursor: CursorHint::Pointer,
        });
    }

    fn resolve_colors(&self, theme: &Theme) -> (Color, Color) {
        let fill = if self.selected {
            theme.colors.element_selected
        } else if self.focused {
            theme.colors.element_hover
        } else {
            match self.style {
                ButtonStyle::Subtle => theme.colors.element_background,
                ButtonStyle::Filled => theme.colors.element_selected,
                ButtonStyle::Ghost => Color::rgba(0, 0, 0, 0),
                ButtonStyle::Tinted => self.tint.unwrap_or(theme.colors.element_selected),
            }
        };
        let border = if self.focused {
            theme.colors.focus_border
        } else {
            match self.style {
                ButtonStyle::Ghost => Color::rgba(0, 0, 0, 0),
                _ => theme.colors.border,
            }
        };
        (fill, border)
    }
}
