use halogen::view;

use crate::ui::actions::Action;
use crate::ui::design::{Ico, Rad, Sp, Sz};
use crate::ui::element::*;
use crate::ui::shell::CursorHint;
use crate::ui::state::PickerItem;
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

pub fn picker_list<T: PickerItem>(
    entries: &[T],
    selected_index: usize,
    scroll_top_px: f32,
    viewport_h: f32,
    theme: &Theme,
) -> Div {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let row_h = (Sz::ROW * scale).round();
    let icon_size = (Ico::XS * scale).round();
    let total_h = entries.len() as f32 * row_h;

    let list_h = total_h.min(viewport_h);
    let scroll = scroll_top_px.min((total_h - list_h).max(0.0));

    let mut list = div()
        .w_full()
        .flex_col()
        .h(list_h)
        .scroll_y(scroll)
        .scroll_total(total_h)
        .hide_scrollbar();

    for (i, entry) in entries.iter().enumerate() {
        if entry.is_section_header() {
            list = list.child(view! { scale,
                <div class="w-full flex-row items-center" h={row_h} px={Sp::MD}>
                    <text class="text-xs truncate" color={tc.text_muted}>{entry.label()}</text>
                </div>
            });
            continue;
        }
        let selected = i == selected_index;
        list = list.child(
            div()
                .w_full()
                .h(row_h)
                .flex_row()
                .items_center()
                .gap((Sp::SM * scale).round())
                .px((Sp::MD * scale).round())
                .rounded((Rad::MD * scale).round())
                .when(selected, |d| d.bg(tc.sidebar_row_selected))
                .when(!selected, |d| d.hover_bg(tc.ghost_element_hover))
                .on_click(Action::SelectOverlayEntry(i))
                .cursor(CursorHint::Pointer)
                .optional_child(
                    entry
                        .icon_svg()
                        .map(|svg| svg_icon(svg, icon_size).color(tc.icon)),
                )
                .child(
                    picker_label(
                        entry.label(),
                        entry.highlight_range(),
                        selected,
                        theme,
                    ),
                )
                .optional_child(
                    entry
                        .detail()
                        .filter(|d| !d.is_empty())
                        .map(|d| text(d).text_xs().color(tc.text_muted).truncate()),
                ),
        );
    }

    list
}

fn picker_label(
    label_text: &str,
    highlight: Option<(usize, usize)>,
    selected: bool,
    theme: &Theme,
) -> Div {
    let tc = &theme.colors;
    let base_color = if selected { tc.text_strong } else { tc.text };

    let container = div().flex_1().overflow_hidden();

    match highlight {
        Some((start, end)) if start < end && end <= label_text.len() => {
            let before = &label_text[..start];
            let matched = &label_text[start..end];
            let after = &label_text[end..];
            let row = view! {
                <div class="flex-row overflow-hidden">
                    if !before.is_empty() {
                        <text class="text-sm" color={base_color}>{before}</text>
                    }
                    <text class="text-sm" color={tc.accent}>{matched}</text>
                    if !after.is_empty() {
                        <text class="text-sm" color={base_color}>{after}</text>
                    }
                </div>
            };
            container.child(row)
        }
        _ => container.child(
            text(label_text)
                .text_sm()
                .color(base_color)
                .truncate(),
        ),
    }
}
