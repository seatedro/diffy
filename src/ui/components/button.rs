use crate::render::{BorderPrimitive, FontKind, Rect, RoundedRectPrimitive, TextPrimitive};
use crate::ui::actions::Action;
use crate::ui::shell::{CursorHint, HitRegion, UiFrame};
use crate::ui::theme::{Color, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonStyle {
    /// Transparent at rest, reveals background on hover. For toolbar actions.
    #[default]
    Ghost,
    /// Faint background + border. For secondary actions.
    Subtle,
    /// Strong background. For primary actions (Start Compare, Load PR).
    Filled,
}

pub struct Button {
    label: String,
    action: Action,
    style: ButtonStyle,
    selected: bool,
    focused: bool,
}

impl Button {
    pub fn new(label: impl Into<String>, action: Action) -> Self {
        Self {
            label: label.into(),
            action,
            style: ButtonStyle::default(),
            selected: false,
            focused: false,
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

    pub fn paint(self, frame: &mut UiFrame, rect: Rect, theme: &Theme) {
        let radius = theme.metrics.control_radius;
        let (fill, border_color, text_color) = self.resolve_colors(theme);

        if fill.a > 0 {
            frame.scene.rounded_rect(RoundedRectPrimitive::uniform(rect, radius, fill));
        }
        if border_color.a > 0 {
            frame.scene.border(BorderPrimitive::uniform(rect, 1.0, radius, border_color));
        }

        let font_size = theme.metrics.ui_small_font_size;
        let line_height = font_size * 1.5;
        let text_y = rect.y + (rect.height - line_height) * 0.5;
        let pad_h = 14.0;
        frame.scene.text(TextPrimitive {
            rect: Rect {
                x: rect.x + pad_h,
                y: text_y,
                width: rect.width - pad_h * 2.0,
                height: line_height,
            },
            text: self.label,
            color: text_color,
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

    fn resolve_colors(&self, theme: &Theme) -> (Color, Color, Color) {
        let transparent = Color::rgba(0, 0, 0, 0);

        if self.focused {
            let fill = match self.style {
                ButtonStyle::Ghost => theme.colors.ghost_element_hover,
                _ => theme.colors.element_hover,
            };
            return (fill, theme.colors.focus_border, theme.colors.text);
        }

        if self.selected {
            return match self.style {
                ButtonStyle::Filled => (
                    theme.colors.accent,
                    transparent,
                    theme.colors.text_strong,
                ),
                _ => (
                    theme.colors.element_selected,
                    transparent,
                    theme.colors.text,
                ),
            };
        }

        match self.style {
            ButtonStyle::Ghost => (
                transparent,
                transparent,
                theme.colors.text_muted,
            ),
            ButtonStyle::Subtle => (
                theme.colors.element_background,
                theme.colors.border_variant,
                theme.colors.text,
            ),
            ButtonStyle::Filled => (
                theme.colors.element_selected,
                transparent,
                theme.colors.text,
            ),
        }
    }
}
