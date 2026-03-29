use crate::render::{Rect, TextPrimitive};
use crate::ui::design::TextStyle;
use crate::ui::theme::{Color, Theme};

use super::super::shell::UiFrame;

const LINE_HEIGHT_FACTOR: f32 = 1.35;

pub struct Label<'a> {
    text: &'a str,
    style: TextStyle,
    color: Option<Color>,
}

impl<'a> Label<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            style: TextStyle::Body,
            color: None,
        }
    }

    pub fn style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn paint(self, frame: &mut UiFrame, rect: Rect, theme: &Theme) {
        let font_size = self.style.font_size(theme);
        let min_height = font_size * LINE_HEIGHT_FACTOR;
        let text_rect = Rect {
            height: rect.height.max(min_height),
            ..rect
        };
        frame.scene.text(TextPrimitive {
            rect: text_rect,
            text: self.text.to_owned(),
            color: self.color.unwrap_or_else(|| self.style.color(theme)),
            font_size,
            font_kind: self.style.font_kind(),
        });
    }

    pub fn paint_into(self, scene: &mut crate::render::Scene, rect: Rect, theme: &Theme) {
        let font_size = self.style.font_size(theme);
        let min_height = font_size * LINE_HEIGHT_FACTOR;
        let text_rect = Rect {
            height: rect.height.max(min_height),
            ..rect
        };
        scene.text(TextPrimitive {
            rect: text_rect,
            text: self.text.to_owned(),
            color: self.color.unwrap_or_else(|| self.style.color(theme)),
            font_size,
            font_kind: self.style.font_kind(),
        });
    }
}
