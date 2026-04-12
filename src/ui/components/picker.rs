use halogen::view;

use crate::actions::Action;
use crate::ui::design::{Ico, Rad, Sp};
use crate::ui::element::*;
use crate::ui::shell::CursorHint;
use crate::ui::state::PickerItem;
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

pub fn picker_list<T: PickerItem>(
    entries: &[T],
    selected_index: usize,
    scroll_top_px: f32,
    max_visible: usize,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let row_h = theme.metrics.ui_row_height.round();
    let gap = (Sp::XS * scale).round();
    let icon_size = (Ico::XS * scale).round();
    let stride = row_h + gap;
    let visible_count = entries.len().min(max_visible);
    let list_h = if visible_count == 0 {
        0.0
    } else {
        visible_count as f32 * stride - gap
    };
    let total_h = if entries.is_empty() {
        0.0
    } else {
        entries.len() as f32 * stride - gap
    };
    let scroll = scroll_top_px.min((total_h - list_h).max(0.0));

    view! { scale,
        <div class="w-full flex-col" gap={Sp::XS} h={list_h}
             overflow_hidden scroll_y={scroll} scroll_total={total_h}
             on_scroll={ScrollActionBuilder::Custom(Action::ScrollActiveOverlayListPx)}
             hide_scrollbar>
            for (i, entry) in entries.iter().enumerate() {
                if entry.is_section_header() {
                    <div class="w-full flex-row items-center" h={row_h} px={Sp::MD}>
                        <text class="text-xs truncate" color={tc.text_muted}>{entry.label()}</text>
                    </div>
                } else {
                    {picker_row(i, entry, selected_index, row_h, icon_size, theme)}
                }
            }
        </div>
    }
}

fn picker_row<T: PickerItem>(
    i: usize,
    entry: &T,
    selected_index: usize,
    row_h: f32,
    icon_size: f32,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let selected = i == selected_index;
    let row_bg = if selected {
        tc.sidebar_row_selected
    } else {
        Color::TRANSPARENT
    };
    let icon_child = entry
        .icon_svg()
        .map(|svg| svg_icon(svg, icon_size).color(tc.icon));
    let detail_child = entry
        .detail()
        .filter(|d| !d.is_empty())
        .map(|d| text(d).text_xs().color(tc.text_muted).truncate());
    view! { scale,
        <div class="w-full shrink-0 flex-row items-center"
             h={row_h} gap={Sp::SM} px={Sp::MD} rounded={Rad::MD}
             bg={row_bg}
             @when {!selected} { hover_bg={tc.sidebar_row_hover} }
             on_click={Action::SelectOverlayEntry(i)}
             cursor={CursorHint::Pointer}>
            {?icon_child}
            {picker_label(entry.label(), entry.highlight_ranges(), selected, theme)}
            {?detail_child}
        </div>
    }
}

fn picker_label(
    label_text: &str,
    highlights: &[(usize, usize)],
    selected: bool,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let base_color = if selected { tc.text_strong } else { tc.text };

    let container = div().flex_1().overflow_hidden();

    if highlights.is_empty() {
        return container
            .child(text(label_text).text_sm().color(base_color).truncate())
            .into_any();
    }

    let mut row = div().flex_row().overflow_hidden();
    let mut cursor = 0;
    for &(start, end) in highlights {
        if start >= end || end > label_text.len() {
            continue;
        }
        if cursor < start {
            row = row.child(text(&label_text[cursor..start]).text_sm().color(base_color));
        }
        row = row.child(text(&label_text[start..end]).text_sm().color(tc.accent));
        cursor = end;
    }
    if cursor < label_text.len() {
        row = row.child(text(&label_text[cursor..]).text_sm().color(base_color));
    }

    container.child(row).into_any()
}
