use std::sync::Arc;
use std::time::Instant;

use winit::window::{CursorIcon, Window};

use crate::actions::Action;
use crate::render::Renderer;
use crate::ui::components::{TooltipSide, TooltipState};
use crate::ui::editor::element::EditorElement;
use crate::ui::element::{ClickEvent, ClickResult, DragHandler};
use crate::ui::shell::UiFrame;
use crate::ui::state::{AppState, FocusTarget};

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
            if !on_thumb {
                outcome.actions = handler.on_move(x, y);
                outcome.dirty = !outcome.actions.is_empty();
            }
            self.pointer_capture = Some(Box::new(handler));
            return outcome;
        }

        if let Some(hit_area) = ui_frame
            .text_input_hit_areas
            .iter()
            .rev()
            .find(|ha| ha.bounds.contains(x, y))
        {
            let byte_offset = hit_test_text_offset(
                renderer.map(Renderer::font_system),
                &hit_area.value,
                hit_area.font_size,
                x - hit_area.text_x,
            );
            self.mouse_drag_target = Some(hit_area.focus_target);
            return InputOutcome::actions(vec![
                Action::SetFocus(Some(hit_area.focus_target)),
                Action::SetTextCursor(byte_offset),
            ]);
        }

        if let Some(idx) = ui_frame
            .hits
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, hit)| hit.rect.contains(x, y).then_some(i))
        {
            let hit = &mut ui_frame.hits[idx];
            let mut actions = Vec::new();
            if let Some(handler) = hit.on_click.take() {
                match handler.invoke(ClickEvent { x, y }) {
                    ClickResult::Handled => {}
                    ClickResult::Actions(handler_actions) => actions.extend(handler_actions),
                    ClickResult::CaptureDrag(drag) => {
                        self.pointer_capture = Some(drag);
                    }
                }
            } else {
                let action = hit.action.clone();
                if matches!(action, Action::SelectFile(_)) {
                    actions.push(Action::SetFocus(Some(FocusTarget::FileList)));
                }
                actions.push(action);
            }
            return InputOutcome::actions(actions);
        }

        if ui_frame
            .viewport_rect
            .is_some_and(|rect| rect.contains(x, y))
        {
            let hovered = editor.hit_test_row(&state.editor, x, y);
            return InputOutcome::actions(vec![
                Action::FocusViewport,
                Action::HoverViewportRow(hovered),
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
            let byte_offset = hit_test_text_offset(
                renderer.map(Renderer::font_system),
                &hit_area.value,
                hit_area.font_size,
                x - hit_area.text_x,
            );
            actions.push(Action::ExtendTextSelection(byte_offset));
        }

        let hovered_hit = ui_frame
            .hits
            .iter()
            .rev()
            .find(|hit| hit.rect.contains(x, y));
        let hovered_file = hovered_hit.and_then(|hit| match &hit.action {
            Action::SelectFile(i) => Some(*i),
            _ => None,
        });
        let hovered_toast = hovered_hit.and_then(|hit| match &hit.action {
            Action::DismissToast(i) => Some(*i),
            _ => None,
        });
        let cursor_hint = if let Some(ref capture) = self.pointer_capture {
            capture.cursor()
        } else {
            hovered_hit
                .map(|hit| hit.cursor)
                .unwrap_or(crate::ui::shell::CursorHint::Default)
        };

        if hovered_file != state.file_list.hovered_index {
            actions.push(Action::HoverFile(hovered_file));
        }
        let current_hovered_toast = state.toasts.iter().position(|toast| toast.hovered);
        if hovered_toast != current_hovered_toast {
            actions.push(Action::HoverToast(hovered_toast));
        }

        let hovered_row = if input_is_blocked_by_overlay(state, ui_frame, x, y) {
            None
        } else {
            editor.hit_test_row(&state.editor, x, y)
        };
        if hovered_row != state.editor.hovered_row {
            actions.push(Action::HoverViewportRow(hovered_row));
        }

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

        InputOutcome::actions(actions)
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
    state.overlays.top().is_some()
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
