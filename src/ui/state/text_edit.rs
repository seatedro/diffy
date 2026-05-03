use unicode_segmentation::UnicodeSegmentation;

use crate::actions::TextEditAction;
use crate::effects::{AiEffect, Effect, UiEffect};
use crate::platform::secrets::AiKeyKind;

use super::{AppState, CompareField, FocusTarget, PickerKind};

pub(super) fn reduce_action(state: &mut AppState, action: TextEditAction) -> Vec<Effect> {
    state.apply_text_edit_action(action)
}

impl AppState {
    pub(super) fn selection_range(&self) -> Option<(usize, usize)> {
        let c = self.text_edit.cursor.get(&self.store);
        let a = self.text_edit.anchor.get(&self.store);
        if c == a {
            None
        } else {
            Some((c.min(a), c.max(a)))
        }
    }

    /// Delete the current selection and collapse cursor. Returns true if something was deleted.
    pub(super) fn delete_selection(&mut self) -> bool {
        self.clamp_cursor();
        if let Some((start, end)) = self.selection_range() {
            self.update_focused_text(|text| {
                text.drain(start..end);
            });
            self.text_edit.cursor.set(&self.store, start);
            self.text_edit.anchor.set(&self.store, start);
            true
        } else {
            false
        }
    }

    /// Called after text mutation to sync compare fields and rebuild pickers.
    pub(super) fn after_text_mutation(&mut self) -> Vec<Effect> {
        match self.focus.get(&self.store) {
            Some(FocusTarget::PickerInput) => match self.overlays.picker.kind.get(&self.store) {
                PickerKind::Repository => self.rebuild_repo_picker(),
                PickerKind::LeftRef => {
                    self.compare.resolved_left.set(&self.store, None);
                    return self.rebuild_ref_picker(CompareField::Left);
                }
                PickerKind::RightRef => {
                    self.compare.resolved_right.set(&self.store, None);
                    return self.rebuild_ref_picker(CompareField::Right);
                }
                PickerKind::Theme => self.rebuild_theme_picker(),
            },
            Some(FocusTarget::CommandPaletteInput) => return self.rebuild_command_palette(),
            Some(FocusTarget::SearchInput) => self.recompute_search_matches(),
            Some(FocusTarget::SettingsOpenAiKey) => {
                if !self.startup.keyring_enabled {
                    return Vec::new();
                }
                return vec![ai_key_save_effect(AiKeyKind::OpenAi, &self.ai_openai_key)];
            }
            Some(FocusTarget::SettingsAnthropicKey) => {
                if !self.startup.keyring_enabled {
                    return Vec::new();
                }
                return vec![ai_key_save_effect(
                    AiKeyKind::Anthropic,
                    &self.ai_anthropic_key,
                )];
            }
            _ => {}
        }
        Vec::new()
    }

    /// Should we persist settings after editing the current field?
    pub(super) fn needs_persist(&self) -> bool {
        matches!(
            self.focus.get(&self.store),
            Some(FocusTarget::PickerInput)
                if matches!(self.overlays.picker.kind.get(&self.store), PickerKind::LeftRef | PickerKind::RightRef)
        )
    }

    pub(super) fn text_edit_effects(&mut self) -> Vec<Effect> {
        let mut effects = self.after_text_mutation();
        if self.needs_persist() {
            effects.extend(self.persist_settings_effect());
        }
        effects
    }

    pub(super) fn insert_text(&mut self, value: String) -> Vec<Effect> {
        if self.with_focused_text(|_| ()).is_none() {
            return Vec::new();
        }
        self.delete_selection();
        self.clamp_cursor();
        let cursor = self.text_edit.cursor.get(&self.store);
        self.update_focused_text(|text| {
            text.insert_str(cursor, &value);
        });
        let new_cursor = cursor + value.len();
        self.text_edit.cursor.set(&self.store, new_cursor);
        self.text_edit.anchor.set(&self.store, new_cursor);
        self.touch_cursor();
        self.text_edit_effects()
    }

    pub(super) fn backspace(&mut self) -> Vec<Effect> {
        if self.with_focused_text(|_| ()).is_none() {
            return Vec::new();
        }
        if self.delete_selection() {
            self.touch_cursor();
            return self.text_edit_effects();
        }
        let cursor = self.text_edit.cursor.get(&self.store);
        if cursor == 0 {
            return Vec::new();
        }
        let prev = self
            .with_focused_text(|t| prev_grapheme_boundary(t, cursor))
            .unwrap_or(0);
        self.update_focused_text(|text| {
            text.drain(prev..cursor);
        });
        self.text_edit.cursor.set(&self.store, prev);
        self.text_edit.anchor.set(&self.store, prev);
        self.touch_cursor();
        self.text_edit_effects()
    }

    pub(super) fn delete_forward(&mut self) -> Vec<Effect> {
        if self.with_focused_text(|_| ()).is_none() {
            return Vec::new();
        }
        if self.delete_selection() {
            self.touch_cursor();
            return self.text_edit_effects();
        }
        let cursor = self.text_edit.cursor.get(&self.store);
        let len = self.with_focused_text(|s| s.len()).unwrap_or(0);
        if cursor >= len {
            return Vec::new();
        }
        let next = self
            .with_focused_text(|t| next_grapheme_boundary(t, cursor))
            .unwrap_or(cursor);
        self.update_focused_text(|text| {
            text.drain(cursor..next);
        });
        self.touch_cursor();
        self.text_edit_effects()
    }

    pub(super) fn move_cursor(&mut self, offset: usize, extend_selection: bool) {
        self.text_edit.cursor.set(&self.store, offset);
        if !extend_selection {
            self.text_edit.anchor.set(&self.store, offset);
        }
        self.touch_cursor();
    }

    pub(super) fn cursor_left(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let start = self
                .text_edit
                .cursor
                .get(&self.store)
                .min(self.text_edit.anchor.get(&self.store));
            self.move_cursor(start, false);
            return;
        }
        let cursor = self.text_edit.cursor.get(&self.store);
        if cursor == 0 {
            return;
        }
        let prev = self
            .with_focused_text(|t| prev_grapheme_boundary(t, cursor))
            .unwrap_or(0);
        self.move_cursor(prev, extend);
    }

    pub(super) fn cursor_right(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let end = self
                .text_edit
                .cursor
                .get(&self.store)
                .max(self.text_edit.anchor.get(&self.store));
            self.move_cursor(end, false);
            return;
        }
        let cursor = self.text_edit.cursor.get(&self.store);
        let len = self.with_focused_text(|s| s.len()).unwrap_or(0);
        if cursor >= len {
            return;
        }
        let next = self
            .with_focused_text(|t| next_grapheme_boundary(t, cursor))
            .unwrap_or(cursor);
        self.move_cursor(next, extend);
    }

    pub(super) fn cursor_word_left(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let start = self
                .text_edit
                .cursor
                .get(&self.store)
                .min(self.text_edit.anchor.get(&self.store));
            self.move_cursor(start, false);
            return;
        }
        let cursor = self.text_edit.cursor.get(&self.store);
        let pos = self
            .with_focused_text(|t| prev_word_boundary(t, cursor))
            .unwrap_or(0);
        self.move_cursor(pos, extend);
    }

    pub(super) fn cursor_word_right(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let end = self
                .text_edit
                .cursor
                .get(&self.store)
                .max(self.text_edit.anchor.get(&self.store));
            self.move_cursor(end, false);
            return;
        }
        let cursor = self.text_edit.cursor.get(&self.store);
        let len = self.with_focused_text(|s| s.len()).unwrap_or(0);
        let pos = self
            .with_focused_text(|t| next_word_boundary(t, cursor))
            .unwrap_or(len);
        self.move_cursor(pos, extend);
    }

    pub(super) fn cursor_home(&mut self, extend: bool) {
        self.move_cursor(0, extend);
    }

    pub(super) fn cursor_end(&mut self, extend: bool) {
        let len = self.with_focused_text(|s| s.len()).unwrap_or(0);
        self.move_cursor(len, extend);
    }

    pub(super) fn select_all(&mut self) {
        let len = self.with_focused_text(|s| s.len()).unwrap_or(0);
        self.text_edit.anchor.set(&self.store, 0);
        self.text_edit.cursor.set(&self.store, len);
        self.touch_cursor();
    }

    /// Copy text selection or, if none, the selected overlay entry.
    /// Returns `(effects, Some(value))` when copying an entry (toast-worthy).
    pub(super) fn copy_selection(&self) -> (Vec<Effect>, Option<String>) {
        if let Some((start, end)) = self.selection_range() {
            if let Some(selected) = self.with_focused_text(|text| text[start..end].to_string()) {
                return (vec![UiEffect::SetClipboard(selected).into()], None);
            }
        }
        // No text selection — copy the selected picker/palette entry's value.
        if matches!(self.focus.get(&self.store), Some(FocusTarget::PickerInput)) {
            let selected = self.overlays.picker.selected_index.get(&self.store);
            let value = self.overlays.picker.entries.with(&self.store, |entries| {
                entries.get(selected).map(|e| e.value.clone())
            });
            if let Some(value) = value {
                return (
                    vec![UiEffect::SetClipboard(value.clone()).into()],
                    Some(value),
                );
            }
        }
        if matches!(
            self.focus.get(&self.store),
            Some(FocusTarget::CommandPaletteInput)
        ) {
            let selected = self
                .overlays
                .command_palette
                .selected_index
                .get(&self.store);
            let label = self
                .overlays
                .command_palette
                .entries
                .with(&self.store, |entries| {
                    entries.get(selected).map(|e| e.label.clone())
                });
            if let Some(label) = label {
                return (
                    vec![UiEffect::SetClipboard(label.clone()).into()],
                    Some(label),
                );
            }
        }
        (Vec::new(), None)
    }

    pub(super) fn cut_selection(&mut self) -> Vec<Effect> {
        let (mut effects, ..) = self.copy_selection();
        if self.delete_selection() {
            self.touch_cursor();
            effects.extend(self.text_edit_effects());
        }
        effects
    }

    pub(super) fn paste(&mut self, value: String) -> Vec<Effect> {
        self.insert_text(value)
    }
}

pub(super) fn prev_grapheme_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let mut prev = 0;
    for (idx, _) in text.grapheme_indices(true) {
        if idx >= offset {
            break;
        }
        prev = idx;
    }
    prev
}

pub(super) fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    for (idx, grapheme) in text.grapheme_indices(true) {
        if idx >= offset {
            return idx + grapheme.len();
        }
    }
    text.len()
}

pub(super) fn prev_word_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let bytes = text.as_bytes();
    let mut pos = offset;
    // Skip whitespace/punctuation backwards
    while pos > 0 && !bytes[pos - 1].is_ascii_alphanumeric() {
        pos -= 1;
    }
    // Skip word chars backwards
    while pos > 0 && bytes[pos - 1].is_ascii_alphanumeric() {
        pos -= 1;
    }
    pos
}

pub(super) fn next_word_boundary(text: &str, offset: usize) -> usize {
    let len = text.len();
    if offset >= len {
        return len;
    }
    let bytes = text.as_bytes();
    let mut pos = offset;
    // Skip word chars forward
    while pos < len && bytes[pos].is_ascii_alphanumeric() {
        pos += 1;
    }
    // Skip whitespace/punctuation forward
    while pos < len && !bytes[pos].is_ascii_alphanumeric() {
        pos += 1;
    }
    pos
}

fn ai_key_save_effect(kind: AiKeyKind, value: &str) -> Effect {
    if value.is_empty() {
        AiEffect::ClearAiKey { kind }.into()
    } else {
        AiEffect::SaveAiKey {
            kind,
            value: value.to_owned(),
        }
        .into()
    }
}

impl AppState {
    pub(super) fn apply_text_edit_action(&mut self, action: TextEditAction) -> Vec<Effect> {
        use TextEditAction::*;
        if self.focus.get(&self.store) == Some(FocusTarget::CommitEditor) {
            return self.apply_commit_editor_action(action);
        }
        if self.focus.get(&self.store) == Some(FocusTarget::ReviewCommentEditor) {
            return self.apply_review_comment_editor_action(action);
        }
        if self.focus.get(&self.store) == Some(FocusTarget::SettingsSteeringPrompt) {
            return self.apply_steering_prompt_action(action);
        }
        match action {
            InsertText(value) => self.insert_text(value),
            Backspace => self.backspace(),
            DeleteForward => self.delete_forward(),
            CursorLeft => {
                self.cursor_left(false);
                Vec::new()
            }
            CursorRight => {
                self.cursor_right(false);
                Vec::new()
            }
            CursorWordLeft => {
                self.cursor_word_left(false);
                Vec::new()
            }
            CursorWordRight => {
                self.cursor_word_right(false);
                Vec::new()
            }
            CursorHome => {
                self.cursor_home(false);
                Vec::new()
            }
            CursorEnd => {
                self.cursor_end(false);
                Vec::new()
            }
            SelectLeft => {
                self.cursor_left(true);
                Vec::new()
            }
            SelectRight => {
                self.cursor_right(true);
                Vec::new()
            }
            SelectWordLeft => {
                self.cursor_word_left(true);
                Vec::new()
            }
            SelectWordRight => {
                self.cursor_word_right(true);
                Vec::new()
            }
            SelectHome => {
                self.cursor_home(true);
                Vec::new()
            }
            SelectEnd => {
                self.cursor_end(true);
                Vec::new()
            }
            SelectAll => {
                self.select_all();
                Vec::new()
            }
            Copy => {
                let (effects, copied) = self.copy_selection();
                if let Some(value) = copied {
                    let truncated = if value.len() > 32 {
                        format!("{}…", &value[..32])
                    } else {
                        value
                    };
                    self.push_info(&format!("Copied {truncated}"));
                }
                effects
            }
            Cut => self.cut_selection(),
            Paste(value) => self.paste(value),
            SetTextCursor(offset) => {
                self.move_cursor(offset, false);
                Vec::new()
            }
            ExtendTextSelection(offset) => {
                self.move_cursor(offset, true);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn apply_commit_editor_action(&mut self, action: TextEditAction) -> Vec<Effect> {
        use TextEditAction::*;
        match action {
            InsertText(value) => self.commit_editor.insert_text(&value),
            Backspace => self.commit_editor.delete_backward(),
            BackspaceWord => self.commit_editor.delete_backward_word(),
            BackspaceLine => self.commit_editor.delete_backward_line(),
            DeleteForward => self.commit_editor.delete_forward(),
            DeleteForwardWord => self.commit_editor.delete_forward_word(),
            CursorLeft => self.commit_editor.move_left(false),
            CursorRight => self.commit_editor.move_right(false),
            CursorUp => self.commit_editor.move_up(false),
            CursorDown => self.commit_editor.move_down(false),
            CursorWordLeft => self.commit_editor.move_word_left(false),
            CursorWordRight => self.commit_editor.move_word_right(false),
            CursorHome => self.commit_editor.move_home(false),
            CursorEnd => self.commit_editor.move_end(false),
            CursorSoftHome => self.commit_editor.move_soft_home(false),
            CursorSoftEnd => self.commit_editor.move_soft_end(false),
            SelectLeft => self.commit_editor.move_left(true),
            SelectRight => self.commit_editor.move_right(true),
            SelectUp => self.commit_editor.move_up(true),
            SelectDown => self.commit_editor.move_down(true),
            SelectWordLeft => self.commit_editor.move_word_left(true),
            SelectWordRight => self.commit_editor.move_word_right(true),
            SelectHome => self.commit_editor.move_home(true),
            SelectEnd => self.commit_editor.move_end(true),
            SelectSoftHome => self.commit_editor.move_soft_home(true),
            SelectSoftEnd => self.commit_editor.move_soft_end(true),
            SelectAll => self.commit_editor.select_all(),
            Copy => {
                if let Some(text) = self.commit_editor.selected_text() {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                }
            }
            Cut => {
                if let Some(text) = self.commit_editor.selected_text() {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                    self.commit_editor.delete_backward();
                }
            }
            Paste(value) => self.commit_editor.insert_text(&value),
            _ => {}
        }
        Vec::new()
    }

    fn apply_review_comment_editor_action(&mut self, action: TextEditAction) -> Vec<Effect> {
        use TextEditAction::*;
        match action {
            InsertText(value) => self.review_comment_editor.insert_text(&value),
            Backspace => self.review_comment_editor.delete_backward(),
            BackspaceWord => self.review_comment_editor.delete_backward_word(),
            BackspaceLine => self.review_comment_editor.delete_backward_line(),
            DeleteForward => self.review_comment_editor.delete_forward(),
            DeleteForwardWord => self.review_comment_editor.delete_forward_word(),
            CursorLeft => self.review_comment_editor.move_left(false),
            CursorRight => self.review_comment_editor.move_right(false),
            CursorUp => self.review_comment_editor.move_up(false),
            CursorDown => self.review_comment_editor.move_down(false),
            CursorWordLeft => self.review_comment_editor.move_word_left(false),
            CursorWordRight => self.review_comment_editor.move_word_right(false),
            CursorHome => self.review_comment_editor.move_home(false),
            CursorEnd => self.review_comment_editor.move_end(false),
            CursorSoftHome => self.review_comment_editor.move_soft_home(false),
            CursorSoftEnd => self.review_comment_editor.move_soft_end(false),
            SelectLeft => self.review_comment_editor.move_left(true),
            SelectRight => self.review_comment_editor.move_right(true),
            SelectUp => self.review_comment_editor.move_up(true),
            SelectDown => self.review_comment_editor.move_down(true),
            SelectWordLeft => self.review_comment_editor.move_word_left(true),
            SelectWordRight => self.review_comment_editor.move_word_right(true),
            SelectHome => self.review_comment_editor.move_home(true),
            SelectEnd => self.review_comment_editor.move_end(true),
            SelectSoftHome => self.review_comment_editor.move_soft_home(true),
            SelectSoftEnd => self.review_comment_editor.move_soft_end(true),
            SelectAll => self.review_comment_editor.select_all(),
            Copy => {
                if let Some(text) = self.review_comment_editor.selected_text()
                    && let Ok(mut clipboard) = arboard::Clipboard::new()
                {
                    let _ = clipboard.set_text(text);
                }
            }
            Cut => {
                if let Some(text) = self.review_comment_editor.selected_text()
                    && let Ok(mut clipboard) = arboard::Clipboard::new()
                {
                    let _ = clipboard.set_text(text);
                    self.review_comment_editor.delete_backward();
                }
            }
            Paste(value) => self.review_comment_editor.insert_text(&value),
            _ => {}
        }
        Vec::new()
    }

    fn apply_steering_prompt_action(&mut self, action: TextEditAction) -> Vec<Effect> {
        use TextEditAction::*;
        let mut changed = true;
        match action {
            InsertText(value) => self.steering_prompt_editor.insert_text(&value),
            Backspace => self.steering_prompt_editor.delete_backward(),
            BackspaceWord => self.steering_prompt_editor.delete_backward_word(),
            BackspaceLine => self.steering_prompt_editor.delete_backward_line(),
            DeleteForward => self.steering_prompt_editor.delete_forward(),
            DeleteForwardWord => self.steering_prompt_editor.delete_forward_word(),
            CursorLeft => {
                self.steering_prompt_editor.move_left(false);
                changed = false;
            }
            CursorRight => {
                self.steering_prompt_editor.move_right(false);
                changed = false;
            }
            CursorUp => {
                self.steering_prompt_editor.move_up(false);
                changed = false;
            }
            CursorDown => {
                self.steering_prompt_editor.move_down(false);
                changed = false;
            }
            CursorWordLeft => {
                self.steering_prompt_editor.move_word_left(false);
                changed = false;
            }
            CursorWordRight => {
                self.steering_prompt_editor.move_word_right(false);
                changed = false;
            }
            CursorHome => {
                self.steering_prompt_editor.move_home(false);
                changed = false;
            }
            CursorEnd => {
                self.steering_prompt_editor.move_end(false);
                changed = false;
            }
            CursorSoftHome => {
                self.steering_prompt_editor.move_soft_home(false);
                changed = false;
            }
            CursorSoftEnd => {
                self.steering_prompt_editor.move_soft_end(false);
                changed = false;
            }
            SelectLeft => {
                self.steering_prompt_editor.move_left(true);
                changed = false;
            }
            SelectRight => {
                self.steering_prompt_editor.move_right(true);
                changed = false;
            }
            SelectUp => {
                self.steering_prompt_editor.move_up(true);
                changed = false;
            }
            SelectDown => {
                self.steering_prompt_editor.move_down(true);
                changed = false;
            }
            SelectWordLeft => {
                self.steering_prompt_editor.move_word_left(true);
                changed = false;
            }
            SelectWordRight => {
                self.steering_prompt_editor.move_word_right(true);
                changed = false;
            }
            SelectHome => {
                self.steering_prompt_editor.move_home(true);
                changed = false;
            }
            SelectEnd => {
                self.steering_prompt_editor.move_end(true);
                changed = false;
            }
            SelectSoftHome => {
                self.steering_prompt_editor.move_soft_home(true);
                changed = false;
            }
            SelectSoftEnd => {
                self.steering_prompt_editor.move_soft_end(true);
                changed = false;
            }
            SelectAll => {
                self.steering_prompt_editor.select_all();
                changed = false;
            }
            Copy => {
                changed = false;
                if let Some(text) = self.steering_prompt_editor.selected_text() {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                }
            }
            Cut => {
                if let Some(text) = self.steering_prompt_editor.selected_text() {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                    self.steering_prompt_editor.delete_backward();
                }
            }
            Paste(value) => self.steering_prompt_editor.insert_text(&value),
            _ => changed = false,
        }
        if changed {
            let snapshot = self.steering_prompt_editor.text().to_owned();
            if self.settings.ai_steering_prompt != snapshot {
                self.settings.ai_steering_prompt = snapshot;
                return self.persist_settings_effect();
            }
        }
        Vec::new()
    }
}
