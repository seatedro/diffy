use crate::ui::actions::Action;
use crate::ui::design::{Alpha, Ico, Rad, Sp};
use crate::ui::element::{div, svg_icon, text, Div, IntoAnyElement};
use crate::ui::icons::lucide;
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

pub fn search_field(
    input: impl IntoAnyElement,
    has_value: bool,
    on_clear: Option<Action>,
    shortcut_hint: Option<&str>,
    theme: &Theme,
) -> Div {
    let tc = &theme.colors;
    let m = &theme.metrics;
    let icon_size = m.ui_small_font_size;

    let search_icon_size = (icon_size - 1.0).max(Ico::XS);

    let mut container = div()
        .w_full()
        .flex_row()
        .items_center()
        .gap(m.spacing_sm)
        .px(m.spacing_sm + Sp::XXS)
        .py(m.spacing_xs)
        .bg(tc.element_background)
        .rounded(m.control_radius)
        .border(tc.border_variant)
        .child(svg_icon(lucide::SEARCH, search_icon_size).color(tc.text_muted))
        .child(div().flex_1().min_w(0.0).child(input));

    if has_value {
        if let Some(clear_action) = on_clear {
            let clear_size = icon_size + Sp::XS;
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
                    .child(svg_icon(lucide::X, icon_size - Sp::XXS).color(tc.text_muted)),
            );
        }
    } else if let Some(hint) = shortcut_hint {
        let kbd_h = m.ui_small_font_size + Sp::XXS;
        container = container.child(
            div()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .h(kbd_h)
                .min_w(kbd_h)
                .px(Sp::XS)
                .border(tc.border_variant)
                .rounded(Rad::SM)
                .shadow(1.0, 1.0, tc.border_soft.with_alpha(Alpha::FAINT))
                .child(
                    text(hint)
                        .text_xs()
                        .color(tc.text_muted)
                        .mono()
                        .text_center(),
                ),
        );
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
