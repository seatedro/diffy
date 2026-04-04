use crate::ui::actions::Action;
use crate::ui::design::{Rad, Sp, Sz};
use crate::ui::element::*;
use crate::ui::shell::CursorHint;
use crate::ui::state::PickerItem;
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

pub fn picker_list<T: PickerItem>(
    entries: &[T],
    selected_index: usize,
    scroll_top_px: u32,
    theme: &Theme,
) -> Div {
    picker_list_inner(entries, selected_index, scroll_top_px, theme, true, false)
}

pub fn picker_list_no_scrollbar<T: PickerItem>(
    entries: &[T],
    selected_index: usize,
    scroll_top_px: u32,
    theme: &Theme,
) -> Div {
    picker_list_inner(entries, selected_index, scroll_top_px, theme, true, true)
}

pub fn picker_list_flat<T: PickerItem>(
    entries: &[T],
    selected_index: usize,
    theme: &Theme,
) -> Div {
    picker_list_inner(entries, selected_index, 0, theme, false, false)
}

fn picker_list_inner<T: PickerItem>(
    entries: &[T],
    selected_index: usize,
    scroll_top_px: u32,
    theme: &Theme,
    scrollable: bool,
    no_scrollbar: bool,
) -> Div {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let row_h = (Sz::ROW * scale).round();

    let mut list = div()
        .flex_1()
        .min_h(0.0)
        .flex_col()
        .clip();

    if scrollable {
        list = list
            .scroll_y(scroll_top_px as f32)
            .scroll_total(entries.len() as f32 * row_h)
            .on_scroll(ScrollActionBuilder::Custom(Action::ScrollActiveOverlayListPx));
        if no_scrollbar {
            list = list.hide_scrollbar();
        }
    }

    for (i, entry) in entries.iter().enumerate() {
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
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .child(
                            text(entry.label())
                                .text_sm()
                                .color(if selected { tc.text_strong } else { tc.text })
                                .truncate(),
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
