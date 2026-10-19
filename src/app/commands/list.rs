use quark::view;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};
use quark_ui::virtual_list::{virtual_list_total_extent, virtual_list_window};

use crate::app::tokens::{self, Ico, Sp, TextStyle};

const OVERSCAN_ROWS: usize = 2;

pub fn row_stride(theme: &Theme) -> f32 {
    theme.metrics.ui_row_height.round() + tokens::scaled(Sp::XS, tokens::ui_scale(theme))
}

pub fn virtual_list(
    count: usize,
    scroll_px: f32,
    visible_rows: usize,
    theme: &Theme,
    label: &str,
    mut row: impl FnMut(usize) -> AnyElement,
    on_scroll: ScrollActionBuilder,
) -> AnyElement {
    let stride = row_stride(theme);
    let viewport = virtual_list_total_extent(count.min(visible_rows), stride, 0.0);
    let total = virtual_list_total_extent(count, stride, 0.0);
    let scroll = scroll_px.clamp(0.0, (total - viewport).max(0.0));
    let window = virtual_list_window(count, scroll, viewport, stride, 0.0, OVERSCAN_ROWS);
    let gap = stride - theme.metrics.ui_row_height.round();
    let rows: Vec<AnyElement> = window
        .range
        .clone()
        .map(|index| {
            let element = row(index);
            view! { <div class="w-full shrink-0 flex-col" h={stride} pb={gap}>{element}</div> }
        })
        .collect();
    view! {
        <div
            class="w-full flex-col overflow-hidden"
            h={viewport}
            scroll_y={scroll}
            scroll_total={total}
            on:scroll={on_scroll}
            accessibility_role={AccessibilityRole::ListBox}
            aria-label={label}
            test_id="picker-list"
        >
            <div class="w-full shrink-0" h={window.top_spacer} />
            {...rows}
            <div class="w-full shrink-0" h={window.bottom_spacer} />
        </div>
    }
}

pub struct OptionRow<'a> {
    pub id: String,
    pub label: &'a str,
    pub highlights: &'a [(usize, usize)],
    pub detail: Option<&'a str>,
    pub rhs: Option<&'a str>,
    pub icon: Option<&'static str>,
    pub selected: bool,
    pub current: bool,
}

pub fn option_row(row: OptionRow, theme: &Theme, on_click: quark_ui::Action) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    view! {
        <div
            class="w-full flex-row items-center"
            h={theme.metrics.ui_row_height.round()}
            px={tokens::scaled(Sp::SM, scale)}
            gap={tokens::scaled(Sp::SM, scale)}
            rounded={theme.metrics.control_radius}
            bg={if row.selected { tokens::selected_fill(tc.elevated_surface, true, theme) } else { Color::TRANSPARENT }}
            @when {!row.selected} { hover_bg={tc.ghost_element_hover} }
            cursor={CursorHint::Pointer}
            id={row.id.clone()}
            key={row.id.clone()}
            accessibility_role={AccessibilityRole::ListBoxOption}
            aria-label={row.label}
            aria-selected={row.selected}
            on:click={on_click}
        >
            {?row.icon.map(|svg| view! { <icon svg={svg} size={Ico::SM} color={if row.current { tc.accent } else { tc.icon }} /> })}
            {highlighted(row.label, row.highlights, if row.selected { tc.text_strong } else { tc.text }, theme)}
            {?row.detail.filter(|d| !d.is_empty()).map(|d| view! {
                <div class="flex-1 flex-row overflow-hidden" min_w={0.0}>
                    <text size={small} class="truncate" color={tc.text_muted}>{d}</text>
                </div>
            })}
            if row.detail.is_none_or(str::is_empty) { <div class="flex-1" /> }
            {?row.rhs.filter(|r| !r.is_empty()).map(|r| view! {
                <text size={small} color={tc.text_muted}>{r}</text>
            })}
            if row.current {
                <icon svg={lucide::CHECK} size={Ico::SM} color={tc.accent} />
            }
        </div>
    }
}

pub fn highlighted(
    label: &str,
    highlights: &[(usize, usize)],
    color: Color,
    theme: &Theme,
) -> AnyElement {
    let size = TextStyle::Body.font_size(theme);
    let accent = theme.colors.accent;
    let mut spans: Vec<AnyElement> = Vec::new();
    let mut cursor = 0;
    for &(start, end) in highlights {
        if start < cursor
            || start >= end
            || end > label.len()
            || !label.is_char_boundary(start)
            || !label.is_char_boundary(end)
        {
            continue;
        }
        if cursor < start {
            spans.push(view! { <text size={size} color={color}>{&label[cursor..start]}</text> });
        }
        spans.push(view! { <text size={size} class="font-semibold" color={accent}>{&label[start..end]}</text> });
        cursor = end;
    }
    if cursor < label.len() {
        spans.push(
            view! { <text size={size} class="truncate" color={color}>{&label[cursor..]}</text> },
        );
    }
    view! { <div class="flex-row items-center overflow-hidden" min_w={0.0}>{...spans}</div> }
}
