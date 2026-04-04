use std::cell::Cell;
use std::rc::Rc;

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::render::{
    Rect, RectPrimitive, RoundedRectPrimitive, Scene, ShadowPrimitive, TextMetrics,
};
use crate::ui::actions::Action;
use crate::ui::components::{
    self, Button, ButtonStyle, SegmentedControl, SegmentedItem, ToastStack,
};
use crate::ui::design::{Ico, Rad, Sp, Sz};
use crate::ui::editor::element::{EditorDocument, EditorElement};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::overlays;
use crate::ui::state::{
    AppState, AsyncStatus, FocusTarget, OverlaySurface,
    SidebarMode, SidebarWidthCache, WorkspaceMode,
};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

// ---------------------------------------------------------------------------
// Public frame types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorHint {
    #[default]
    Default,
    Pointer,
    Text,
    ResizeCol,
}

#[derive(Debug, Clone, Default)]
pub struct UiFrame {
    pub scene: Scene,
    pub hits: Vec<HitRegion>,
    pub scroll_regions: Vec<ScrollRegion>,
    pub text_input_hit_areas: Vec<TextInputHitArea>,
    pub scrollbar_tracks: Vec<ScrollbarTrack>,
    pub file_list_rect: Option<Rect>,
    pub sidebar_resize_handle_rect: Option<Rect>,
    pub viewport_rect: Option<Rect>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn build_ui_frame(
    state: &mut AppState,
    theme: &Theme,
    editor: &mut EditorElement,
    text_metrics: TextMetrics,
    width: f32,
    height: f32,
    cx: &mut ElementContext,
) -> UiFrame {
    let viewport_bounds: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));
    let file_list_bounds: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));
    let sidebar_resize_bounds: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));
    let ui_scale = ui_scale(theme);

    let sidebar_list_height =
        (height - theme.metrics.title_bar_height - theme.metrics.status_bar_height - Sz::SIDEBAR_LIST_OFFSET * ui_scale).max(0.0);
    state.file_list.row_height = (Sz::ROW * ui_scale).round();
    state.file_list.gap = (Sp::XS * ui_scale).round();
    let overlay_row_height = (Sz::ROW * ui_scale).round().max(24.0) as u32;
    state.overlays.picker.list.row_height_px = overlay_row_height;
    state.overlays.command_palette.list.row_height_px = overlay_row_height;
    state.file_list.viewport_height = sidebar_list_height;
    state.file_list.clamp_scroll(state.workspace.files.len());
    let sidebar_width_factor = cx
        .ui_signals
        .map(|s| cx.read(s.sidebar_width_factor))
        .unwrap_or(1.0);
    let sidebar_width = preferred_sidebar_width(state, theme, cx, width) * sidebar_width_factor;

    let mut root = div()
        .w(width)
        .h(height)
        .flex_col()
        .bg(theme.colors.background)
        .child(title_bar(state, theme, sidebar_width_factor))
        .child(
            div()
                .flex_row()
                .flex_1()
                .min_h(0.0)
                .when(sidebar_width_factor > 0.001, |d| {
                    d.child(sidebar(
                        state,
                        theme,
                        sidebar_width,
                        file_list_bounds.clone(),
                        cx,
                    ))
                    .child(sidebar_resizer(theme, sidebar_resize_bounds.clone()))
                })
                .child(main_surface(
                    state,
                    theme,
                    text_metrics,
                    viewport_bounds.clone(),
                )),
        )
        .child(status_bar(state, theme));

    if let Some(top) = state.overlays.stack.last().cloned() {
        let overlay = match top.surface {
            OverlaySurface::CompareSheet => overlays::compare_sheet(state, theme, width, height),
            OverlaySurface::RepoPicker => overlays::repo_picker(state, theme, width, height),
            OverlaySurface::RefPicker(field) => overlays::ref_picker(state, theme, field, width, height),
            OverlaySurface::CommandPalette => overlays::command_palette(state, theme, width, height),
            OverlaySurface::PullRequestModal => overlays::pull_request_modal(state, theme, width, height),
            OverlaySurface::GitHubAuthModal => overlays::auth_modal(state, theme, width, height),
        };
        root = root.child(overlay);
    }

    let mut root = root.into_any();

    // --- Render element tree ---
    let mut scene = Scene::default();
    render_element(&mut root, &mut scene, cx, width, height);
    let mut scrollbar_tracks = std::mem::take(&mut cx.scrollbar_tracks);

    // --- Viewport content (painted after element tree, clipped to bounds) ---
    if state.workspace_mode == WorkspaceMode::Ready {
        if let Some(vp_bounds) = viewport_bounds.get() {
            let document = match state.workspace.active_file.as_ref() {
                Some(active_file) if active_file.file.is_binary => EditorDocument::Binary {
                    path: &active_file.path,
                },
                Some(active_file) => EditorDocument::Text {
                    compare_generation: state.workspace.compare_generation,
                    file_index: active_file.index,
                    path: &active_file.path,
                    doc: &active_file.render_doc,
                },
                None => EditorDocument::Empty,
            };
            editor.prepare(&mut state.editor, document, vp_bounds, text_metrics);
            scene.clip(vp_bounds);
            editor.paint(&mut scene, theme, &state.editor, document);
            scene.pop_clip();

            // Register viewport scrollbar for drag support
            if state.editor.content_height_px > state.editor.viewport_height_px
                && state.editor.viewport_height_px > 0
            {
                let sb = editor.scrollbar_rect();
                let ratio = state.editor.viewport_height_px as f32
                    / state.editor.content_height_px as f32;
                let thumb_h = (sb.height * ratio).max(Sp::XXL * ui_scale).min(sb.height);
                let scroll_range = state.editor.max_scroll_top_px().max(1) as f32;
                let top_ratio = state.editor.scroll_top_px as f32 / scroll_range;
                let thumb_y = sb.y + (sb.height - thumb_h) * top_ratio;
                scrollbar_tracks.push(ScrollbarTrack {
                    track_rect: Rect {
                        x: sb.x - Rad::LG * ui_scale,
                        y: sb.y,
                        width: sb.width + Sp::MD * ui_scale,
                        height: sb.height,
                    },
                    thumb_top: thumb_y,
                    thumb_height: thumb_h,
                    content_height: state.editor.content_height_px as f32,
                    viewport_height: state.editor.viewport_height_px as f32,
                    action_builder: ScrollActionBuilder::ViewportLines,
                });
            }
        }
    }

    if !state.toasts.is_empty() {
        let mut toast_root = ToastStack::new(&state.toasts, width, height).build().into_any();
        render_element(&mut toast_root, &mut scene, cx, width, height);
    }

    let hits = std::mem::take(&mut cx.hits);
    let scroll_regions = std::mem::take(&mut cx.scroll_regions);
    let text_input_hit_areas = std::mem::take(&mut cx.text_input_hit_areas);
    let file_list_rect = scroll_regions.iter().find_map(|region| {
        matches!(region.action_builder, ScrollActionBuilder::FileList).then_some(region.bounds)
    });

    UiFrame {
        scene,
        hits,
        scroll_regions,
        text_input_hit_areas,
        scrollbar_tracks,
        file_list_rect: file_list_rect.or_else(|| file_list_bounds.get()),
        sidebar_resize_handle_rect: sidebar_resize_bounds.get(),
        viewport_rect: viewport_bounds.get(),
    }
}

// ---------------------------------------------------------------------------
// Title bar
// ---------------------------------------------------------------------------

fn title_bar(state: &AppState, theme: &Theme, sidebar_visible: f32) -> Div {
    let tc = &theme.colors;

    let repo_label = state
        .compare
        .repo_path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("diffy");

    let left = div()
        .flex_row()
        .flex_shrink_0()
        .min_w(0.0)
        .items_center()
        .gap(Sp::SM)
        .child(
            Button::new(Action::ToggleSidebar)
                .icon(lucide::PANEL_LEFT)
                .active(sidebar_visible > 0.5),
        )
        .child(svg_icon(lucide::GIT_COMPARE, Ico::LG).color(tc.accent))
        .child(
            div()
                .min_w(0.0)
                .child(text(repo_label).semibold().color(tc.text_strong).truncate()),
        );

    let center_text = if state.workspace_mode == WorkspaceMode::Ready {
        format!(
            "{} files  \u{00b7}  {} \u{2192} {}",
            state.workspace.files.len(),
            display_ref(
                state
                    .compare
                    .resolved_left
                    .as_deref()
                    .unwrap_or(&state.compare.left_ref)
            ),
            display_ref(
                state
                    .compare
                    .resolved_right
                    .as_deref()
                    .unwrap_or(&state.compare.right_ref)
            ),
        )
    } else if state.workspace_mode == WorkspaceMode::Loading {
        "Comparing\u{2026}".to_owned()
    } else {
        String::new()
    };
    let center = div().flex_1().min_w(0.0).px_4().optional_child(
        (!center_text.is_empty())
            .then_some(text(center_text).text_sm().color(tc.text_muted).truncate()),
    );

    let compare_active = state.overlays.top() == Some(OverlaySurface::CompareSheet);
    let pr_active = state.overlays.top() == Some(OverlaySurface::PullRequestModal);

    let right = div()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            Button::new(Action::OpenCompareSheet)
                .icon(lucide::GIT_COMPARE)
                .label("Compare")
                .active(compare_active),
        )
        .child(
            Button::new(Action::OpenPullRequestModal)
                .icon(lucide::GIT_PULL_REQUEST)
                .label("PR")
                .active(pr_active),
        )
        .child(toolbar_separator(tc))
        .child(SegmentedControl::new(vec![
            SegmentedItem::new(
                "Split",
                Action::SetLayoutMode(LayoutMode::Split),
                state.compare.layout == LayoutMode::Split,
            ),
            SegmentedItem::new(
                "Unified",
                Action::SetLayoutMode(LayoutMode::Unified),
                state.compare.layout == LayoutMode::Unified,
            ),
        ]))
        .child(
            Button::new(Action::ToggleWrap)
                .icon(lucide::WRAP_TEXT)
                .label("Wrap")
                .active(state.editor.wrap_enabled),
        )
        .child(Button::new(Action::ToggleThemeMode).icon(
            if theme.mode == crate::ui::theme::ThemeMode::Dark {
                lucide::MOON
            } else {
                lucide::SUN
            },
        ));

    div()
        .flex_row()
        .items_center()
        .min_w(0.0)
        .h(theme.metrics.title_bar_height)
        .w_full()
        .px(Sp::XL)
        .bg(tc.title_bar_background)
        .border_b(tc.border_variant)
        .child(left)
        .child(center)
        .child(div().flex_1().min_w(0.0))
        .child(right)
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

fn ui_scale(theme: &Theme) -> f32 {
    (theme.metrics.ui_font_size / 16.0).max(0.7)
}

fn preferred_sidebar_width(
    state: &mut AppState,
    theme: &Theme,
    cx: &mut ElementContext,
    available_width: f32,
) -> f32 {
    const MAIN_SURFACE_MIN_WIDTH: f32 = 320.0;
    let ui_scale = ui_scale(theme);
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
    let file_icon_width = Ico::MD * ui_scale;
    let hard_max = available_width.max(0.0);
    let max_width = if hard_max >= auto_min_width {
        (available_width - MAIN_SURFACE_MIN_WIDTH)
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
                    row_gap + (theme.metrics.ui_small_font_size + 4.0).round()
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

fn sidebar_resizer(theme: &Theme, bounds_cell: Rc<Cell<Option<Rect>>>) -> Canvas {
    let tc = theme.colors;
    let scale = ui_scale(theme);
    let handle_width = (Ico::LG * scale).round().max(Ico::SM);
    let track_width = (Sz::SEPARATOR_W * scale).max(1.0);
    let thumb_width = (Rad::LG * scale).round().max(Rad::MD);
    let thumb_height = (Sp::XXXXL * scale).round().max(Sz::SIDEBAR_LIST_OFFSET);

    canvas(move |bounds, scene, cx| {
        bounds_cell.set(Some(bounds));
        let hovered = cx
            .mouse_position
            .is_some_and(|(mx, my)| bounds.contains(mx, my));
        let center_x = bounds.x + bounds.width * 0.5;
        let center_y = bounds.y + bounds.height * 0.5;
        let line_color = if hovered {
            tc.accent.with_alpha(100)
        } else {
            tc.border_variant.with_alpha(120)
        };
        let glow = if hovered {
            tc.accent.with_alpha(80)
        } else {
            tc.accent.with_alpha(28)
        };
        let thumb_color = if hovered {
            Color::rgba(255, 255, 255, 210)
        } else {
            tc.scrollbar_thumb.with_alpha(180)
        };

        scene.rect(RectPrimitive {
            rect: Rect {
                x: center_x - track_width * 0.5,
                y: bounds.y + handle_width,
                width: track_width,
                height: (bounds.height - handle_width * 2.0).max(0.0),
            },
            color: line_color,
        });
        scene.shadow(ShadowPrimitive {
            rect: Rect {
                x: center_x - thumb_width * 0.5,
                y: center_y - thumb_height * 0.5,
                width: thumb_width,
                height: thumb_height,
            },
            blur_radius: handle_width,
            corner_radius: thumb_width,
            offset: [0.0, 0.0],
            color: glow,
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

fn sidebar(
    state: &AppState,
    theme: &Theme,
    sidebar_width: f32,
    _bounds_cell: Rc<Cell<Option<Rect>>>,
    cx: &ElementContext,
) -> Div {
    let tc = &theme.colors;
    let all_files = &state.workspace.files;
    let file_count = all_files.len();
    let scale = ui_scale(theme);
    let filter = &state.file_list.filter;
    let has_filter = !filter.is_empty();
    let is_tree = state.file_list.mode == SidebarMode::TreeView;

    let filtered_indices: Vec<usize> = if has_filter {
        let lower = filter.to_lowercase();
        all_files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.path.to_lowercase().contains(&lower))
            .map(|(i, _)| i)
            .collect()
    } else {
        (0..file_count).collect()
    };
    let visible_count = filtered_indices.len();

    let total_adds: i32 = all_files.iter().map(|f| f.additions).sum();
    let total_dels: i32 = all_files.iter().map(|f| f.deletions).sum();

    let header = div()
        .px((Sp::MD * scale).round())
        .pt((Sp::MD * scale).round())
        .pb((Sp::SM * scale).round())
        .flex_col()
        .gap(Sp::SM * scale)
        .child(
            div()
                .flex_row()
                .items_center()
                .gap(Sp::SM * scale)
                .child(text("FILES").text_xs().semibold().color(tc.text_muted))
                .optional_child(if file_count > 0 {
                    Some(
                        div()
                            .px((Rad::LG * scale).round())
                            .py((Sp::XXS * scale).round())
                            .rounded_sm()
                            .bg(Color::rgba(255, 255, 255, 10))
                            .child(
                                text(file_count.to_string())
                                    .text_xs()
                                    .color(tc.text_muted),
                            ),
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
                            .child(
                                svg_icon(mode_icon, Ico::SIDEBAR_MODE).color(tc.text_muted),
                            ),
                    )
                } else {
                    None
                }),
        )
        .optional_child(if file_count > 0 {
            Some(
                div()
                    .flex_row()
                    .items_center()
                    .gap(Sp::XS * scale)
                    .child(components::stat_summary(
                        file_count,
                        total_adds.unsigned_abs(),
                        total_dels.unsigned_abs(),
                    ).compact()),
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
            .h((Sz::SEARCH_INPUT * scale).round());
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
                    .gap_2()
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
                    .gap_2()
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
        let total_height =
            row_count as f32 * (row_height + state.file_list.gap);
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
            .gap((Sp::XS * scale).round())
            .clip()
            .scroll_y(scroll_px)
            .scroll_total(total_height)
            .on_scroll(ScrollActionBuilder::FileList);

        for &index in &filtered_indices {
            let file = &all_files[index];
            let selected = state.workspace.selected_file_index == Some(index);
            let viewed = state.file_list.viewed_files.contains(&index);
            let icon_color = if selected {
                tc.text_accent
            } else {
                tc.text_muted
            };
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

            row = row.child(components::file_icon(&file.path, Ico::MD * scale).selected(selected));

            row = row.child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(text(&file.path).text_sm().color(text_color).truncate()),
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
                row = row.child(
                    svg_icon(lucide::CHECK, Ico::XS).color(tc.line_add_text),
                );
            }

            list = list.child(row);
        }

        sidebar_div = sidebar_div.child(list);
    }

    sidebar_div
}

// ---------------------------------------------------------------------------
// Main surface
// ---------------------------------------------------------------------------

fn main_surface(
    state: &AppState,
    theme: &Theme,
    _text_metrics: TextMetrics,
    viewport_bounds: Rc<Cell<Option<Rect>>>,
) -> Div {
    let tc = &theme.colors;
    let mut main = div()
        .flex_1()
        .flex_col()
        .h_full()
        .min_h(0.0)
        .bg(tc.editor_surface);

    let has_overlay = state.active_overlay_name().is_some();
    match state.workspace_mode {
        WorkspaceMode::Ready => {
            let file_label = state
                .workspace
                .selected_file_path
                .as_deref()
                .unwrap_or("No file selected");

            // File header bar
            main = main.child(
                div()
                    .h(Sz::ROW)
                    .px_4()
                    .flex_row()
                    .items_center()
                    .border_b(tc.border_variant)
                    .child(components::file_icon(file_label, Ico::SM))
                    .child(div().w(Sp::SM))
                    .child(text(file_label).text_sm().color(tc.text_muted).truncate()),
            );

            // Viewport canvas
            let vb = viewport_bounds.clone();
            main = main.child(
                canvas(move |bounds, _scene, _cx| {
                    vb.set(Some(bounds));
                })
                .flex_1(),
            );
        }
        WorkspaceMode::Loading => {
            main = main.child(loading_card(state, theme));
        }
        WorkspaceMode::Empty if !has_overlay => {
            main = main.child(empty_state(state, theme));
        }
        WorkspaceMode::Empty => {}
    }

    main
}

fn loading_card(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let scale = ui_scale(theme);
    div()
        .flex_1()
        .items_center()
        .justify_center()
        .p(Sp::XL * scale)
        .child(
            div()
                .w_full()
                .max_w(Sz::CARD_SM * scale)
                .p(Sp::XL * scale)
                .flex_col()
                .gap(Sp::MD * scale)
                .items_center()
                .bg(tc.elevated_surface)
                .rounded_xl()
                .border_b(tc.border)
                .shadow(16.0, 6.0, Color::rgba(0, 0, 0, 80))
                .shadow(4.0, 2.0, Color::rgba(0, 0, 0, 40))
                .child(svg_icon(lucide::LOADER, Ico::XXL).color(tc.text_muted))
                .child(
                    div().w_full().min_w(0.0).child(
                        text("Comparing repository\u{2026}")
                            .semibold()
                            .text_center()
                            .color(tc.text_strong)
                            .truncate(),
                    ),
                )
                .child(
                    div().w_full().min_w(0.0).child(
                        text(format!(
                            "{} \u{2022} {} \u{2192} {}",
                            compare_mode_label(state.compare.mode),
                            display_ref(&state.compare.left_ref),
                            display_ref(&state.compare.right_ref)
                        ))
                        .text_sm()
                        .text_center()
                        .color(tc.text_muted)
                        .truncate(),
                    ),
                ),
        )
}

fn empty_state(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let has_repo = state.compare.repo_path.is_some();
    let scale = ui_scale(theme);

    let (title, subtitle) = if has_repo {
        (
            "Ready to compare",
            "Use the compare sheet, PR modal, or command palette to build a diff.",
        )
    } else {
        (
            "Start a new compare",
            "Choose a repository, select refs, then open the native diff workspace.",
        )
    };

    let hero_icon = if has_repo {
        lucide::GIT_COMPARE
    } else {
        lucide::GIT_BRANCH
    };

    let mut card = div()
        .w_full()
        .max_w(Sz::CARD_MD * scale)
        .p(Sp::XXL * scale)
        .flex_col()
        .gap(Sp::LG * scale)
        .bg(tc.elevated_surface)
        .rounded_xl()
        .border_b(tc.border)
        .shadow(20.0, 8.0, Color::rgba(0, 0, 0, 80))
        .shadow(4.0, 2.0, Color::rgba(0, 0, 0, 40))
        // Hero icon
        .child(svg_icon(hero_icon, Ico::HERO).color(tc.accent))
        // Heading
        .child(text(title).text_lg().semibold().color(tc.text_strong))
        // Subtitle
        .child(
            div()
                .w_full()
                .min_w(0.0)
                .child(text(subtitle).text_sm().color(tc.text_muted).truncate()),
        )
        // Action buttons
        .child(
            div()
                .flex_row()
                .flex_wrap()
                .gap(Sp::MD * scale)
                .pt(Sp::XS * scale)
                .child(
                    Button::new(Action::OpenCompareSheet)
                        .icon(lucide::PLAY)
                        .label("Open Compare")
                        .style(ButtonStyle::Filled),
                )
                .child(
                    Button::new(Action::OpenRepositoryDialog)
                        .icon(lucide::FOLDER_OPEN)
                        .label("Open Folder")
                        .style(ButtonStyle::Subtle),
                ),
        );

    // Recent repositories section
    if !state.settings.recent_repos.is_empty() {
        let mut recent_section = div().pt(Sp::SM).flex_col().gap(Sp::XS).child(
            text("Recent repositories")
                .text_xs()
                .semibold()
                .color(tc.text_muted),
        );

        for repo in state.settings.recent_repos.iter().take(5) {
            let label = repo.display().to_string();
            recent_section = recent_section.child(
                div()
                    .w_full()
                    .py(Sp::XS)
                    .px_2()
                    .rounded_sm()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .hover_bg(tc.ghost_element_hover)
                    .on_click(Action::OpenRepository(repo.clone()))
                    .cursor(CursorHint::Pointer)
                    .child(svg_icon(lucide::FOLDER, 13.0).color(tc.text_muted))
                    .child(text(label).text_sm().color(tc.text).truncate()),
            );
        }

        card = card.child(recent_section);
    }

    div()
        .flex_1()
        .items_center()
        .justify_center()
        .p(Sp::XL * scale)
        .child(card)
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

fn status_bar(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let (status_icon, status_color, status_text) = match state.repository.status {
        AsyncStatus::Ready => (lucide::CHECK, tc.line_add_text, "ready"),
        AsyncStatus::Loading => (lucide::LOADER, tc.text_muted, "loading"),
        AsyncStatus::Failed => (lucide::ALERT_CIRCLE, tc.status_error, "error"),
        AsyncStatus::Idle => (lucide::INFO, tc.text_muted, "idle"),
    };

    let right_text = format!(
        "{}  \u{00b7}  {}",
        compare_mode_label(state.compare.mode),
        renderer_label(state.compare.renderer),
    );

    div()
        .flex_row()
        .items_center()
        .h(theme.metrics.status_bar_height)
        .w_full()
        .px_4()
        .bg(tc.status_bar_background)
        .border_t(tc.border_variant)
        .child(svg_icon(status_icon, Ico::XS).color(status_color))
        .child(div().w(Rad::LG))
        .child(text(status_text).text_xs().color(tc.text_muted))
        .child(spacer())
        .child(text(right_text).text_xs().color(tc.text_muted))
}

fn toolbar_separator(tc: &crate::ui::theme::ThemeColors) -> Div {
    div().w(Sz::SEPARATOR_W).h(Sz::SEPARATOR_H).bg(tc.border_variant)
}

fn compare_mode_label(mode: CompareMode) -> &'static str {
    match mode {
        CompareMode::SingleCommit => "single-commit",
        CompareMode::TwoDot => "two-dot",
        CompareMode::ThreeDot => "three-dot",
    }
}

fn renderer_label(renderer: RendererKind) -> &'static str {
    match renderer {
        RendererKind::Builtin => "built-in",
        RendererKind::Difftastic => "difftastic",
    }
}

fn display_ref(value: &str) -> &str {
    if value.is_empty() { "?" } else { value }
}
