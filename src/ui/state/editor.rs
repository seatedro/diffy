use crate::actions::EditorAction;
use crate::effects::Effect;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: EditorAction) -> Vec<Effect> {
    state.apply_editor_action(action)
}

impl AppState {
    pub(super) fn apply_editor_action(&mut self, action: EditorAction) -> Vec<Effect> {
        use EditorAction::*;
        match action {
            ScrollViewportLines(delta) => {
                self.scroll_viewport_lines(delta);
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            ScrollViewportPx(delta_px) => {
                self.scroll_viewport_px(delta_px);
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            ScrollViewportPages(delta) => {
                self.scroll_viewport_pages(delta);
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            ScrollViewportTo(px) => {
                self.editor.scroll_top_px.set(&self.store, px);
                self.editor_clamp_scroll();
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            ScrollViewportHalfPage(dir) => {
                self.scroll_viewport_half_page(dir);
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            HoverViewportRow(row) => {
                self.editor.hovered_row.set(&self.store, row);
                if row.is_none() {
                    self.editor.hovered_hunk_index.set(&self.store, None);
                }
                Vec::new()
            }
            FocusViewport => {
                self.set_focus(Some(FocusTarget::Editor));
                Vec::new()
            }
            GoToNextHunk => {
                self.navigate_to_hunk(true);
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            GoToPreviousHunk => {
                self.navigate_to_hunk(false);
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            GoToNextFile => {
                self.navigate_to_file(true);
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            GoToPreviousFile => {
                self.navigate_to_file(false);
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            OpenSearch => {
                self.open_search();
                Vec::new()
            }
            CloseSearch => {
                self.close_search();
                Vec::new()
            }
            SearchNext => {
                self.search_navigate(1);
                Vec::new()
            }
            SearchPrevious => {
                self.search_navigate(-1);
                Vec::new()
            }
            EditorClick(x, y) => {
                if self.focus.get(&self.store) == Some(FocusTarget::SettingsSteeringPrompt) {
                    self.steering_prompt_editor.click(x, y);
                } else {
                    self.commit_editor.click(x, y);
                }
                Vec::new()
            }
            EditorDrag(x, y) => {
                if self.focus.get(&self.store) == Some(FocusTarget::SettingsSteeringPrompt) {
                    self.steering_prompt_editor.drag(x, y);
                } else {
                    self.commit_editor.drag(x, y);
                }
                Vec::new()
            }
            EditorScrollPx(delta) => {
                if self.focus.get(&self.store) == Some(FocusTarget::SettingsSteeringPrompt) {
                    self.steering_prompt_editor.scroll(delta as f32);
                } else {
                    self.commit_editor.scroll(delta as f32);
                }
                Vec::new()
            }
            ExpandContextAbove(hunk_index, amount) => self.expand_context(
                hunk_index,
                crate::ui::editor::expansion::ExpandDirection::Above,
                amount,
            ),
            ExpandContextBelow(hunk_index, amount) => self.expand_context(
                hunk_index,
                crate::ui::editor::expansion::ExpandDirection::Below,
                amount,
            ),
            ExpandAllContext => self.expand_all_context(),
        }
    }
}
