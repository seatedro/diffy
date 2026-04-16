use unicode_segmentation::UnicodeSegmentation;

use crate::effects::Effect;

use super::{AppState, CompareField, FocusTarget, PickerKind};

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
                return (vec![Effect::SetClipboard(selected)], None);
            }
        }
        // No text selection — copy the selected picker/palette entry's value.
        if matches!(self.focus.get(&self.store), Some(FocusTarget::PickerInput)) {
            let selected = self.overlays.picker.selected_index.get(&self.store);
            let value = self
                .overlays
                .picker
                .entries
                .with(&self.store, |entries| {
                    entries.get(selected).map(|e| e.value.clone())
                });
            if let Some(value) = value {
                return (
                    vec![Effect::SetClipboard(value.clone())],
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
                    vec![Effect::SetClipboard(label.clone())],
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
