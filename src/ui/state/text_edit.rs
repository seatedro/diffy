use unicode_segmentation::UnicodeSegmentation;

use crate::effects::Effect;

use super::{AppState, CompareField, FocusTarget, PickerKind};

impl AppState {
    pub(super) fn selection_range(&self) -> Option<(usize, usize)> {
        let (c, a) = (self.text_edit.cursor, self.text_edit.anchor);
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
            if let Some(text) = self.focused_text_mut() {
                text.drain(start..end);
            }
            self.text_edit.cursor = start;
            self.text_edit.anchor = start;
            true
        } else {
            false
        }
    }

    /// Called after text mutation to sync compare fields and rebuild pickers.
    pub(super) fn after_text_mutation(&mut self) -> Vec<Effect> {
        match self.focus.get(&self.store) {
            Some(FocusTarget::PickerInput) => match self.overlays.picker.kind {
                PickerKind::Repository => self.rebuild_repo_picker(),
                PickerKind::LeftRef => {
                    self.compare.resolved_left = None;
                    return self.rebuild_ref_picker(CompareField::Left);
                }
                PickerKind::RightRef => {
                    self.compare.resolved_right = None;
                    return self.rebuild_ref_picker(CompareField::Right);
                }
                PickerKind::Theme => self.rebuild_theme_picker(),
            },
            Some(FocusTarget::CommandPaletteInput) => self.rebuild_command_palette(),
            Some(FocusTarget::SearchInput) => self.recompute_search_matches(),
            _ => {}
        }
        Vec::new()
    }

    /// Should we persist settings after editing the current field?
    pub(super) fn needs_persist(&self) -> bool {
        matches!(
            self.focus.get(&self.store),
            Some(FocusTarget::PickerInput)
                if matches!(self.overlays.picker.kind, PickerKind::LeftRef | PickerKind::RightRef)
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
        if self.focused_text().is_none() {
            return Vec::new();
        }
        self.delete_selection();
        self.clamp_cursor();
        let cursor = self.text_edit.cursor;
        if let Some(text) = self.focused_text_mut() {
            text.insert_str(cursor, &value);
        }
        self.text_edit.cursor += value.len();
        self.text_edit.anchor = self.text_edit.cursor;
        self.touch_cursor();
        self.text_edit_effects()
    }

    pub(super) fn backspace(&mut self) -> Vec<Effect> {
        if self.focused_text().is_none() {
            return Vec::new();
        }
        if self.delete_selection() {
            self.touch_cursor();
            return self.text_edit_effects();
        }
        let cursor = self.text_edit.cursor;
        if cursor == 0 {
            return Vec::new();
        }
        let prev = self
            .focused_text()
            .map(|t| prev_grapheme_boundary(t, cursor))
            .unwrap_or(0);
        if let Some(text) = self.focused_text_mut() {
            text.drain(prev..cursor);
        }
        self.text_edit.cursor = prev;
        self.text_edit.anchor = prev;
        self.touch_cursor();
        self.text_edit_effects()
    }

    pub(super) fn delete_forward(&mut self) -> Vec<Effect> {
        if self.focused_text().is_none() {
            return Vec::new();
        }
        if self.delete_selection() {
            self.touch_cursor();
            return self.text_edit_effects();
        }
        let cursor = self.text_edit.cursor;
        let len = self.focused_text().map_or(0, |s| s.len());
        if cursor >= len {
            return Vec::new();
        }
        let next = self
            .focused_text()
            .map(|t| next_grapheme_boundary(t, cursor))
            .unwrap_or(cursor);
        if let Some(text) = self.focused_text_mut() {
            text.drain(cursor..next);
        }
        self.touch_cursor();
        self.text_edit_effects()
    }

    pub(super) fn move_cursor(&mut self, offset: usize, extend_selection: bool) {
        self.text_edit.cursor = offset;
        if !extend_selection {
            self.text_edit.anchor = offset;
        }
        self.touch_cursor();
    }

    pub(super) fn cursor_left(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let start = self.text_edit.cursor.min(self.text_edit.anchor);
            self.move_cursor(start, false);
            return;
        }
        let cursor = self.text_edit.cursor;
        if cursor == 0 {
            return;
        }
        let prev = self
            .focused_text()
            .map(|t| prev_grapheme_boundary(t, cursor))
            .unwrap_or(0);
        self.move_cursor(prev, extend);
    }

    pub(super) fn cursor_right(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let end = self.text_edit.cursor.max(self.text_edit.anchor);
            self.move_cursor(end, false);
            return;
        }
        let cursor = self.text_edit.cursor;
        let len = self.focused_text().map_or(0, |s| s.len());
        if cursor >= len {
            return;
        }
        let next = self
            .focused_text()
            .map(|t| next_grapheme_boundary(t, cursor))
            .unwrap_or(cursor);
        self.move_cursor(next, extend);
    }

    pub(super) fn cursor_word_left(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let start = self.text_edit.cursor.min(self.text_edit.anchor);
            self.move_cursor(start, false);
            return;
        }
        let cursor = self.text_edit.cursor;
        let pos = self
            .focused_text()
            .map(|t| prev_word_boundary(t, cursor))
            .unwrap_or(0);
        self.move_cursor(pos, extend);
    }

    pub(super) fn cursor_word_right(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let end = self.text_edit.cursor.max(self.text_edit.anchor);
            self.move_cursor(end, false);
            return;
        }
        let cursor = self.text_edit.cursor;
        let len = self.focused_text().map_or(0, |s| s.len());
        let pos = self
            .focused_text()
            .map(|t| next_word_boundary(t, cursor))
            .unwrap_or(len);
        self.move_cursor(pos, extend);
    }

    pub(super) fn cursor_home(&mut self, extend: bool) {
        self.move_cursor(0, extend);
    }

    pub(super) fn cursor_end(&mut self, extend: bool) {
        let len = self.focused_text().map_or(0, |s| s.len());
        self.move_cursor(len, extend);
    }

    pub(super) fn select_all(&mut self) {
        let len = self.focused_text().map_or(0, |s| s.len());
        self.text_edit.anchor = 0;
        self.text_edit.cursor = len;
        self.touch_cursor();
    }

    /// Copy text selection or, if none, the selected overlay entry.
    /// Returns `(effects, Some(value))` when copying an entry (toast-worthy).
    pub(super) fn copy_selection(&self) -> (Vec<Effect>, Option<String>) {
        if let Some((start, end)) = self.selection_range() {
            if let Some(text) = self.focused_text() {
                let selected = text[start..end].to_string();
                return (vec![Effect::SetClipboard(selected)], None);
            }
        }
        // No text selection — copy the selected picker/palette entry's value.
        if matches!(self.focus.get(&self.store), Some(FocusTarget::PickerInput)) {
            if let Some(entry) = self
                .overlays
                .picker
                .entries
                .get(self.overlays.picker.selected_index)
            {
                return (
                    vec![Effect::SetClipboard(entry.value.clone())],
                    Some(entry.value.clone()),
                );
            }
        }
        if matches!(self.focus.get(&self.store), Some(FocusTarget::CommandPaletteInput)) {
            if let Some(entry) = self
                .overlays
                .command_palette
                .entries
                .get(self.overlays.command_palette.selected_index)
            {
                return (
                    vec![Effect::SetClipboard(entry.label.clone())],
                    Some(entry.label.clone()),
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
