use crate::ui::actions::Action;
use crate::ui::element::{div, svg_icon, Div, IntoAnyElement};
use crate::ui::icons::lucide;
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

pub fn search_field(
    input: impl IntoAnyElement,
    has_value: bool,
    on_clear: Option<Action>,
    theme: &Theme,
) -> Div {
    let tc = &theme.colors;
    let m = &theme.metrics;
    let icon_size = m.ui_small_font_size;

    let mut container = div()
        .flex_row()
        .items_center()
        .gap(m.spacing_xs)
        .px(m.spacing_sm)
        .bg(tc.element_background)
        .rounded(m.control_radius)
        .border(tc.border_variant)
        .child(svg_icon(lucide::SEARCH, icon_size).color(tc.text_muted))
        .child(div().flex_1().child(input));

    if has_value {
        if let Some(clear_action) = on_clear {
            let clear_size = icon_size + 4.0;
            container = container.child(
                div()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .w(clear_size)
                    .h(clear_size)
                    .rounded(clear_size / 2.0)
                    .hover_bg(tc.ghost_element_hover)
                    .on_click(clear_action)
                    .child(svg_icon(lucide::X, icon_size - 2.0).color(tc.text_muted)),
            );
        }
    }

    container
}

pub fn filter_bar(theme: &Theme) -> Div {
    let tc = &theme.colors;
    let m = &theme.metrics;

    div()
        .flex_row()
        .items_center()
        .gap(m.spacing_sm)
        .px(m.spacing_sm)
        .py(m.spacing_xs)
        .border_b(tc.border_variant)
}
