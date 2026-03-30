use crate::ui::element::{div, Div};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopoverSide {
    Top,
    Bottom,
    Left,
    Right,
}

pub fn popover_panel(
    anchor_x: f32,
    anchor_y: f32,
    anchor_w: f32,
    anchor_h: f32,
    side: PopoverSide,
    theme: &Theme,
) -> Div {
    let tc = &theme.colors;
    let m = &theme.metrics;

    let gap = m.spacing_xs;
    let (x, y) = match side {
        PopoverSide::Bottom => (anchor_x, anchor_y + anchor_h + gap),
        PopoverSide::Top => (anchor_x, anchor_y - gap),
        PopoverSide::Right => (anchor_x + anchor_w + gap, anchor_y),
        PopoverSide::Left => (anchor_x - gap, anchor_y),
    };

    div()
        .absolute()
        .left(x)
        .top(y)
        .z_index(200)
        .flex_col()
        .bg(tc.elevated_surface)
        .border(tc.border)
        .rounded(m.panel_radius)
        .shadow(16.0, 8.0, Color::rgba(0, 0, 0, 40))
        .shadow(4.0, 2.0, Color::rgba(0, 0, 0, 30))
        .shadow(1.0, 0.0, Color::rgba(0, 0, 0, 15))
}

pub fn popover_section() -> Div {
    div().flex_col().w_full()
}

pub fn popover_divider(theme: &Theme) -> Div {
    let tc = &theme.colors;
    div()
        .w_full()
        .py(4.0)
        .child(div().w_full().h(1.0).bg(tc.border_variant))
}
