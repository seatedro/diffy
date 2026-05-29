use std::cell::Cell;
use std::rc::Rc;

use halogen::view;

use crate::actions::Action;
use crate::effects::Effect;
use crate::render::{Rect, Scene, TextMetrics};
use crate::ui::components::{Button, ButtonSize, ButtonStyle, ToastStack};
use crate::ui::design::{Bp, Rad, Shadow, Sp, Sz};
use crate::ui::editor::element::{EditorDocument, EditorElement, ScrollbarOverride};
use crate::ui::editor_element::{CursorSnapshot, text_editor_element};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::overlays;
use crate::ui::settings_page;
use crate::ui::sidebar as sidebar_mod;
use crate::ui::state::{
    ActiveFile, ActiveFileLoading, AppState, AppView, ViewportDocument, WorkspaceSource,
};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;
use crate::ui::toolbar as toolbar_mod;
use crate::ui::window_chrome;

pub use halogen::CursorHint;

#[derive(Debug, Clone, Default)]
pub struct UiFrame {
    pub scene: Scene,
    pub hits: Vec<HitRegion>,
    pub scroll_regions: Vec<ScrollRegion>,
    pub text_input_hit_areas: Vec<TextInputHitArea>,
    pub scrollbar_tracks: Vec<ScrollbarTrack>,
    pub tooltip_regions: Vec<TooltipRegion>,
    pub accessibility: crate::ui::accessibility::AccessibilityFrame,
    pub effects: Vec<Effect>,
    pub file_list_rect: Option<Rect>,
    pub sidebar_resize_handle_rect: Option<Rect>,
    pub viewport_rect: Option<Rect>,
    pub viewport_document: Option<ViewportDocument>,
}

pub fn build_ui_frame(
    state: &mut AppState,
    theme: &Theme,
    editor: &mut EditorElement,
    text_metrics: TextMetrics,
    width: f32,
    height: f32,
    is_maximized: bool,
    cx: &mut ElementContext,
) -> UiFrame {
    let mut effects = Vec::new();
    cx.accessibility = crate::ui::accessibility::AccessibilityFrame::new(width, height);
    let viewport_bounds: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));
    let file_list_bounds: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));
    let sidebar_resize_bounds: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));
    let ui_scale = theme.metrics.ui_scale();

    let m = &theme.metrics;
    let row_h = m.ui_row_height;
    let has_files = state.workspace_file_count() > 0;
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
    state
        .file_list
        .row_height
        .set_if_changed(&state.store, row_h.round());
    state
        .file_list
        .gap
        .set_if_changed(&state.store, (Sp::XS * ui_scale).round());
    let overlay_row_height = row_h.round().max(24.0) as u32;
    let overlay_gap = (Sp::XS * ui_scale).round() as u32;
    let picker_entries_len = state
        .overlays
        .picker
        .entries
        .with(&state.store, |e| e.len());
    let mut picker_list = state.overlays.picker.list.get(&state.store);
    picker_list.row_height_px = overlay_row_height;
    picker_list.gap_px = overlay_gap;
    picker_list.viewport_height_px =
        picker_list.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, picker_entries_len);
    state
        .overlays
        .picker
        .list
        .set_if_changed(&state.store, picker_list);
    let palette_entries_len = state
        .overlays
        .command_palette
        .entries
        .with(&state.store, |e| e.len());
    let mut palette_list = state.overlays.command_palette.list.get(&state.store);
    palette_list.row_height_px = overlay_row_height;
    palette_list.gap_px = overlay_gap;
    palette_list.viewport_height_px =
        palette_list.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, palette_entries_len);
    state
        .overlays
        .command_palette
        .list
        .set_if_changed(&state.store, palette_list);
    state
        .file_list
        .viewport_height
        .set_if_changed(&state.store, sidebar_list_height);
    state.file_list_clamp_scroll(state.sidebar_row_count());

    // Settings → Keymaps body viewport. The keymaps layout is:
    //   title row (pt=XL + title_block + pb=LG) above
    //   scroll body (fills remaining height)
    // Status bar sits below the body, so the viewport equals the window
    // height minus title bar, status bar, and the title row's vertical chrome.
    let keymaps_title_block_h =
        m.heading_font_size * 1.4 + Sp::XXS * ui_scale + m.ui_small_font_size * 1.4;
    let keymaps_viewport_h = (height
        - m.title_bar_height
        - m.status_bar_height
        - Sp::XL * ui_scale
        - keymaps_title_block_h
        - Sp::LG * ui_scale)
        .max(0.0);
    state
        .keymaps_viewport_height_px
        .set(&state.store, keymaps_viewport_h);
    state
        .keymaps_content_height_px
        .set(&state.store, settings_page::keymaps_content_height(theme));
    state.clamp_keymaps_scroll();
    let sidebar_width_factor = if state.sidebar_visible.get(&state.store) {
        1.0
    } else {
        0.0
    };
    let sidebar_width =
        sidebar_mod::preferred_sidebar_width(state, theme, cx, width) * sidebar_width_factor;

    let in_settings = state.app_view.get(&state.store) == AppView::Settings;

    // Once the reveal delay has elapsed we want the skeleton to take the
    // sidebar slot even if `workspace_mode` is still Ready — a re-compare
    // keeps the old file list around as scaffolding during the grace
    // window, but after the grace window we're committed to showing the
    // loading view, so blow the old sidebar away.
    let progress_visible = state.compare_progress.with(&state.store, |p| {
        p.as_ref().is_some_and(|p| state.clock_ms >= p.reveal_at_ms)
    });
    let sidebar_slot_visible = sidebar_width_factor > 0.001 && width >= Bp::COMPACT * ui_scale;
    let show_real_sidebar = state.is_workspace_ready() && !progress_visible;
    let show_skeleton_sidebar = progress_visible;

    let body: AnyElement = if in_settings {
        settings_page::settings_page(state, theme).into_any()
    } else {
        div()
            .flex_row()
            .flex_1()
            .min_h(0.0)
            .when(show_real_sidebar && sidebar_slot_visible, |d| {
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
            })
            // Shimmer placeholder: renders in the sidebar slot while the
            // progress panel is up, giving the user a preview of the
            // destination UI as the data fills in.
            .when(show_skeleton_sidebar && sidebar_slot_visible, |d| {
                d.child(
                    div()
                        .w(sidebar_width)
                        .h_full()
                        .bg(theme.colors.sidebar_background)
                        .border_r(theme.colors.border_variant)
                        .child(crate::ui::components::sidebar_skeleton(theme)),
                )
            })
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
        .child(window_chrome::window_chrome(
            state,
            theme,
            sidebar_width_factor,
            is_maximized,
        ))
        .child(body)
        .child(crate::ui::status_bar::status_bar(state, theme));

    if let Some(overlay) = overlays::render_active_overlay(state, theme, width, height) {
        root = root.child(overlay);
    }

    if let Some(menu) = state.context_menu.render(theme) {
        root = root.child(menu);
    }

    if let Some(edges) = window_chrome::resize_edges(width, height) {
        root = root.child(edges);
    }

    let toast_stack = state.toasts.with(&state.store, |toasts| {
        if toasts.is_empty() {
            None
        } else {
            use crate::ui::components::toast::{
                DESC_MAX_LINES, TITLE_MAX_LINES, ToastLayout, compute_toast_height,
                toast_inner_text_width, toast_stack_width,
            };
            let title_fs = theme.metrics.ui_small_font_size;
            let desc_fs = theme.metrics.ui_small_font_size - 1.0;
            let toast_text_w = toast_inner_text_width(toast_stack_width(width, ui_scale));
            let layouts: Vec<ToastLayout> = toasts
                .iter()
                .map(|t| {
                    let title_lines = crate::ui::element::wrap_text_to_lines(
                        cx.font_system,
                        &t.message,
                        title_fs,
                        crate::render::FontKind::Ui,
                        crate::render::FontWeight::Medium,
                        toast_text_w,
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
                                toast_text_w,
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
    let mut input_viewport_document = None;

    if state.is_workspace_ready() {
        if let Some(vp_bounds) = viewport_bounds.get() {
            let continuous_scroll = state.settings.continuous_scroll;
            state
                .editor
                .viewport_width_px
                .set_if_changed(&state.store, vp_bounds.width.max(0.0).round() as u32);
            state
                .editor
                .viewport_height_px
                .set_if_changed(&state.store, vp_bounds.height.max(0.0).round() as u32);
            if continuous_scroll {
                effects.extend(state.sync_editor_scroll_from_global());
            }

            let active_file_snapshot = state.workspace.active_file.get(&state.store);
            let active_file_loading = state.workspace.active_file_loading.get(&state.store);
            let render_generation = state.workspace_render_generation();
            let selected_file_index = state.workspace.selected_file_index.get(&state.store);
            let selected_file_path = state.workspace.selected_file_path.get(&state.store);
            let active_file_matches_selection =
                active_file_snapshot.as_ref().is_some_and(|active_file| {
                    selected_file_index == Some(active_file.index)
                        && selected_file_path.as_deref() == Some(active_file.path.as_str())
                });

            let mut viewport_document = if continuous_scroll {
                let (doc, doc_effects) = state.build_continuous_viewport_document();
                effects.extend(doc_effects);
                doc
            } else if let Some(active_file) = active_file_snapshot.as_ref().filter(|active_file| {
                active_file_matches_selection && !active_file.carbon_file.is_binary
            }) {
                Some(ViewportDocument::single(
                    active_file.render_doc.clone(),
                    render_generation,
                    active_file.index,
                    active_file.path.clone(),
                ))
            } else {
                None
            };
            let mut editor_snap = state.editor.snapshot(&state.store);
            editor_snap.review_enabled = state.pull_request_review_enabled();
            if let Some(doc) = viewport_document.as_ref().filter(|doc| doc.is_continuous()) {
                editor_snap.scroll_top_px = state
                    .global_scroll_position_px()
                    .saturating_sub(doc.start_offset_px);
            }

            // Pinned card width: derived from vp_bounds with the same inset prepare
            // applies, so it equals content_bounds.width this frame and is identical at
            // measure (here) and render (overlay loop). Reused by both.
            let review_card_width = editor.content_width_for_bounds(vp_bounds, text_metrics);

            // Precompute pass: measure each visible-file thread card's natural height via
            // compute_layout BEFORE borrowing blocks_mut, so the block can reserve the
            // exact height the overlay will render. Keyed by thread id (globally unique).
            // The gather paths below mirror the populate branches exactly so every block
            // created has a measured height (the fallback is only a safety net).
            let mut review_card_heights: std::collections::HashMap<
                crate::core::review::ReviewThreadId,
                u16,
            > = std::collections::HashMap::new();
            if state.pull_request_review_enabled() {
                let mut threads_to_measure: Vec<crate::core::review::ReviewThread> = Vec::new();
                if let Some(doc) = viewport_document.as_ref().filter(|doc| doc.is_continuous()) {
                    for slot in doc.slot_indices.iter().copied() {
                        if let Some(file) = state.viewport_file_snapshot(slot) {
                            threads_to_measure
                                .extend(state.active_pr_review_threads_for_file(&file.carbon_file));
                        }
                    }
                } else if let Some(active_file) = active_file_snapshot.as_ref() {
                    threads_to_measure =
                        state.active_pr_review_threads_for_file(&active_file.carbon_file);
                }
                for thread in &threads_to_measure {
                    let expanded = state.review_thread_expanded(thread);
                    let h = crate::ui::editor::review::measure_review_thread_card_height(
                        thread,
                        expanded,
                        theme,
                        ui_scale,
                        review_card_width,
                        cx,
                    );
                    review_card_heights.insert(thread.id.clone(), h);
                }
            }

            if let Some(doc) = viewport_document.as_ref().filter(|doc| doc.is_continuous()) {
                editor.blocks_mut().clear();
                populate_continuous_review_blocks(
                    state,
                    editor.blocks_mut(),
                    doc,
                    &review_card_heights,
                );
                editor.set_hunk_expand_caps(Vec::new());
            } else if let Some(active_file) = active_file_snapshot.as_ref() {
                let expansion = state
                    .workspace
                    .expansions
                    .with(&state.store, |m| m.get(&active_file.path).cloned())
                    .unwrap_or_default();
                let caps = crate::ui::editor::expansion::populate_expand_blocks(
                    editor.blocks_mut(),
                    &active_file.carbon_file,
                    &active_file.render_doc,
                    &expansion,
                );
                let review_threads =
                    state.active_pr_review_threads_for_file(&active_file.carbon_file);
                if review_threads.is_empty() {
                    let review_comments =
                        state.active_pr_review_comments_for_file(&active_file.carbon_file);
                    crate::ui::editor::review::populate_review_comment_blocks(
                        editor.blocks_mut(),
                        &active_file.render_doc,
                        &review_comments,
                    );
                } else {
                    crate::ui::editor::review::populate_review_thread_blocks(
                        editor.blocks_mut(),
                        &active_file.render_doc,
                        &active_file.carbon_file,
                        &review_threads,
                        &review_card_heights,
                        |thread| state.review_thread_expanded(thread),
                    );
                }
                editor.set_hunk_expand_caps(caps);
            } else {
                editor.blocks_mut().clear();
                editor.set_hunk_expand_caps(Vec::new());
            }
            let scrollbar_override = if continuous_scroll {
                let scrollbar = state.continuous_viewport_scrollbar_metrics();
                Some(ScrollbarOverride {
                    total_height_px: scrollbar.content_height_px,
                    scroll_top_px: scrollbar.scroll_top_px,
                    max_scroll_top_px: scrollbar.max_scroll_top_px,
                })
            } else {
                None
            };
            editor.set_scrollbar_override(scrollbar_override);
            {
                let document = editor_document_for(
                    viewport_document.as_ref(),
                    active_file_snapshot.as_ref(),
                    active_file_loading.as_ref(),
                    render_generation,
                    active_file_matches_selection,
                );
                editor.prepare(&mut editor_snap, document, vp_bounds, text_metrics);
            }

            state
                .editor
                .content_height_px
                .set_if_changed(&state.store, editor_snap.content_height_px);

            let height_changed = if let Some(doc) =
                viewport_document.as_ref().filter(|doc| doc.is_continuous())
            {
                update_continuous_slot_heights(state, doc, &editor_snap)
            } else if let Some(active_file) = active_file_snapshot.as_ref()
                && editor_snap.content_height_px > 0
            {
                state
                    .update_file_content_height_px(active_file.index, editor_snap.content_height_px)
            } else {
                false
            };

            if continuous_scroll && height_changed {
                effects.extend(state.sync_editor_scroll_from_global());
                let (doc, doc_effects) = state.build_continuous_viewport_document();
                effects.extend(doc_effects);
                viewport_document = doc;
                if let Some(doc) = viewport_document.as_ref() {
                    editor_snap.scroll_top_px = state
                        .global_scroll_position_px()
                        .saturating_sub(doc.start_offset_px);
                }
                let scrollbar = state.continuous_viewport_scrollbar_metrics();
                editor.set_scrollbar_override(Some(ScrollbarOverride {
                    total_height_px: scrollbar.content_height_px,
                    scroll_top_px: scrollbar.scroll_top_px,
                    max_scroll_top_px: scrollbar.max_scroll_top_px,
                }));
                {
                    let document = editor_document_for(
                        viewport_document.as_ref(),
                        active_file_snapshot.as_ref(),
                        active_file_loading.as_ref(),
                        render_generation,
                        active_file_matches_selection,
                    );
                    editor.prepare(&mut editor_snap, document, vp_bounds, text_metrics);
                }
                state
                    .editor
                    .content_height_px
                    .set_if_changed(&state.store, editor_snap.content_height_px);
            }

            let using_continuous_doc = viewport_document
                .as_ref()
                .is_some_and(|doc| doc.is_continuous());
            let document = editor_document_for(
                viewport_document.as_ref(),
                active_file_snapshot.as_ref(),
                active_file_loading.as_ref(),
                render_generation,
                active_file_matches_selection,
            );
            let hovered_render_line_index = match document {
                EditorDocument::Text { .. } => editor_snap
                    .hovered_row
                    .and_then(|row| editor.render_line_index_for_row(row))
                    .map(|line| line as usize),
                _ => None,
            };
            editor_snap.hovered_render_line_index = hovered_render_line_index;
            editor_snap.hovered_hunk_index = if using_continuous_doc {
                None
            } else {
                match document {
                    EditorDocument::Text { doc, .. } => hovered_render_line_index
                        .and_then(|line_index| doc.lines.get(line_index))
                        .and_then(|line| (line.hunk_index >= 0).then_some(line.hunk_index)),
                    _ => None,
                }
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
                .hovered_render_line_index
                .set_if_changed(&state.store, editor_snap.hovered_render_line_index);
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
            let supports_hunk_mutation =
                state
                    .repository
                    .capabilities
                    .with(&state.store, |capabilities| {
                        capabilities.is_some_and(|capabilities| capabilities.partial_hunk_mutation)
                    });
            editor.layout.show_staging_controls = state.workspace.source.get(&state.store)
                == WorkspaceSource::Status
                && !using_continuous_doc
                && supports_hunk_mutation;
            editor.layout.file_is_staged = matches!(
                state.workspace.selected_change_bucket.get(&state.store),
                Some(crate::core::vcs::model::ChangeBucket::Staged)
            );
            scene.clip(vp_bounds);
            editor.paint(&mut scene, theme, &editor_snap, document);
            scene.pop_clip();
            let editor_scroll_builder = if continuous_scroll {
                ScrollActionBuilder::ViewportGlobal
            } else {
                ScrollActionBuilder::ViewportLines
            };
            editor.append_accessibility(
                &mut cx.accessibility,
                &editor_snap,
                document,
                editor_scroll_builder,
            );

            // Review thread cards: the blocks reserve height but paint nothing; render
            // each as a real view! element overlay at its on-screen rect, clipped to the
            // viewport band (minus any sticky file header) so partly-scrolled cards don't
            // paint or capture clicks over the chrome above.
            if state.pull_request_review_enabled() {
                // The width the card is measured AND rendered at must match the editor's
                // content column, or wrapped line counts (hence reserved height) would
                // diverge from what is painted. Guard against future inset drift.
                debug_assert!(
                    (review_card_width - editor.layout.content_bounds.width).abs() < 1.5,
                    "review_card_width {} drifted from content_bounds.width {}",
                    review_card_width,
                    editor.layout.content_bounds.width
                );
                let band = match editor.sticky_header_rect() {
                    Some(h) => Rect {
                        x: vp_bounds.x,
                        y: h.bottom(),
                        width: vp_bounds.width,
                        height: (vp_bounds.bottom() - h.bottom()).max(0.0),
                    },
                    None => vp_bounds,
                };
                let cards = editor.visible_review_card_rows();
                if !cards.is_empty() {
                    scene.clip(band);
                    for (idx, rect) in cards {
                        let Some((thread, expanded)) = editor
                            .blocks()
                            .get(idx)
                            .and_then(|block| block.review_card())
                            .map(|(thread, expanded)| (thread.clone(), expanded))
                        else {
                            continue;
                        };
                        let mut card = crate::ui::editor::review::build_review_thread_card(
                            &thread,
                            expanded,
                            theme,
                            ui_scale,
                            review_card_width,
                            cx.font_system,
                        );
                        let hit_start = cx.hits.len();
                        render_element_at(
                            &mut card,
                            &mut scene,
                            cx,
                            rect.x,
                            rect.y,
                            review_card_width,
                            rect.height,
                        );
                        // Clip the card's freshly-registered hit rects to the visible band
                        // so a partly-off-screen card cannot steal clicks over the chrome.
                        let tail = cx.hits.split_off(hit_start);
                        for mut region in tail {
                            if let Some(clipped) = region.rect.intersection(band) {
                                region.rect = clipped;
                                cx.hits.push(region);
                            }
                        }
                    }
                    scene.pop_clip();
                }
            }

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
                            (
                                crate::actions::RepositoryAction::UnstageSelectedLines.into(),
                                "Unstage Lines",
                                lucide::MINUS,
                            )
                        } else {
                            (
                                crate::actions::RepositoryAction::StageSelectedLines.into(),
                                "Stage Lines",
                                lucide::PLUS,
                            )
                        };
                        let mut bar = build_staging_bar(
                            theme,
                            ui_scale,
                            bar_rect,
                            stage_action,
                            stage_label,
                            stage_icon,
                            crate::actions::RepositoryAction::DiscardSelectedLines.into(),
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
                                crate::actions::RepositoryAction::UnstageHunkAt(hunk_index).into(),
                                "Unstage Hunk",
                                lucide::MINUS,
                            )
                        } else {
                            (
                                crate::actions::RepositoryAction::StageHunkAt(hunk_index).into(),
                                "Stage Hunk",
                                lucide::PLUS,
                            )
                        };
                        let mut bar = build_staging_bar(
                            theme,
                            ui_scale,
                            bar_rect,
                            stage_action,
                            stage_label,
                            stage_icon,
                            crate::actions::RepositoryAction::DiscardHunkAt(hunk_index).into(),
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

            if state.pull_request_review_enabled() {
                if let EditorDocument::Text { doc, .. } = document {
                    let has_line_selection = !editor_snap.line_selection.is_empty();
                    let line_bar_rect = if has_line_selection {
                        editor.line_selection_bar_rect(doc, &editor_snap)
                    } else {
                        None
                    };
                    let composer_open = state
                        .github
                        .pull_request
                        .review_composer
                        .with(&state.store, |composer| composer.draft.is_some());
                    if composer_open {
                        if let Some(anchor_rect) = line_bar_rect {
                            let composer_h = (190.0 * ui_scale).round();
                            let y = (anchor_rect.y + anchor_rect.height)
                                .min(vp_bounds.bottom() - composer_h)
                                .max(vp_bounds.y);
                            let composer_rect = Rect {
                                x: anchor_rect.x,
                                y,
                                width: anchor_rect.width,
                                height: composer_h,
                            };
                            let mut composer =
                                build_review_composer(state, theme, ui_scale, composer_rect);
                            render_element_at(
                                &mut composer,
                                &mut scene,
                                cx,
                                composer_rect.x,
                                composer_rect.y,
                                composer_rect.width,
                                composer_rect.height,
                            );
                        }
                    } else if let Some(bar_rect) = line_bar_rect {
                        let mut bar = build_review_bar(theme, ui_scale, bar_rect);
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

            let continuous_scroll = state.settings.continuous_scroll;
            let (content_h, viewport_h, action_builder) = if continuous_scroll {
                let scrollbar = state.continuous_viewport_scrollbar_metrics();
                (
                    scrollbar.content_height_px,
                    scrollbar.viewport_height_px,
                    ScrollActionBuilder::ViewportGlobal,
                )
            } else {
                (
                    state.editor.content_height_px.get(&state.store),
                    state.editor.viewport_height_px.get(&state.store),
                    ScrollActionBuilder::ViewportLines,
                )
            };
            if content_h > viewport_h
                && viewport_h > 0
                && let Some(sb) = editor.scrollbar_layout()
            {
                scrollbar_tracks.push(ScrollbarTrack {
                    track_rect: Rect {
                        x: sb.track.x - Rad::LG * ui_scale,
                        y: sb.track.y,
                        width: sb.track.width + Sp::MD * ui_scale,
                        height: sb.track.height,
                    },
                    thumb_top: sb.thumb_top,
                    thumb_height: sb.thumb_height,
                    content_height: content_h as f32,
                    viewport_height: viewport_h as f32,
                    action_builder,
                });
            }
            input_viewport_document = viewport_document.clone();
        }
    }

    let hits = std::mem::take(&mut cx.hits);
    let scroll_regions = std::mem::take(&mut cx.scroll_regions);
    let text_input_hit_areas = std::mem::take(&mut cx.text_input_hit_areas);
    let tooltip_regions = std::mem::take(&mut cx.tooltip_regions);
    let accessibility = std::mem::take(&mut cx.accessibility);
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
        accessibility,
        effects,
        file_list_rect: file_list_rect.or_else(|| file_list_bounds.get()),
        sidebar_resize_handle_rect: sidebar_resize_bounds.get(),
        viewport_rect: viewport_bounds.get(),
        viewport_document: input_viewport_document,
    }
}

fn editor_document_for<'a>(
    viewport: Option<&'a ViewportDocument>,
    active_file: Option<&'a ActiveFile>,
    active_file_loading: Option<&'a ActiveFileLoading>,
    compare_generation: u64,
    active_file_matches_selection: bool,
) -> EditorDocument<'a> {
    if let Some(viewport) = viewport {
        return EditorDocument::Text {
            compare_generation: viewport.generation,
            file_index: viewport.start_index,
            path: &viewport.path,
            doc: viewport.doc.as_ref(),
            show_file_headers: viewport.is_continuous(),
        };
    }

    match active_file {
        Some(active_file) if active_file_matches_selection && active_file.carbon_file.is_binary => {
            EditorDocument::Binary {
                path: &active_file.path,
            }
        }
        Some(active_file) if active_file_matches_selection => EditorDocument::Text {
            compare_generation,
            file_index: active_file.index,
            path: &active_file.path,
            doc: active_file.render_doc.as_ref(),
            show_file_headers: false,
        },
        None if active_file_loading.is_some() => EditorDocument::Loading {
            path: &active_file_loading.expect("loading file").path,
        },
        Some(_) if active_file_loading.is_some() => EditorDocument::Loading {
            path: &active_file_loading.expect("loading file").path,
        },
        None => EditorDocument::Empty,
        Some(_) => EditorDocument::Empty,
    }
}

fn update_continuous_slot_heights(
    state: &mut AppState,
    continuous: &ViewportDocument,
    editor_snap: &crate::ui::editor::state::EditorState,
) -> bool {
    let mut changed = false;
    for (position_index, slot_index) in continuous.slot_indices.iter().copied().enumerate() {
        if continuous
            .slot_loading
            .get(position_index)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let Some(start) = editor_snap.file_positions.get(position_index).copied() else {
            continue;
        };
        let end = editor_snap
            .file_positions
            .get(position_index + 1)
            .copied()
            .unwrap_or(editor_snap.content_height_px);
        let height = end.saturating_sub(start);
        if height > 0 {
            changed |= state.update_file_content_height_px(slot_index, height);
        }
    }
    changed
}

fn populate_continuous_review_blocks(
    state: &AppState,
    blocks: &mut crate::ui::editor::decoration::BlockRegistry,
    viewport: &ViewportDocument,
    heights: &std::collections::HashMap<crate::core::review::ReviewThreadId, u16>,
) {
    if !state.pull_request_review_enabled() {
        return;
    }

    let render_doc = viewport.doc.as_ref();
    for slot_index in viewport.slot_indices.iter().copied() {
        let Some(file) = state.viewport_file_snapshot(slot_index) else {
            continue;
        };
        let Some(line_range) =
            crate::ui::editor::review::render_doc_file_line_range(render_doc, &file.path)
        else {
            continue;
        };
        let review_threads = state.active_pr_review_threads_for_file(&file.carbon_file);
        if review_threads.is_empty() {
            let review_comments = state.active_pr_review_comments_for_file(&file.carbon_file);
            crate::ui::editor::review::populate_review_comment_blocks_in_range(
                blocks,
                render_doc,
                line_range,
                &review_comments,
            );
        } else {
            crate::ui::editor::review::populate_review_thread_blocks_in_range(
                blocks,
                render_doc,
                &file.carbon_file,
                line_range,
                &review_threads,
                heights,
                |thread| state.review_thread_expanded(thread),
            );
        }
    }
}

fn build_review_bar(theme: &Theme, ui_scale: f32, bar_rect: Rect) -> AnyElement {
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
                <Button action={crate::actions::GitHubAction::OpenReviewCommentComposer.into()}
                        style={ButtonStyle::Ghost}
                        size={ButtonSize::Compact}>
                    <Icon>{lucide::PENCIL}</Icon>
                    <Label>{"Comment"}</Label>
                </Button>
                <Button action={crate::actions::RepositoryAction::ClearLineSelection.into()}
                        style={ButtonStyle::Ghost}
                        size={ButtonSize::Compact}>
                    <Icon>{lucide::X}</Icon>
                </Button>
            </div>
        </div>
    }
}

fn build_review_composer(state: &AppState, theme: &Theme, ui_scale: f32, rect: Rect) -> AnyElement {
    let tc = &theme.colors;
    let focused =
        state.focus.get(&state.store) == Some(crate::ui::state::FocusTarget::ReviewCommentEditor);
    let submitting = state
        .github
        .pull_request
        .review_composer
        .with(&state.store, |composer| {
            composer.status == crate::ui::state::AsyncStatus::Loading
        });
    let submit_icon = if submitting {
        lucide::LOADER
    } else {
        lucide::CHECK
    };
    let submit_label = if submitting {
        "Posting"
    } else {
        "Post Comment"
    };
    let cursor = CursorSnapshot {
        x: state.review_comment_editor.cursor_pos.x,
        y: state.review_comment_editor.cursor_pos.y,
        moved_at_ms: state.review_comment_editor.cursor_moved_at_ms,
    };
    let editor = text_editor_element()
        .placeholder("Leave a review comment")
        .is_empty(state.review_comment_editor.is_empty())
        .focused(focused)
        .focus_target(crate::ui::state::FocusTarget::ReviewCommentEditor)
        .editor_id(2)
        .font_size(theme.metrics.ui_small_font_size)
        .text_color(tc.text)
        .cursor(cursor)
        .selection(state.review_comment_editor.selection_rects())
        .content_height(state.review_comment_editor.content_height())
        .scroll_y(state.review_comment_editor.scroll_y)
        .w_full()
        .flex_1();

    view! { ui_scale,
        <div class="flex-row"
             w={rect.width} h={rect.height}
             z_index={60}
             pr={Sp::SM}>
            <spacer />
            <div class="flex-col"
                 w={rect.width.min(620.0)}
                 h={rect.height}
                 bg={tc.modal_surface}
                 border_b={tc.border}
                 border_l={tc.border}
                 border_r={tc.border}
                 border_t={tc.border}
                 rounded={Rad::LG}
                 shadow_preset={Shadow::DROPDOWN}
                 on_click={Action::Noop}
                 p={Sp::SM}
                 gap={Sp::SM}>
                <div class="flex-1 w-full"
                     min_h={0.0}
                     px={Sp::SM}
                     py={Sp::XS}
                     rounded={Rad::MD}
                     border={if focused { tc.accent } else { tc.border_variant }}
                     on_click={crate::actions::AppAction::SetFocus(Some(crate::ui::state::FocusTarget::ReviewCommentEditor)).into()}
                     cursor={CursorHint::Text}>
                    {editor}
                </div>
                <div class="flex-row items-center" gap={Sp::XS}>
                    <spacer />
                    <Button action={crate::actions::GitHubAction::CancelReviewComment.into()}
                            style={ButtonStyle::Ghost}
                            size={ButtonSize::Compact}
                            disabled={submitting}>
                        <Icon>{lucide::X}</Icon>
                        <Label>{"Cancel"}</Label>
                    </Button>
                    <Button action={crate::actions::GitHubAction::SubmitReviewComment.into()}
                            style={ButtonStyle::Subtle}
                            size={ButtonSize::Compact}
                            disabled={submitting}>
                        <Icon>{submit_icon}</Icon>
                        <Label>{submit_label}</Label>
                    </Button>
                </div>
            </div>
        </div>
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
