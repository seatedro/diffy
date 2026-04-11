use std::cell::Cell;
use std::rc::Rc;

use crate::render::{Rect, RectPrimitive, RoundedRectPrimitive};
use crate::actions::Action;
use crate::ui::components;
use crate::ui::design::{Alpha, Ico, Rad, Sp, Sz};
use crate::effects::Effect;
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{AppState, FocusTarget, SidebarMode, SidebarWidthCache};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

pub(crate) struct SidebarResizeDrag {
    origin_x: f32,
    starting_width: f32,
}

impl SidebarResizeDrag {
    pub fn new(origin_x: f32, starting_width: f32) -> Self {
        Self {
            origin_x,
            starting_width,
        }
    }
}

impl DragHandler for SidebarResizeDrag {
    fn on_move(&mut self, x: f32, _y: f32) -> Vec<Action> {
        let target_width = (self.starting_width + (x - self.origin_x)).round().max(0.0) as u32;
        vec![Action::SetSidebarWidthPx(target_width)]
    }

    fn on_release(&mut self, state: &crate::ui::state::AppState) -> DragReleaseResult {
        DragReleaseResult {
            actions: Vec::new(),
            effects: vec![Effect::SaveSettings(state.settings.clone())],
        }
    }

    fn cursor(&self) -> CursorHint {
        CursorHint::ResizeCol
    }
}

pub(crate) fn preferred_sidebar_width(
    state: &mut AppState,
    theme: &Theme,
    cx: &mut ElementContext,
    available_width: f32,
) -> f32 {
    let ui_scale = theme.metrics.ui_scale();
    let list_side_padding = Sp::MD * ui_scale;
    let row_side_padding = Sp::SM * 2.0 * ui_scale;
    let row_gap = Sp::SM * ui_scale;
    let stats_gap = Sp::XS * ui_scale;
    let header_side_padding = Sp::XXL + Sp::XS;
    let header_badge_outer_padding = Sp::SM * 2.0 * ui_scale;
    let header_badge_inner_padding = Sp::MD * ui_scale;
    let scrollbar_gutter = Ico::LG * ui_scale;
    let auto_min_width = theme.metrics.sidebar_width;
    let manual_min_width = (theme.metrics.sidebar_width * 0.64).round();
    let file_icon_width = Ico::XS;
    let hard_max = available_width.max(0.0);
    let max_width = if hard_max >= auto_min_width {
        (available_width - Sz::MAIN_SURFACE_MIN_W)
            .max(auto_min_width)
            .min(hard_max)
    } else {
        hard_max
    };
    if state.workspace.files.is_empty() {
        return state
            .settings
            .sidebar_width_px
            .map(|width| width as f32)
            .unwrap_or(auto_min_width)
            .clamp(0.0, hard_max.max(0.0));
    }
    if max_width <= manual_min_width {
        return max_width;
    }
    if let Some(preferred_width) = state.settings.sidebar_width_px {
        return (preferred_width as f32).clamp(manual_min_width, max_width);
    }

    let cached_intrinsic_width = state.workspace.sidebar_auto_width.and_then(|cache| {
        (cache.compare_generation == state.workspace.compare_generation
            && cache.ui_scale_pct == state.settings.ui_scale_pct)
            .then_some(cache.intrinsic_width_px)
    });

    let intrinsic_width = if let Some(width) = cached_intrinsic_width {
        width
    } else {
        let header_label_width = measure_text_width(
            cx.font_system,
            "FILES",
            theme.metrics.ui_small_font_size - 1.0,
            crate::render::FontKind::Ui,
            crate::render::FontWeight::Semibold,
        );
        let header_badge_width = if state.workspace.files.is_empty() {
            0.0
        } else {
            let count_width = measure_text_width(
                cx.font_system,
                &state.workspace.files.len().to_string(),
                theme.metrics.ui_small_font_size - 1.0,
                crate::render::FontKind::Ui,
                crate::render::FontWeight::Normal,
            );
            header_badge_outer_padding + header_badge_inner_padding + count_width
        };
        let header_width = header_side_padding + header_label_width + header_badge_width;

        let widest_row = state
            .workspace
            .files
            .iter()
            .map(|file| {
                let path_width = measure_text_width(
                    cx.font_system,
                    &file.path,
                    theme.metrics.ui_small_font_size,
                    crate::render::FontKind::Ui,
                    crate::render::FontWeight::Normal,
                );

                let stats_width = if file.additions > 0 || file.deletions > 0 {
                    let additions_width = measure_text_width(
                        cx.font_system,
                        &format!("+{}", file.additions),
                        theme.metrics.ui_small_font_size - 1.0,
                        crate::render::FontKind::Ui,
                        crate::render::FontWeight::Normal,
                    );
                    let deletions_width = measure_text_width(
                        cx.font_system,
                        &format!("\u{2212}{}", file.deletions),
                        theme.metrics.ui_small_font_size - 1.0,
                        crate::render::FontKind::Ui,
                        crate::render::FontWeight::Normal,
                    );
                    row_gap + additions_width + stats_gap + deletions_width
                } else {
                    0.0
                };

                let status_badge_width = if !file.status.is_empty() {
                    row_gap + (theme.metrics.ui_small_font_size + Sp::XS).round()
                } else {
                    0.0
                };

                list_side_padding
                    + row_side_padding
                    + file_icon_width
                    + row_gap
                    + path_width
                    + stats_width
                    + status_badge_width
                    + scrollbar_gutter
            })
            .fold(0.0_f32, f32::max);

        let intrinsic_width = widest_row.max(header_width);
        state.workspace.sidebar_auto_width = Some(SidebarWidthCache {
            compare_generation: state.workspace.compare_generation,
            ui_scale_pct: state.settings.ui_scale_pct,
            intrinsic_width_px: intrinsic_width,
        });
        intrinsic_width
    };

    intrinsic_width.clamp(auto_min_width, max_width)
}

pub(crate) fn sidebar_resizer(
    theme: &Theme,
    bounds_cell: Rc<Cell<Option<Rect>>>,
    starting_width: f32,
) -> Canvas {
    let tc = theme.colors;
    let scale = theme.metrics.ui_scale();
    let handle_width = (Sp::SM * scale).round().max(Sp::SM);
    let track_width = 1.0_f32;
    let thumb_width = (Sp::XXS * scale).round().max(2.0);
    let thumb_height = (Sp::XXL * scale).round().max(Sp::LG);

    canvas(move |bounds, scene, cx| {
        bounds_cell.set(Some(bounds));
        let hovered = cx
            .mouse_position
            .is_some_and(|(mx, my)| bounds.contains(mx, my));
        let center_x = bounds.x + bounds.width * 0.5;
        let center_y = bounds.y + bounds.height * 0.5;
        let line_color = if hovered {
            tc.border_variant
        } else {
            tc.border_variant.with_alpha(Alpha::SOFT)
        };
        let thumb_color = if hovered {
            tc.text_muted
        } else {
            tc.border_variant
        };

        let sw = starting_width;
        cx.push_click_handler(
            bounds,
            CursorHint::ResizeCol,
            ClickHandler::new(move |event| {
                ClickResult::CaptureDrag(Box::new(SidebarResizeDrag::new(event.x, sw)))
            }),
        );

        scene.rect(RectPrimitive {
            rect: Rect {
                x: center_x - track_width * 0.5,
                y: bounds.y,
                width: track_width,
                height: bounds.height,
            },
            color: line_color,
        });
        scene.rounded_rect(RoundedRectPrimitive::uniform(
            Rect {
                x: center_x - thumb_width * 0.5,
                y: center_y - thumb_height * 0.5,
                width: thumb_width,
                height: thumb_height,
            },
            thumb_width,
            thumb_color,
        ));
    })
    .w(handle_width)
}

pub(crate) fn sidebar(
    state: &AppState,
    theme: &Theme,
    sidebar_width: f32,
    _bounds_cell: Rc<Cell<Option<Rect>>>,
    cx: &ElementContext,
) -> Div {
    let tc = &theme.colors;
    let all_files = &state.workspace.files;
    let file_count = all_files.len();
    let scale = theme.metrics.ui_scale();
    let filter = &state.file_list.filter;
    let has_filter = !filter.is_empty();
    let is_tree = state.file_list.mode == SidebarMode::TreeView;

    let filtered_indices: Vec<usize> = if has_filter {
        let haystack: Vec<&str> = all_files.iter().map(|f| f.path.as_str()).collect();
        let config = neo_frizbee::Config {
            max_typos: Some(2),
            sort: false,
            ..Default::default()
        };
        let mut matches = neo_frizbee::match_list(filter, &haystack, &config);
        matches.sort_by(|a, b| b.score.cmp(&a.score));
        matches.iter().map(|m| m.index as usize).collect()
    } else {
        (0..file_count).collect()
    };
    let visible_count = filtered_indices.len();

    let total_adds: i32 = all_files.iter().map(|f| f.additions).sum();
    let total_dels: i32 = all_files.iter().map(|f| f.deletions).sum();

    let row_h = theme.metrics.ui_row_height.round();

    let header = div()
        .px((Sp::MD * scale).round())
        .flex_col()
        .child(
            div()
                .h(row_h)
                .flex_row()
                .items_center()
                .gap(Sp::SM * scale)
                .child(text("FILES").text_xs().semibold().color(tc.text_muted))
                .optional_child(if file_count > 0 {
                    Some(
                        div()
                            .px((Rad::LG * scale).round())
                            .py((Sp::XXS * scale).round())
                            .rounded((Rad::LG * scale).round())
                            .bg(Color::rgba(255, 255, 255, 10))
                            .child(text(file_count.to_string()).text_xs().color(tc.text_muted)),
                    )
                } else {
                    None
                })
                .child(spacer())
                .optional_child(if file_count > 0 {
                    let mode_icon = if is_tree {
                        lucide::ROWS
                    } else {
                        lucide::FOLDER
                    };
                    Some(
                        div()
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .w((Sz::MODE_TOGGLE * scale).round())
                            .h((Sz::MODE_TOGGLE * scale).round())
                            .rounded((Rad::SM * scale).round())
                            .hover_bg(tc.ghost_element_hover)
                            .on_click(Action::ToggleSidebarMode)
                            .child(svg_icon(mode_icon, Ico::SIDEBAR_MODE).color(tc.text_muted)),
                    )
                } else {
                    None
                }),
        )
        .optional_child(if file_count > 0 {
            Some(
                div()
                    .h(row_h)
                    .flex_row()
                    .items_center()
                    .gap(Sp::XS * scale)
                    .child(
                        components::stat_summary(
                            file_count,
                            total_adds.unsigned_abs(),
                            total_dels.unsigned_abs(),
                        )
                        .compact(),
                    ),
            )
        } else {
            None
        });

    let search_bar = if file_count > 0 {
        let search_focused = cx.is_focused(FocusTarget::SidebarSearch);
        let input = text_input("", &state.file_list.filter)
            .placeholder("Filter files\u{2026}")
            .focused(search_focused)
            .focus_target(FocusTarget::SidebarSearch)
            .cursor(state.text_edit.cursor)
            .anchor(state.text_edit.anchor)
            .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
            .on_click(Action::SetFocus(Some(FocusTarget::SidebarSearch)))
            .bare()
            .w_full()
            .h(theme.metrics.ui_row_height.round());
        let hint = if !search_focused && !has_filter {
            Some("/")
        } else {
            None
        };
        Some(
            div()
                .w_full()
                .px((Sp::SM + Sp::XXS) * scale)
                .pb((Sp::SM * scale).round())
                .child(components::search_field(
                    input,
                    has_filter,
                    Some(Action::ClearSidebarFilter),
                    hint,
                    theme,
                )),
        )
    } else {
        None
    };

    let mut sidebar_div = div()
        .flex_col()
        .w(sidebar_width)
        .flex_shrink_0()
        .h_full()
        .min_h(0.0)
        .bg(tc.sidebar_background)
        .border_r(tc.border_variant)
        .child(header)
        .optional_child(search_bar);

    if all_files.is_empty() {
        let (icon, msg) = if state.compare.repo_path.is_some() {
            (lucide::GIT_COMPARE, "Run a compare to see changes.")
        } else {
            (lucide::FOLDER_OPEN, "Open a repository to start.")
        };
        sidebar_div = sidebar_div.child(
            div().flex_1().items_center().justify_center().child(
                div()
                    .flex_col()
                    .items_center()
                    .gap((Sp::SM * scale).round())
                    .child(svg_icon(icon, Ico::XL).color(tc.text_muted))
                    .child(text(msg).text_sm().color(tc.text_muted)),
            ),
        );
    } else if visible_count == 0 && has_filter {
        sidebar_div = sidebar_div.child(
            div().flex_1().items_center().justify_center().child(
                div()
                    .flex_col()
                    .items_center()
                    .gap((Sp::SM * scale).round())
                    .child(svg_icon(lucide::SEARCH, Ico::XL).color(tc.text_muted))
                    .child(
                        text("No files match filter.")
                            .text_sm()
                            .color(tc.text_muted),
                    ),
            ),
        );
    } else if is_tree && !has_filter {
        let entries: Vec<components::FileTreeEntry> = filtered_indices
            .iter()
            .map(|&i| {
                let f = &all_files[i];
                components::FileTreeEntry {
                    path: f.path.clone(),
                    status: f.status.clone(),
                    additions: f.additions,
                    deletions: f.deletions,
                }
            })
            .collect();

        let tree = components::file_tree(entries)
            .expanded(state.file_list.expanded_folders.clone())
            .selected(state.workspace.selected_file_index)
            .on_select_file(Action::SelectFile)
            .on_toggle_folder(Action::ToggleFolder);

        let row_count = visible_count + state.file_list.expanded_folders.len();
        let row_height = state.file_list.row_height;
        let total_height = row_count as f32 * (row_height + state.file_list.gap);
        let scroll_px = state.file_list.scroll_offset_px;

        sidebar_div = sidebar_div.child(
            div()
                .flex_1()
                .min_h(0.0)
                .flex_col()
                .clip()
                .scroll_y(scroll_px)
                .scroll_total(total_height)
                .on_scroll(ScrollActionBuilder::FileList)
                .child(tree),
        );
    } else {
        let row_height = state.file_list.row_height;
        let total_height = state.file_list.total_content_height(visible_count);
        let scroll_px = state.file_list.scroll_offset_px;

        let mut list = div()
            .flex_1()
            .min_h(0.0)
            .flex_col()
            .px((Rad::LG * scale).round())
            .pt((Sp::XXS * scale).round())
            .gap((Sp::XS * scale).round())
            .clip()
            .scroll_y(scroll_px)
            .scroll_total(total_height)
            .on_scroll(ScrollActionBuilder::FileList);

        for &index in &filtered_indices {
            let file = &all_files[index];
            let selected = state.workspace.selected_file_index == Some(index);
            let viewed = state.file_list.viewed_files.contains(&index);
            let text_color = if selected { tc.text_strong } else { tc.text };

            let mut row = div()
                .w_full()
                .h(row_height)
                .flex_row()
                .items_center()
                .px(Sp::SM * scale)
                .gap(Sp::SM * scale)
                .on_click(Action::SelectFile(index))
                .cursor(CursorHint::Pointer);

            if selected {
                row = row.bg(tc.sidebar_row_selected).border_l(tc.accent);
            } else {
                row = row.hover_bg(tc.sidebar_row_hover);
            }

            row = row.child(components::file_icon(&file.path, Ico::XS).selected(selected));

            let (filename, dir_path) = match file.path.rfind('/') {
                Some(pos) => (&file.path[pos + 1..], Some(&file.path[..pos])),
                None => (file.path.as_str(), None),
            };

            row = row.child(
                div()
                    .flex_1()
                    .flex_row()
                    .items_center()
                    .gap(Sp::SM * scale)
                    .overflow_hidden()
                    .min_w(0.0)
                    .child(
                        div()
                            .flex_shrink_0()
                            .child(text(filename).text_sm().color(text_color)),
                    )
                    .optional_child(
                        dir_path.map(|p| text(p).text_xs().color(tc.text_muted).truncate()),
                    ),
            );

            if file.additions > 0 || file.deletions > 0 {
                row = row.child(
                    div()
                        .flex_row()
                        .gap(Sp::XS * scale)
                        .flex_shrink_0()
                        .child(
                            text(format!("+{}", file.additions))
                                .text_xs()
                                .color(tc.line_add_text),
                        )
                        .child(
                            text(format!("\u{2212}{}", file.deletions))
                                .text_xs()
                                .color(tc.line_del_text),
                        ),
                );
            }

            if !file.status.is_empty() {
                row = row.child(components::status_badge(&file.status));
            }

            if viewed {
                row = row.child(svg_icon(lucide::CHECK, Ico::XS).color(tc.line_add_text));
            }

            list = list.child(row);
        }

        sidebar_div = sidebar_div.child(list);
    }

    sidebar_div
}
