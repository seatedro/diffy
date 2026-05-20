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
                let mut effects = self.scroll_viewport_lines(delta);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            ScrollViewportPx(delta_px) => {
                let mut effects = self.scroll_viewport_px(delta_px);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            ScrollViewportPages(delta) => {
                let mut effects = self.scroll_viewport_pages(delta);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            ScrollViewportTo(px) => {
                self.editor.scroll_top_px.set(&self.store, px);
                self.editor_clamp_scroll();
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            ScrollViewportToGlobal(px) => self.scroll_viewport_to_global(px),
            BeginViewportScrollbarDrag {
                content_height_px,
                viewport_height_px,
                scroll_top_px,
                max_scroll_top_px,
            } => {
                self.begin_viewport_scrollbar_drag(
                    content_height_px,
                    viewport_height_px,
                    scroll_top_px,
                    max_scroll_top_px,
                );
                Vec::new()
            }
            EndViewportScrollbarDrag => {
                self.end_viewport_scrollbar_drag();
                let current = self.workspace.global_scroll_top_px.get(&self.store);
                let mut effects = self.scroll_viewport_to_global(current);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            ScrollViewportHalfPage(dir) => {
                let mut effects = self.scroll_viewport_half_page(dir);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            HoverViewportRow(row) => {
                self.editor.hovered_row.set(&self.store, row);
                if row.is_none() {
                    self.editor.hovered_render_line_index.set(&self.store, None);
                    self.editor.hovered_hunk_index.set(&self.store, None);
                }
                Vec::new()
            }
            MoveRowCursor(delta) => {
                self.move_editor_row_cursor(delta);
                Vec::new()
            }
            FocusViewport => {
                self.set_focus(Some(FocusTarget::Editor));
                Vec::new()
            }
            GoToNextHunk => {
                self.navigate_to_hunk(true);
                if self.settings.continuous_scroll {
                    self.sync_global_scroll_from_editor();
                }
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            GoToPreviousHunk => {
                self.navigate_to_hunk(false);
                if self.settings.continuous_scroll {
                    self.sync_global_scroll_from_editor();
                }
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            GoToNextFile => {
                let mut effects = self.navigate_to_file(true);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            GoToPreviousFile => {
                let mut effects = self.navigate_to_file(false);
                effects.extend(self.request_active_file_syntax_effect());
                effects
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
                match self.focus.get(&self.store) {
                    Some(FocusTarget::SettingsSteeringPrompt) => {
                        self.steering_prompt_editor.click(x, y);
                    }
                    Some(FocusTarget::ReviewCommentEditor) => {
                        self.review_comment_editor.click(x, y);
                    }
                    _ => {
                        self.commit_editor.click(x, y);
                    }
                }
                Vec::new()
            }
            EditorDrag(x, y) => {
                match self.focus.get(&self.store) {
                    Some(FocusTarget::SettingsSteeringPrompt) => {
                        self.steering_prompt_editor.drag(x, y);
                    }
                    Some(FocusTarget::ReviewCommentEditor) => {
                        self.review_comment_editor.drag(x, y);
                    }
                    _ => {
                        self.commit_editor.drag(x, y);
                    }
                }
                Vec::new()
            }
            EditorScrollPx(delta) => {
                match self.focus.get(&self.store) {
                    Some(FocusTarget::SettingsSteeringPrompt) => {
                        self.steering_prompt_editor.scroll(delta as f32);
                    }
                    Some(FocusTarget::ReviewCommentEditor) => {
                        self.review_comment_editor.scroll(delta as f32);
                    }
                    _ => {
                        self.commit_editor.scroll(delta as f32);
                    }
                }
                Vec::new()
            }
            BeginViewportTextSelection { point, generation } => {
                self.editor.text_selection.set(
                    &self.store,
                    Some(crate::ui::editor::state::ViewportTextSelection::new(
                        generation, point,
                    )),
                );
                Vec::new()
            }
            ExtendViewportTextSelection(point) => {
                self.editor.text_selection.update(&self.store, |selection| {
                    if let Some(selection) = selection {
                        selection.focus = point;
                    }
                });
                Vec::new()
            }
            ClearViewportTextSelection => {
                self.editor.text_selection.set(&self.store, None);
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
