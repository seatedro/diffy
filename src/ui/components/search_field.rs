use crate::actions::Action;
use crate::ui::components::{Button, ButtonSize};
use crate::ui::design::{Ico, Sp};
use crate::ui::element::{Div, IntoAnyElement, div, svg_icon};
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
    let search_icon_size = Ico::XS;

    let mut container = div()
        .w_full()
        .flex_row()
        .items_center()
        .gap(m.spacing_sm)
        .px(m.spacing_sm + Sp::XXS)
        .py(m.spacing_xs)
        .rounded(m.control_radius)
        .border(tc.border_variant)
        .child(svg_icon(lucide::SEARCH, search_icon_size).color(tc.text_muted))
        .child(div().flex_1().min_w(0.0).child(input));

    if has_value {
        if let Some(clear_action) = on_clear {
            container = container.child(
                Button::new(clear_action)
                    .icon(lucide::X)
                    .size(ButtonSize::Compact),
            );
        }
    } else if let Some(hint) = shortcut_hint {
        container = container.child(super::kbd(hint, theme));
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
