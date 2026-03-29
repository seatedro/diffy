use crate::render::Rect;
use crate::ui::design::Elevation;
use crate::ui::shell::UiFrame;
use crate::ui::theme::{Color, Theme};

pub struct Surface {
    elevation: Elevation,
    fill: Option<Color>,
    border: Option<Color>,
}

impl Surface {
    pub fn new(elevation: Elevation) -> Self {
        Self {
            elevation,
            fill: None,
            border: None,
        }
    }

    pub fn panel() -> Self {
        Self::new(Elevation::Surface)
    }

    pub fn raised() -> Self {
        Self::new(Elevation::Raised)
    }

    pub fn modal() -> Self {
        Self::new(Elevation::Modal)
    }

    pub fn popover() -> Self {
        Self::new(Elevation::Popover)
    }

    pub fn fill(mut self, color: Color) -> Self {
        self.fill = Some(color);
        self
    }

    pub fn border(mut self, color: Color) -> Self {
        self.border = Some(color);
        self
    }

    pub fn paint(self, frame: &mut UiFrame, rect: Rect, theme: &Theme) {
        let fill = self.fill.unwrap_or_else(|| self.elevation.default_fill(theme));
        let border = self.border.unwrap_or_else(|| self.elevation.default_border(theme));
        self.elevation.paint(frame, rect, fill, border, theme);
    }
}
