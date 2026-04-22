use std::cell::Cell;
use std::rc::Rc;

use halogen::view;

use crate::actions::Action;
use crate::render::{Rect, Scene, TextMetrics};
use crate::ui::components::{Button, ButtonSize, ButtonStyle, ToastStack};
use crate::ui::design::{Bp, Rad, Shadow, Sp, Sz};
use crate::ui::editor::element::{EditorDocument, EditorElement};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::overlays;
use crate::ui::settings_page;
use crate::ui::sidebar as sidebar_mod;
use crate::ui::state::{AppState, AppView, WorkspaceSource};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;
use crate::ui::title_bar;
use crate::ui::toolbar as toolbar_mod;

pub use halogen::CursorHint;

#[derive(Debug, Clone, Default)]
pub struct UiFrame {
    pub scene: Scene,
    pub hits: Vec<HitRegion>,
    pub scroll_regions: Vec<ScrollRegion>,
    pub text_input_hit_areas: Vec<TextInputHitArea>,
    pub scrollbar_tracks: Vec<ScrollbarTrack>,
    pub tooltip_regions: Vec<TooltipRegion>,
    pub file_list_rect: Option<Rect>,
    pub sidebar_resize_handle_rect: Option<Rect>,
    pub viewport_rect: Option<Rect>,
}

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
    let ui_scale = theme.metrics.ui_scale();

    let m = &theme.metrics;
    let row_h = m.ui_row_height;
    let has_files = state.workspace.files.with(&state.store, |f| !f.is_empty());
    let sidebar_header_h = if has_files {
        3.0 * row_h + Sp::SM * ui_scale
    } else {
        row_h
    };
    let commit_box_h = if state.workspace.source.get(&state.store) == WorkspaceSource::Status {
        (Sz::COMMIT_BOX_H * ui_scale).round() + Sp::SM * 2.0 * ui_scale
    } else {
        0.0
    };
    let sidebar_list_height =
        (height - m.title_bar_height - m.status_bar_height - sidebar_header_h - commit_box_h)
            .max(0.0);
    state.file_list.row_height.set(&state.store, row_h.round());
    state
        .file_list
        .gap
        .set(&state.store, (Sp::XS * ui_scale).round());
    let overlay_row_height = row_h.round().max(24.0) as u32;
    let overlay_gap = (Sp::XS * ui_scale).round() as u32;
    let picker_entries_len = state
        .overlays
        .picker
        .entries
        .with(&state.store, |e| e.len());
    state.overlays.picker.list.update(&state.store, |l| {
        l.row_height_px = overlay_row_height;
        l.gap_px = overlay_gap;
        l.viewport_height_px = l.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, picker_entries_len);
    });
    let palette_entries_len = state
        .overlays
        .command_palette
        .entries
        .with(&state.store, |e| e.len());
    state
        .overlays
        .command_palette
        .list
        .update(&state.store, |l| {
            l.row_height_px = overlay_row_height;
            l.gap_px = overlay_gap;
            l.viewport_height_px =
                l.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, palette_entries_len);
        });
    state
        .file_list
        .viewport_height
        .set(&state.store, sidebar_list_height);
    state.file_list_clamp_scroll(state.sidebar_row_count());
    let sidebar_width_factor = if state.sidebar_visible.get(&state.store) {
        1.0
    } else {
        0.0
    };
    let sidebar_width =
        sidebar_mod::preferred_sidebar_width(state, theme, cx, width) * sidebar_width_factor;

    let in_settings = state.app_view.get(&state.store) == AppView::Settings;

    let body: AnyElement = if in_settings {
        settings_page::settings_page(state, theme).into_any()
    } else {
        div()
            .flex_row()
            .flex_1()
            .min_h(0.0)
            .when(
                state.is_workspace_ready()
                    && sidebar_width_factor > 0.001
                    && width >= Bp::COMPACT * ui_scale,
                |d| {
                    d.child(sidebar_mod::sidebar(
                        state,
                        theme,
                        sidebar_width,
                        file_list_bounds.clone(),
                        cx,
                    ))
                    .child(sidebar_mod::sidebar_resizer(
                        theme,
                        sidebar_resize_bounds.clone(),
                        sidebar_width,
                    ))
                },
            )
            .child(toolbar_mod::main_surface(
                state,
                theme,
                text_metrics,
                viewport_bounds.clone(),
            ))
            .into_any()
    };

    let mut root = div()
        .w(width)
        .h(height)
        .flex_col()
        .bg(theme.colors.background)
        .child(title_bar::title_bar(
            state,
            theme,
            sidebar_width_factor,
            width,
        ))
        .child(body)
        .child(crate::ui::status_bar::status_bar(state, theme));

    if let Some(overlay) = overlays::render_active_overlay(state, theme, width, height) {
        root = root.child(overlay);
    }

    let toast_stack = state.toasts.with(&state.store, |toasts| {
        if toasts.is_empty() {
            None
        } else {
            use crate::ui::components::toast::{
                DESC_MAX_LINES, INNER_TEXT_W, TITLE_MAX_LINES, ToastLayout, compute_toast_height,
            };
            let title_fs = theme.metrics.ui_small_font_size;
            let desc_fs = theme.metrics.ui_small_font_size - 1.0;
            let layouts: Vec<ToastLayout> = toasts
                .iter()
                .map(|t| {
                    let title_lines = crate::ui::element::wrap_text_to_lines(
                        cx.font_system,
                        &t.message,
                        title_fs,
                        crate::render::FontKind::Ui,
                        crate::render::FontWeight::Medium,
                        INNER_TEXT_W,
                        TITLE_MAX_LINES,
                    );
                    let description_lines = t
                        .description
                        .as_deref()
                        .map(|d| {
                            crate::ui::element::wrap_text_to_lines(
                                cx.font_system,
                                d,
                                desc_fs,
                                crate::render::FontKind::Ui,
                                crate::render::FontWeight::Normal,
                                INNER_TEXT_W,
                                DESC_MAX_LINES,
                            )
                        })
                        .unwrap_or_default();
                    let height =
                        compute_toast_height(theme, title_lines.len(), description_lines.len());
                    ToastLayout {
                        title_lines,
                        description_lines,
                        height,
                    }
                })
                .collect();
            Some(
                ToastStack::new(
                    toasts,
                    &state.animation,
                    width,
                    height,
                    ui_scale,
                    m.status_bar_height,
                    state.clock_ms,
                    &layouts,
                )
                .build(),
            )
        }
    });
    if let Some(stack) = toast_stack {
        root = root.child(stack);
    }

    let mut root = root.into_any();

    let mut scene = Scene::default();
    render_element(&mut root, &mut scene, cx, width, height);
    let mut scrollbar_tracks = std::mem::take(&mut cx.scrollbar_tracks);

    if state.is_workspace_ready() {
        if let Some(vp_bounds) = viewport_bounds.get() {
            let active_file_snapshot = state.workspace.active_file.get(&state.store);
            let active_file_loading = state.workspace.active_file_loading.get(&state.store);
            let compare_generation = state.workspace.compare_generation.get(&state.store);
            let document = match active_file_snapshot.as_ref() {
                Some(active_file) if active_file.file.is_binary => EditorDocument::Binary {
                    path: &active_file.path,
                },
                Some(active_file) => EditorDocument::Text {
                    compare_generation,
                    file_index: active_file.index,
                    path: &active_file.path,
                    doc: &active_file.render_doc,
                },
                None if active_file_loading.is_some() => EditorDocument::Loading {
                    path: &active_file_loading.as_ref().expect("loading file").path,
                },
                None => EditorDocument::Empty,
            };
            let mut editor_snap = state.editor.snapshot(&state.store);
            if let Some(active_file) = active_file_snapshot.as_ref() {
                let expansion = state
                    .workspace
                    .expansions
                    .with(&state.store, |m| m.get(&active_file.path).cloned())
                    .unwrap_or_default();
                let caps = crate::ui::editor::expansion::populate_expand_blocks(
                    editor.blocks_mut(),
                    &active_file.base_file,
                    &active_file.render_doc,
                    &expansion,
                    active_file.file_line_count,
                );
                editor.set_hunk_expand_caps(caps);
            } else {
                editor.blocks_mut().clear();
                editor.set_hunk_expand_caps(Vec::new());
            }
            editor.prepare(&mut editor_snap, document, vp_bounds, text_metrics);
            editor_snap.hovered_hunk_index = match document {
                EditorDocument::Text { doc, .. } => editor_snap
                    .hovered_row
                    .and_then(|row| editor.render_line_index_for_row(row))
                    .and_then(|line_index| doc.lines.get(line_index as usize))
                    .and_then(|line| (line.hunk_index >= 0).then_some(line.hunk_index)),
                _ => None,
            };
            // Write back every field prepare may have mutated.
            state
                .editor
                .viewport_width_px
                .set_if_changed(&state.store, editor_snap.viewport_width_px);
            state
                .editor
                .viewport_height_px
                .set_if_changed(&state.store, editor_snap.viewport_height_px);
            state
                .editor
                .content_height_px
                .set_if_changed(&state.store, editor_snap.content_height_px);
            state
                .editor
                .scroll_top_px
                .set_if_changed(&state.store, editor_snap.scroll_top_px);
            state
                .editor
                .visible_row_start
                .set_if_changed(&state.store, editor_snap.visible_row_start);
            state
                .editor
                .visible_row_end
                .set_if_changed(&state.store, editor_snap.visible_row_end);
            state
                .editor
                .hovered_row
                .set_if_changed(&state.store, editor_snap.hovered_row);
            state
                .editor
                .hovered_hunk_index
                .set_if_changed(&state.store, editor_snap.hovered_hunk_index);
            state
                .editor
                .hunk_positions
                .set_if_changed(&state.store, editor_snap.hunk_positions.clone());
            state
                .editor
                .file_positions
                .set_if_changed(&state.store, editor_snap.file_positions.clone());
            state
                .editor
                .search_match_y_positions
                .set_if_changed(&state.store, editor_snap.search_match_y_positions.clone());
            state
                .editor
                .line_selection
                .set_if_changed(&state.store, editor_snap.line_selection.clone());
            editor.layout.show_staging_controls =
                state.workspace.source.get(&state.store) == WorkspaceSource::Status;
            editor.layout.file_is_staged = matches!(
                state.workspace.selected_status_scope.get(&state.store),
                Some(crate::core::vcs::git::StatusScope::Staged)
            );
            scene.clip(vp_bounds);
            editor.paint(&mut scene, theme, &editor_snap, document);
            scene.pop_clip();

            if editor.layout.show_staging_controls {
                if let EditorDocument::Text { doc, .. } = document {
                    let is_staged = editor.layout.file_is_staged;
                    let has_line_selection = !editor_snap.line_selection.is_empty();

                    let line_bar_rect = if has_line_selection {
                        editor.line_selection_bar_rect(doc, &editor_snap)
                    } else {
                        None
                    };
                    // The hunk bar is suppressed whenever a line selection
                    // exists — even if the line bar isn't currently visible
                    // (e.g. scrolled off-screen) — to avoid dual-bar overlap.
                    let hunk_bar_rect = if !has_line_selection {
                        editor.hunk_action_bar_rect(doc)
                    } else {
                        None
                    };

                    if let Some(bar_rect) = line_bar_rect {
                        let (stage_action, stage_label, stage_icon) = if is_staged {
                            (Action::UnstageSelectedLines, "Unstage Lines", lucide::MINUS)
                        } else {
                            (Action::StageSelectedLines, "Stage Lines", lucide::PLUS)
                        };
                        let mut bar = build_staging_bar(
                            theme,
                            ui_scale,
                            bar_rect,
                            stage_action,
                            stage_label,
                            stage_icon,
                            Action::DiscardSelectedLines,
                            "Discard Lines",
                        );
                        render_element_at(
                            &mut bar,
                            &mut scene,
                            cx,
                            bar_rect.x,
                            bar_rect.y,
                            bar_rect.width,
                            bar_rect.height,
                        );
                    } else if let Some((bar_rect, hunk_index)) = hunk_bar_rect {
                        let (stage_action, stage_label, stage_icon) = if is_staged {
                            (
                                Action::UnstageHunkAt(hunk_index),
                                "Unstage Hunk",
                                lucide::MINUS,
                            )
                        } else {
                            (Action::StageHunkAt(hunk_index), "Stage Hunk", lucide::PLUS)
                        };
                        let mut bar = build_staging_bar(
                            theme,
                            ui_scale,
                            bar_rect,
                            stage_action,
                            stage_label,
                            stage_icon,
                            Action::DiscardHunkAt(hunk_index),
                            "Discard Hunk",
                        );
                        render_element_at(
                            &mut bar,
                            &mut scene,
                            cx,
                            bar_rect.x,
                            bar_rect.y,
                            bar_rect.width,
                            bar_rect.height,
                        );
                    }
                }
            }

            let content_h = state.editor.content_height_px.get(&state.store);
            let viewport_h = state.editor.viewport_height_px.get(&state.store);
            if content_h > viewport_h && viewport_h > 0 {
                let sb = editor.scrollbar_rect();
                let ratio = viewport_h as f32 / content_h as f32;
                let thumb_h = (sb.height * ratio).max(Sp::XXL * ui_scale).min(sb.height);
                let scroll_range = state.editor_max_scroll_top_px().max(1) as f32;
                let top_ratio = state.editor.scroll_top_px.get(&state.store) as f32 / scroll_range;
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
                    content_height: content_h as f32,
                    viewport_height: viewport_h as f32,
                    action_builder: ScrollActionBuilder::ViewportLines,
                });
            }
        }
    }

    let hits = std::mem::take(&mut cx.hits);
    let scroll_regions = std::mem::take(&mut cx.scroll_regions);
    let text_input_hit_areas = std::mem::take(&mut cx.text_input_hit_areas);
    let tooltip_regions = std::mem::take(&mut cx.tooltip_regions);
    let file_list_rect = scroll_regions.iter().find_map(|region| {
        matches!(region.action_builder, ScrollActionBuilder::FileList).then_some(region.bounds)
    });

    UiFrame {
        scene,
        hits,
        scroll_regions,
        text_input_hit_areas,
        scrollbar_tracks,
        tooltip_regions,
        file_list_rect: file_list_rect.or_else(|| file_list_bounds.get()),
        sidebar_resize_handle_rect: sidebar_resize_bounds.get(),
        viewport_rect: viewport_bounds.get(),
    }
}

fn build_staging_bar(
    theme: &Theme,
    ui_scale: f32,
    bar_rect: Rect,
    stage_action: Action,
    stage_label: &'static str,
    stage_icon: &'static str,
    discard_action: Action,
    discard_label: &'static str,
) -> AnyElement {
    let tc = &theme.colors;
    view! { ui_scale,
        <div class="flex-row items-center"
             w={bar_rect.width} h={bar_rect.height}
             z_index={50}
             pr={Sp::SM}>
            <spacer />
            <div class="flex-row items-center"
                 bg={tc.modal_surface}
                 border_b={tc.border}
                 border_l={tc.border}
                 border_r={tc.border}
                 border_t={tc.border}
                 rounded={Rad::MD}
                 shadow_preset={Shadow::DROPDOWN}
                 on_click={Action::Noop}
                 gap={Sp::XXS}
                 px={Sp::XXS}
                 py={Sp::XXS}>
                <Button action={stage_action}
                        style={ButtonStyle::Ghost}
                        size={ButtonSize::Compact}>
                    <Icon>{stage_icon}</Icon>
                    <Label>{stage_label}</Label>
                </Button>
                <Button action={discard_action}
                        style={ButtonStyle::Ghost}
                        size={ButtonSize::Compact}>
                    <Icon>{lucide::CORNER_UP_LEFT}</Icon>
                    <Label>{discard_label}</Label>
                </Button>
            </div>
        </div>
    }
}
