use halogen::{SemanticRole, view};

use crate::ui::design::{Ico, Rad, Sp};
use crate::ui::element::*;
use crate::ui::shell::CursorHint;
use crate::ui::state::{PickerItem, PickerLabelStyle};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme, ThemeColors};

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
    let icon_size = Ico::XS;
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
             id={"picker-list"}
             test_id={"picker-list"}
             semantic_role={SemanticRole::ScrollArea}
             overflow_hidden scroll_y={scroll} scroll_total={total_h}
             on_scroll={ScrollActionBuilder::Custom(crate::actions::scroll_active_overlay_list_px)}
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
    let rhs_child = entry
        .rhs()
        .filter(|d| !d.is_empty())
        .map(|d| text(d).text_xs().color(tc.text_muted));
    let disabled = entry.is_disabled();
    view! { scale,
        <div class="w-full shrink-0 flex-row items-center"
             id={format!("picker-row:{i}:{}", entry.label())}
             key={format!("{i}:{}", entry.label())}
             test_id={"picker-row"}
             semantic_role={SemanticRole::ListBoxOption}
             h={row_h} gap={Sp::SM} px={Sp::MD} rounded={Rad::MD}
             bg={row_bg}
             @when {!selected && !disabled} { hover_bg={tc.sidebar_row_hover} }
             on_click={crate::actions::OverlayAction::SelectOverlayEntry(i).into()}
             hit_identity={HitIdentity::OverlayEntry(i)}
             accessibility_role={accesskit::Role::ListBoxOption}
             accessibility_id={format!("picker-row:{i}:{}", entry.label())}
             accessibility_label={entry.label()}
             accessibility_selected={selected}
             accessibility_disabled={disabled}
             cursor={CursorHint::Pointer}>
            {?icon_child}
            {picker_label(
                entry.label(),
                entry.label_style(),
                entry.highlight_ranges(),
                selected,
                theme
            )}
            {?detail_child}
            {?rhs_child}
        </div>
    }
}

fn picker_label(
    label_text: &str,
    label_style: PickerLabelStyle,
    highlights: &[(usize, usize)],
    selected: bool,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let base_color = if selected { tc.text_strong } else { tc.text };

    if let PickerLabelStyle::JjChangeId {
        prefix_len,
        working_copy,
    } = label_style
    {
        return jj_change_id_label(label_text, prefix_len, working_copy, highlights, tc);
    }

    if highlights.is_empty() {
        return view! {
            <div class="flex-1 overflow-hidden">
                <text class="text-sm truncate" color={base_color}>{label_text}</text>
            </div>
        };
    }

    let mut spans = Vec::new();
    let mut cursor = 0;
    for &(start, end) in highlights {
        if start >= end || end > label_text.len() {
            continue;
        }
        if cursor < start {
            spans.push((&label_text[cursor..start], base_color));
        }
        spans.push((&label_text[start..end], tc.accent));
        cursor = end;
    }
    if cursor < label_text.len() {
        spans.push((&label_text[cursor..], base_color));
    }

    view! {
        <div class="flex-1 overflow-hidden">
            <div class="flex-row overflow-hidden">
                for (segment, color) in spans {
                    <text class="text-sm" color={color}>{segment}</text>
                }
            </div>
        </div>
    }
}

fn jj_change_id_label(
    label_text: &str,
    prefix_len: usize,
    working_copy: bool,
    highlights: &[(usize, usize)],
    tc: &ThemeColors,
) -> AnyElement {
    let split = prefix_len.min(label_text.len());
    let split = if label_text.is_char_boundary(split) {
        split
    } else {
        0
    };
    let (prefix, rest) = label_text.split_at(split);
    let prefix_color = if working_copy {
        tc.syntax_keyword.lerp(tc.text_strong, 0.28)
    } else {
        tc.syntax_keyword
    };

    if highlights.is_empty() {
        return view! {
            <div class="flex-1 overflow-hidden">
                <div class="flex-row overflow-hidden">
                    {text(prefix).text_sm().bold().color(prefix_color)}
                    {text(rest).text_sm().color(tc.text_muted)}
                </div>
            </div>
        };
    }

    let mut spans: Vec<AnyElement> = Vec::new();
    let mut cursor = 0;
    let mut push_segment = |start: usize, end: usize, highlighted: bool| {
        if start >= end {
            return;
        }
        let mut segment_start = start;
        let color = if highlighted { tc.accent } else { prefix_color };
        if segment_start < split {
            let segment_end = end.min(split);
            spans.push(
                text(&label_text[segment_start..segment_end])
                    .text_sm()
                    .bold()
                    .color(color)
                    .into_any(),
            );
            segment_start = segment_end;
        }
        if segment_start < end {
            let color = if highlighted {
                tc.accent
            } else {
                tc.text_muted
            };
            spans.push(
                text(&label_text[segment_start..end])
                    .text_sm()
                    .color(color)
                    .into_any(),
            );
        }
    };

    for &(start, end) in highlights {
        if start >= end || end > label_text.len() {
            continue;
        }
        if cursor < start {
            push_segment(cursor, start, false);
        }
        push_segment(start, end, true);
        cursor = end;
    }
    if cursor < label_text.len() {
        push_segment(cursor, label_text.len(), false);
    }

    view! {
        <div class="flex-1 overflow-hidden">
            <div class="flex-row overflow-hidden">
                for span in spans {
                    {span}
                }
            </div>
        </div>
    }
}
