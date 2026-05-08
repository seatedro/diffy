use std::sync::Arc;
use std::time::Instant;

use winit::window::{CursorIcon, Window};

use crate::actions::{
    AppAction, EditorAction, FileListAction, OverlayAction, RepositoryAction, TextEditAction,
};
use crate::render::Renderer;
use crate::ui::components::{TooltipSide, TooltipState};
use crate::ui::editor::element::EditorElement;
use crate::ui::element::{ClickEvent, ClickResult, DragHandler, HitIdentity};
use crate::ui::shell::UiFrame;
use crate::ui::state::{AppState, FocusTarget, WorkspaceSource};

use super::{InputOutcome, InputSystem};

impl InputSystem {
    pub(super) fn handle_left_click(
        &mut self,
        state: &AppState,
        ui_frame: &mut UiFrame,
        editor: &EditorElement,
        renderer: Option<&mut Renderer>,
        x: f32,
        y: f32,
    ) -> InputOutcome {
        if let Some(track) = ui_frame
            .scrollbar_tracks
            .iter()
            .rev()
            .find(|t| t.track_rect.contains(x, y))
        {
            let on_thumb = y >= track.thumb_top && y <= track.thumb_top + track.thumb_height;
            let mut handler = crate::ui::element::ScrollbarDragHandler::new(track, y);
            let mut outcome = InputOutcome::default();
            if matches!(
                track.action_builder,
                crate::ui::element::ScrollActionBuilder::ViewportGlobal
            ) {
                let content_height_px = track.content_height.max(0.0).round() as u32;
                let viewport_height_px = track.viewport_height.max(0.0).round() as u32;
                outcome.actions.push(
                    EditorAction::BeginViewportScrollbarDrag {
                        content_height_px,
                        viewport_height_px,
                        scroll_top_px: state.global_scroll_position_px(),
                        max_scroll_top_px: content_height_px.saturating_sub(viewport_height_px),
                    }
                    .into(),
                );
            }
            if !on_thumb {
                outcome.actions.extend(handler.on_move(x, y));
            }
            self.pointer_capture = Some(Box::new(handler));
            outcome.dirty = !outcome.actions.is_empty();
            return outcome;
        }

        if let Some(hit_area) = ui_frame
            .text_input_hit_areas
            .iter()
            .rev()
            .find(|ha| ha.bounds.contains(x, y))
        {
            if hit_area.multiline {
                let click_x = (x - hit_area.text_x) as i32;
                let click_y = (y - hit_area.text_y) as i32;
                self.mouse_drag_target = Some(hit_area.focus_target);
                return InputOutcome::actions(vec![
                    AppAction::SetFocus(Some(hit_area.focus_target)).into(),
                    EditorAction::EditorClick(click_x, click_y).into(),
                ]);
            }
            let byte_offset = hit_test_text_offset(
                renderer.map(Renderer::font_system),
                &hit_area.value,
                hit_area.font_size,
                x - hit_area.text_x,
            );
            self.mouse_drag_target = Some(hit_area.focus_target);
            return InputOutcome::actions(vec![
                AppAction::SetFocus(Some(hit_area.focus_target)).into(),
                TextEditAction::SetTextCursor(byte_offset).into(),
            ]);
        }

        if let Some(idx) = ui_frame
            .hits
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, hit)| hit.rect.contains(x, y).then_some(i))
        {
            let hit = &ui_frame.hits[idx];
            let mut actions = Vec::new();
            if matches!(hit.identity, Some(HitIdentity::File(_))) {
                actions.push(AppAction::SetFocus(Some(FocusTarget::FileList)).into());
            }
            match hit.on_click.invoke(ClickEvent { x, y }) {
                ClickResult::Handled => {}
                ClickResult::Actions(handler_actions) => actions.extend(handler_actions),
                ClickResult::CaptureDrag(drag) => {
                    self.pointer_capture = Some(drag);
                }
            }
            return InputOutcome::actions(actions);
        }

        if ui_frame
            .viewport_rect
            .is_some_and(|rect| rect.contains(x, y))
        {
            if let Some(path) = editor.file_header_path_at(x, y) {
                let mut actions = vec![EditorAction::FocusViewport.into()];
                if self.modifiers.super_key() || self.modifiers.control_key() {
                    actions.push(AppAction::CopyText(path).into());
                } else {
                    actions.push(FileListAction::SelectFilePath(path).into());
                }
                return InputOutcome::actions(actions);
            }
            let editor_snap = state.editor.snapshot(&state.store);
            let hovered = editor.hit_test_row(&editor_snap, x, y);
            if let Some(row) = hovered
                && editor.is_block_row(row)
                && let Some(block_action) = editor.block_action_for_row(row)
            {
                return InputOutcome::actions(vec![
                    EditorAction::FocusViewport.into(),
                    EditorAction::HoverViewportRow(hovered).into(),
                    block_action,
                ]);
            }
            let status_source = state.workspace.source.get(&state.store) == WorkspaceSource::Status;
            let review_source = state.pull_request_review_enabled();
            let supports_hunk_mutation =
                state
                    .repository
                    .capabilities
                    .with(&state.store, |capabilities| {
                        capabilities.is_some_and(|capabilities| capabilities.partial_hunk_mutation)
                    });
            let single_file_status_actions =
                status_source && !state.settings.continuous_scroll && supports_hunk_mutation;
            if (status_source || review_source) && editor.is_gutter_hit(x, y) {
                if let Some(row) = hovered {
                    if editor.is_block_row(row) {
                        return InputOutcome::actions(vec![
                            EditorAction::FocusViewport.into(),
                            EditorAction::HoverViewportRow(hovered).into(),
                        ]);
                    }
                    let line_idx =
                        editor.render_line_index_for_row(row).unwrap_or(u32::MAX) as usize;
                    let is_hunk_sep = state.workspace.active_file.with(&state.store, |af| {
                        af.as_ref()
                            .and_then(|a| a.render_doc.lines.get(line_idx).copied())
                            .is_some_and(|line| {
                                line.row_kind()
                                    == crate::ui::editor::render_doc::RenderRowKind::HunkSeparator
                            })
                    });
                    let mut actions = vec![
                        EditorAction::FocusViewport.into(),
                        EditorAction::HoverViewportRow(hovered).into(),
                    ];
                    if is_hunk_sep && single_file_status_actions {
                        let is_staged = matches!(
                            state.workspace.selected_change_bucket.get(&state.store),
                            Some(crate::core::vcs::model::ChangeBucket::Staged)
                        );
                        actions.push(if is_staged {
                            RepositoryAction::UnstageHunk.into()
                        } else {
                            RepositoryAction::StageHunk.into()
                        });
                    } else if is_hunk_sep {
                        // Hunk headers are not review-comment anchors.
                    } else if status_source && !single_file_status_actions {
                        // Continuous status rows are not single-file line anchors.
                    } else if self.modifiers.shift_key() {
                        let anchor = state
                            .editor
                            .line_selection
                            .with(&state.store, |ls| ls.last_toggled_row);
                        if let Some(anchor) = anchor {
                            actions.push(
                                RepositoryAction::ToggleLineSelectionRange(line_idx, anchor).into(),
                            );
                        } else {
                            actions.push(RepositoryAction::ToggleLineSelection(line_idx).into());
                        }
                    } else {
                        actions.push(RepositoryAction::ToggleLineSelection(line_idx).into());
                    }
                    return InputOutcome::actions(actions);
                }
            }
            return InputOutcome::actions(vec![
                EditorAction::FocusViewport.into(),
                EditorAction::HoverViewportRow(hovered).into(),
            ]);
        }

        InputOutcome::default()
    }

    pub(super) fn handle_pointer_moved(
        &mut self,
        state: &AppState,
        ui_frame: &UiFrame,
        editor: &EditorElement,
        renderer: Option<&mut Renderer>,
        window: Option<&Arc<Window>>,
        tooltip_state: &mut TooltipState,
        launch_at: Instant,
        x: f32,
        y: f32,
    ) -> InputOutcome {
        self.mouse_position = Some((x, y));

        let mut actions = Vec::new();
        if let Some(ref mut capture) = self.pointer_capture {
            actions.extend(capture.on_move(x, y));
        }

        if let Some(drag_target) = self.mouse_drag_target
            && let Some(hit_area) = ui_frame
                .text_input_hit_areas
                .iter()
                .find(|ha| ha.focus_target == drag_target)
        {
            if hit_area.multiline {
                let drag_x = (x - hit_area.text_x) as i32;
                let drag_y = (y - hit_area.text_y) as i32;
                actions.push(EditorAction::EditorDrag(drag_x, drag_y).into());
            } else {
                let byte_offset = hit_test_text_offset(
                    renderer.map(Renderer::font_system),
                    &hit_area.value,
                    hit_area.font_size,
                    x - hit_area.text_x,
                );
                actions.push(TextEditAction::ExtendTextSelection(byte_offset).into());
            }
        }

        let hovered_hit = ui_frame
            .hits
            .iter()
            .rev()
            .find(|hit| hit.rect.contains(x, y));
        let hovered_file = hovered_hit.and_then(|hit| match hit.identity {
            Some(HitIdentity::File(i)) => Some(i),
            _ => None,
        });
        let hovered_toast = hovered_hit.and_then(|hit| match hit.identity {
            Some(HitIdentity::Toast(i)) => Some(i),
            _ => None,
        });
        let hovered_overlay_entry = hovered_hit.and_then(|hit| match hit.identity {
            Some(HitIdentity::OverlayEntry(i)) => Some(i),
            _ => None,
        });
        let cursor_hint = if let Some(ref capture) = self.pointer_capture {
            capture.cursor()
        } else {
            let from_hits = hovered_hit
                .map(|hit| hit.cursor)
                .unwrap_or(crate::ui::shell::CursorHint::Default);
            if from_hits == crate::ui::shell::CursorHint::Default
                && ui_frame
                    .text_input_hit_areas
                    .iter()
                    .any(|ha| ha.bounds.contains(x, y))
            {
                crate::ui::shell::CursorHint::Text
            } else {
                from_hits
            }
        };

        if hovered_file != state.file_list.hovered_index.get(&state.store) {
            actions.push(FileListAction::HoverFile(hovered_file).into());
        }
        if hovered_overlay_entry != state.overlays.picker.hovered_index.get(&state.store) {
            actions.push(OverlayAction::HoverOverlayEntry(hovered_overlay_entry).into());
        }
        let current_hovered_toast = state
            .toasts
            .with(&state.store, |toasts| toasts.iter().position(|t| t.hovered));
        if hovered_toast != current_hovered_toast {
            actions.push(AppAction::HoverToast(hovered_toast).into());
        }

        let hovered_row = if input_is_blocked_by_overlay(state, ui_frame, x, y) {
            None
        } else {
            let editor_snap = state.editor.snapshot(&state.store);
            editor.hit_test_row(&editor_snap, x, y)
        };
        if hovered_row != state.editor.hovered_row.get(&state.store) {
            actions.push(EditorAction::HoverViewportRow(hovered_row).into());
        }

        let cursor_hint = if ui_frame
            .scrollbar_tracks
            .iter()
            .any(|t| t.track_rect.contains(x, y))
        {
            crate::ui::shell::CursorHint::Pointer
        } else if editor.file_header_path_at(x, y).is_some() {
            crate::ui::shell::CursorHint::Pointer
        } else if hovered_row.is_some_and(|row| editor.is_block_row(row)) {
            crate::ui::shell::CursorHint::Pointer
        } else if cursor_hint == crate::ui::shell::CursorHint::Default
            && hovered_row.is_some()
            && !editor.is_gutter_hit(x, y)
        {
            crate::ui::shell::CursorHint::Text
        } else {
            cursor_hint
        };

        let now_ms = launch_at.elapsed().as_millis() as u64;
        let hovered_tooltip = ui_frame
            .tooltip_regions
            .iter()
            .rev()
            .find(|region| region.bounds.contains(x, y));
        if let Some(region) = hovered_tooltip {
            if tooltip_state.text != region.text {
                tooltip_state.show(
                    &region.text,
                    x,
                    region.bounds.y + region.bounds.height,
                    TooltipSide::Bottom,
                    500,
                    now_ms,
                );
            }
        } else {
            tooltip_state.hide();
        }
        tooltip_state.tick(now_ms);

        if let Some(window) = window {
            let icon = match cursor_hint {
                crate::ui::shell::CursorHint::Default => CursorIcon::Default,
                crate::ui::shell::CursorHint::Pointer => CursorIcon::Pointer,
                crate::ui::shell::CursorHint::Text => CursorIcon::Text,
                crate::ui::shell::CursorHint::ResizeCol => CursorIcon::EwResize,
            };
            window.set_cursor(icon);
        }

        // Always request a redraw on mouse movement so that hitbox-based
        // hover styles (e.g. picker item highlights) update immediately.
        let mut outcome = InputOutcome::actions(actions);
        outcome.dirty = true;
        outcome
    }

    pub(super) fn handle_left_release(&mut self, state: &AppState) -> InputOutcome {
        let mut outcome = InputOutcome::default();
        if let Some(mut capture) = self.pointer_capture.take() {
            let result = capture.on_release(state);
            outcome.actions = result.actions;
            outcome.effects = result.effects;
            outcome.dirty = true;
        }
        self.mouse_drag_target = None;
        outcome
    }
}

fn input_is_blocked_by_overlay(state: &AppState, ui_frame: &UiFrame, x: f32, y: f32) -> bool {
    state.overlays_top().is_some()
        && ui_frame
            .hits
            .iter()
            .rev()
            .any(|hit| hit.rect.contains(x, y))
}

pub fn hit_test_text_offset(
    font_system: Option<&mut glyphon::FontSystem>,
    text: &str,
    font_size: f32,
    click_x: f32,
) -> usize {
    if text.is_empty() || click_x <= 0.0 {
        return 0;
    }
    let Some(font_system) = font_system else {
        return text.len();
    };

    let metrics = glyphon::Metrics::new(font_size, font_size * 1.2);
    let mut buffer = glyphon::Buffer::new(font_system, metrics);
    let attrs = glyphon::Attrs::new().family(glyphon::Family::SansSerif);
    buffer.set_size(font_system, None, None);
    buffer.set_text(font_system, text, &attrs, glyphon::Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);

    let mut best_offset = text.len();
    let mut best_dist = f32::MAX;

    for run in buffer.layout_runs() {
        let dist = click_x.abs();
        if dist < best_dist {
            best_dist = dist;
            best_offset = 0;
        }
        for glyph in run.glyphs.iter() {
            let left_dist = (click_x - glyph.x).abs();
            if left_dist < best_dist {
                best_dist = left_dist;
                best_offset = glyph.start;
            }
            let right_dist = (click_x - (glyph.x + glyph.w)).abs();
            if right_dist < best_dist {
                best_dist = right_dist;
                best_offset = glyph.end;
            }
        }
        let dist = (click_x - run.line_w).abs();
        if dist < best_dist {
            best_dist = dist;
            best_offset = text.len();
        }
    }

    best_offset
}
