use std::collections::BTreeSet;

use crate::core::compare::LayoutMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    pub line_index: u32,
    pub byte_start: u32,
    pub byte_len: u32,
    pub side: MatchSide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchSide {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    pub matches: Vec<SearchMatch>,
    pub active_index: Option<usize>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            matches: Vec::new(),
            active_index: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorState {
    pub layout: LayoutMode,
    pub wrap_enabled: bool,
    pub wrap_column: u32,
    pub scroll_top_px: u32,
    pub content_height_px: u32,
    pub viewport_width_px: u32,
    pub viewport_height_px: u32,
    pub hovered_row: Option<usize>,
    pub visible_row_start: Option<usize>,
    pub visible_row_end: Option<usize>,
    pub focused: bool,
    pub hunk_positions: Vec<u32>,
    pub file_positions: Vec<u32>,
    pub search: SearchState,
    pub search_match_y_positions: Vec<u32>,
    pub line_selection: LineSelection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineSelection {
    pub entries: BTreeSet<(i16, i16)>,
    pub last_toggled_row: Option<usize>,
}

impl LineSelection {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.last_toggled_row = None;
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, hunk_index: i16, line_index: i16) -> bool {
        self.entries.contains(&(hunk_index, line_index))
    }

    pub fn toggle(&mut self, hunk_index: i16, line_index: i16) {
        let key = (hunk_index, line_index);
        if !self.entries.remove(&key) {
            self.entries.insert(key);
        }
    }

    pub fn selected_lines_for_hunk(&self, hunk_index: i16) -> Vec<usize> {
        self.entries
            .iter()
            .filter(|(h, _)| *h == hunk_index)
            .map(|(_, l)| *l as usize)
            .collect()
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            layout: LayoutMode::Unified,
            wrap_enabled: false,
            wrap_column: 0,
            scroll_top_px: 0,
            content_height_px: 0,
            viewport_width_px: 0,
            viewport_height_px: 0,
            hovered_row: None,
            visible_row_start: None,
            visible_row_end: None,
            focused: false,
            hunk_positions: Vec::new(),
            file_positions: Vec::new(),
            search: SearchState::default(),
            search_match_y_positions: Vec::new(),
            line_selection: LineSelection::default(),
        }
    }
}

impl EditorState {
    pub fn clear_document(&mut self) {
        self.scroll_top_px = 0;
        self.content_height_px = 0;
        self.hovered_row = None;
        self.visible_row_start = None;
        self.visible_row_end = None;
        self.hunk_positions.clear();
        self.file_positions.clear();
        self.search_match_y_positions.clear();
        self.line_selection.clear();
    }

    pub fn max_scroll_top_px(&self) -> u32 {
        self.content_height_px
            .saturating_sub(self.viewport_height_px.max(1))
    }

    pub fn clamp_scroll(&mut self) {
        self.scroll_top_px = self.scroll_top_px.min(self.max_scroll_top_px());
    }

    pub fn current_hunk_index(&self) -> Option<(usize, usize)> {
        if self.hunk_positions.is_empty() {
            return None;
        }
        let scroll = self.scroll_top_px;
        let idx = self
            .hunk_positions
            .partition_point(|&y| y <= scroll)
            .saturating_sub(1);
        Some((idx, self.hunk_positions.len()))
    }
}
