use winit::event::{MouseScrollDelta, TouchPhase};

use crate::actions::Action;
use crate::ui::editor::element::EditorElement;
use crate::ui::element::ScrollActionBuilder;
use crate::ui::shell::UiFrame;
use crate::ui::state::AppState;

use super::{InputOutcome, InputSystem, ScrollTarget};

fn custom_scroll_is_editor(build: fn(i32) -> Action) -> bool {
    matches!(build(0), Action::EditorScrollPx(_))
}

impl InputSystem {
    pub(super) fn handle_wheel(
        &mut self,
        state: &AppState,
        ui_frame: &UiFrame,
        editor: &EditorElement,
        delta: MouseScrollDelta,
        phase: TouchPhase,
    ) -> InputOutcome {
        let Some((x, y)) = self.mouse_position else {
            return InputOutcome::default();
        };

        if matches!(phase, TouchPhase::Started | TouchPhase::Cancelled) {
            self.reset_scroll_remainders();
        }

        let Some(target) = self.scroll_target_at(state, ui_frame, x, y) else {
            return InputOutcome::default();
        };
        let line_step_px = self.scroll_target_line_step_px(state, &target, editor);
        let lines_per_notch = state.settings.wheel_scroll_lines.max(1) as f32;
        let delta_px = scroll_delta_to_px(delta, line_step_px, lines_per_notch);
        let rounded_delta_px = match &target {
            ScrollTarget::Region(ScrollActionBuilder::FileList) => {
                quantize_scroll_delta_px(&mut self.file_list_scroll_remainder_px, delta_px)
            }
            ScrollTarget::Region(ScrollActionBuilder::Custom(build)) => {
                if custom_scroll_is_editor(*build) {
                    quantize_scroll_delta_px(&mut self.editor_scroll_remainder_px, delta_px)
                } else {
                    quantize_scroll_delta_px(&mut self.overlay_scroll_remainder_px, delta_px)
                }
            }
            ScrollTarget::Region(ScrollActionBuilder::ViewportLines)
            | ScrollTarget::ViewportFallback => {
                quantize_scroll_delta_px(&mut self.viewport_scroll_remainder_px, delta_px)
            }
        };

        let mut actions = Vec::new();
        if rounded_delta_px != 0 {
            match target {
                ScrollTarget::Region(ScrollActionBuilder::FileList) => {
                    actions.push(Action::ScrollFileListPx(rounded_delta_px));
                }
                ScrollTarget::Region(ScrollActionBuilder::Custom(build)) => {
                    actions.push(build(rounded_delta_px));
                }
                ScrollTarget::Region(ScrollActionBuilder::ViewportLines)
                | ScrollTarget::ViewportFallback => {
                    actions.push(Action::ScrollViewportPx(rounded_delta_px));
                }
            }
        }

        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.reset_scroll_remainders();
        }

        InputOutcome::actions(actions)
    }

    pub(super) fn reset_scroll_remainders(&mut self) {
        self.file_list_scroll_remainder_px = 0.0;
        self.overlay_scroll_remainder_px = 0.0;
        self.editor_scroll_remainder_px = 0.0;
        self.viewport_scroll_remainder_px = 0.0;
    }

    pub(super) fn scroll_target_at(
        &self,
        state: &AppState,
        ui_frame: &UiFrame,
        x: f32,
        y: f32,
    ) -> Option<ScrollTarget> {
        for region in ui_frame.scroll_regions.iter().rev() {
            if region.bounds.contains(x, y) {
                return Some(ScrollTarget::Region(region.action_builder.clone()));
            }
        }

        if state.overlays_top().is_some()
            && ui_frame
                .hits
                .iter()
                .rev()
                .any(|hit| hit.rect.contains(x, y))
        {
            return None;
        }

        ui_frame
            .viewport_rect
            .filter(|rect| rect.contains(x, y))
            .map(|_| ScrollTarget::ViewportFallback)
    }

    pub(super) fn scroll_target_line_step_px(
        &self,
        state: &AppState,
        target: &ScrollTarget,
        editor: &EditorElement,
    ) -> f32 {
        match target {
            ScrollTarget::Region(ScrollActionBuilder::FileList) => {
                state.file_list_row_stride().max(1.0)
            }
            ScrollTarget::Region(ScrollActionBuilder::Custom(build)) => {
                if custom_scroll_is_editor(*build) {
                    state.commit_editor.scroll_line_height_px().max(1.0)
                } else {
                    active_overlay_row_height_px(state)
                }
            }
            ScrollTarget::Region(ScrollActionBuilder::ViewportLines)
            | ScrollTarget::ViewportFallback => editor.scroll_line_height_px(),
        }
    }
}

fn active_overlay_row_height_px(state: &AppState) -> f32 {
    use crate::ui::state::OverlaySurface;
    match state.overlays_top() {
        Some(
            OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_) | OverlaySurface::ThemePicker,
        ) => state
            .overlays
            .picker
            .list
            .with(&state.store, |l| l.stride_px().max(1)) as f32,
        Some(OverlaySurface::CommandPalette) => state
            .overlays
            .command_palette
            .list
            .with(&state.store, |l| l.stride_px().max(1))
            as f32,
        _ => 36.0,
    }
}

pub fn scroll_delta_to_px(delta: MouseScrollDelta, line_step_px: f32, lines_per_notch: f32) -> f32 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => -y * line_step_px * lines_per_notch,
        MouseScrollDelta::PixelDelta(position) => -(position.y as f32),
    }
}

pub fn quantize_scroll_delta_px(remainder_px: &mut f32, delta_px: f32) -> i32 {
    *remainder_px += delta_px;
    let whole_px = remainder_px.trunc() as i32;
    *remainder_px -= whole_px as f32;
    whole_px
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::editor::element::EditorElement;

    #[test]
    fn commit_editor_scroll_uses_editor_line_height() {
        let input = InputSystem::default();
        let state = AppState::default();
        let editor = EditorElement::default();
        let target = ScrollTarget::Region(ScrollActionBuilder::Custom(Action::EditorScrollPx));

        let line_step = input.scroll_target_line_step_px(&state, &target, &editor);

        assert!((line_step - state.commit_editor.scroll_line_height_px()).abs() < f32::EPSILON);
    }
}
