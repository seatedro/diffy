use crate::ui::design::{Shadow, Sp};
use crate::ui::element::{div, text, Div};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooltipSide {
    Top,
    Bottom,
    Left,
    Right,
}

pub fn tooltip_layer(content: &str, x: f32, y: f32, side: TooltipSide, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let m = &theme.metrics;

    let (offset_x, offset_y) = match side {
        TooltipSide::Top => (0.0, -(m.spacing_sm + Sp::XS)),
        TooltipSide::Bottom => (0.0, m.spacing_sm + Sp::XS),
        TooltipSide::Left => (-(m.spacing_sm + Sp::XS), 0.0),
        TooltipSide::Right => (m.spacing_sm + Sp::XS, 0.0),
    };

    div()
        .absolute()
        .left(x + offset_x)
        .top(y + offset_y)
        .z_index(500)
        .px(m.spacing_sm)
        .py(m.spacing_xs)
        .bg(tc.elevated_surface)
        .border(tc.border)
        .rounded(m.control_radius - Sp::XXS)
        .shadow_preset(Shadow::TOOLTIP)
        .child(text(content).text_xs().color(tc.text))
}

pub struct TooltipState {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub side: TooltipSide,
    pub visible: bool,
    pub show_at_ms: u64,
}

impl Default for TooltipState {
    fn default() -> Self {
        Self {
            text: String::new(),
            x: 0.0,
            y: 0.0,
            side: TooltipSide::Bottom,
            visible: false,
            show_at_ms: 0,
        }
    }
}

impl TooltipState {
    pub fn show(&mut self, text: impl Into<String>, x: f32, y: f32, side: TooltipSide, delay_ms: u64, now_ms: u64) {
        self.text = text.into();
        self.x = x;
        self.y = y;
        self.side = side;
        self.show_at_ms = now_ms + delay_ms;
        self.visible = false;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.text.clear();
    }

    pub fn tick(&mut self, now_ms: u64) {
        if !self.text.is_empty() && !self.visible && now_ms >= self.show_at_ms {
            self.visible = true;
        }
    }

    pub fn render(&self, theme: &Theme) -> Option<Div> {
        if self.visible && !self.text.is_empty() {
            Some(tooltip_layer(&self.text, self.x, self.y, self.side, theme))
        } else {
            None
        }
    }
}
