use std::collections::BTreeSet;
use std::sync::Arc;

use halogen::Store;

use crate::core::compare::LayoutMode;
use crate::core::forge::github::GitHubReviewSide;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ViewportTextSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ViewportTextPoint {
    pub line_index: u32,
    pub side: ViewportTextSide,
    pub byte_offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCommentTarget {
    pub path: String,
    pub side: GitHubReviewSide,
    pub line: u32,
    pub start_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportTextSelection {
    pub generation: u64,
    pub anchor: ViewportTextPoint,
    pub focus: ViewportTextPoint,
}

impl ViewportTextSelection {
    pub fn new(generation: u64, point: ViewportTextPoint) -> Self {
        Self {
            generation,
            anchor: point,
            focus: point,
        }
    }

    pub fn normalized(&self) -> (ViewportTextPoint, ViewportTextPoint) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    pub fn contains_point(&self, point: ViewportTextPoint) -> bool {
        if self.is_collapsed() {
            return false;
        }
        let (start, end) = self.normalized();
        point >= start && point <= end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Store)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    /// Shared so per-frame snapshots are pointer bumps, not Vec clones.
    pub matches: Arc<Vec<SearchMatch>>,
    pub active_index: Option<usize>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            matches: Arc::default(),
            active_index: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Store)]
pub struct EditorState {
    pub layout: LayoutMode,
    pub wrap_enabled: bool,
    pub wrap_column: u32,
    /// Compare generation the document geometry below (scroll, content
    /// height, visible rows, hunk/file positions) was last prepared for.
    /// `0` means "no document". Lets prepare re-clamp scroll when a new
    /// compare generation replaces the active document and lets paint guard
    /// against layout/state mismatches without panicking.
    pub doc_generation: u64,
    pub scroll_top_px: u32,
    pub content_height_px: u32,
    pub viewport_width_px: u32,
    pub viewport_height_px: u32,
    pub hovered_row: Option<usize>,
    pub review_add_hovered: bool,
    pub hovered_render_line_index: Option<usize>,
    pub hovered_hunk_index: Option<i16>,
    pub visible_row_start: Option<usize>,
    pub visible_row_end: Option<usize>,
    pub focused: bool,
    pub review_enabled: bool,
    /// Arc-shared so per-frame snapshot reads and `set_if_changed`
    /// write-backs are pointer swaps/compares instead of Vec clones.
    pub hunk_positions: Arc<Vec<u32>>,
    pub file_positions: Arc<Vec<u32>>,
    #[store(flatten)]
    pub search: SearchState,
    pub search_match_y_positions: Arc<Vec<u32>>,
    pub line_selection: LineSelection,
    pub text_selection: Option<ViewportTextSelection>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineSelection {
    pub entries: BTreeSet<LineSelectionKey>,
    pub last_toggled_row: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LineSelectionKey {
    pub file_path: Option<String>,
    pub hunk_id: u32,
    pub side: carbon::DiffSide,
    pub source_index: u32,
}

impl LineSelection {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.last_toggled_row = None;
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, hunk_id: u32, side: carbon::DiffSide, source_index: u32) -> bool {
        self.contains_in_file(None, hunk_id, side, source_index)
    }

    pub fn contains_in_file(
        &self,
        file_path: Option<&str>,
        hunk_id: u32,
        side: carbon::DiffSide,
        source_index: u32,
    ) -> bool {
        self.entries.iter().any(|key| {
            key.hunk_id == hunk_id
                && key.side == side
                && key.source_index == source_index
                && match key.file_path.as_deref() {
                    Some(path) => file_path == Some(path),
                    None => true,
                }
        })
    }

    pub fn toggle(&mut self, hunk_id: u32, side: carbon::DiffSide, source_index: u32) {
        let key = LineSelectionKey {
            file_path: None,
            hunk_id,
            side,
            source_index,
        };
        if !self.entries.remove(&key) {
            self.entries.insert(key);
        }
    }

    pub fn selected_lines_for_hunk(&self, hunk_id: u32) -> Vec<LineSelectionKey> {
        self.entries
            .iter()
            .filter(|key| key.hunk_id == hunk_id)
            .cloned()
            .collect()
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            layout: LayoutMode::Unified,
            wrap_enabled: false,
            wrap_column: 0,
            doc_generation: 0,
            scroll_top_px: 0,
            content_height_px: 0,
            viewport_width_px: 0,
            viewport_height_px: 0,
            hovered_row: None,
            review_add_hovered: false,
            hovered_render_line_index: None,
            hovered_hunk_index: None,
            visible_row_start: None,
            visible_row_end: None,
            focused: false,
            review_enabled: false,
            hunk_positions: Arc::default(),
            file_positions: Arc::default(),
            search: SearchState::default(),
            search_match_y_positions: Arc::default(),
            line_selection: LineSelection::default(),
            text_selection: None,
        }
    }
}

impl EditorState {
    pub fn clear_document(&mut self) {
        self.doc_generation = 0;
        self.scroll_top_px = 0;
        self.content_height_px = 0;
        self.hovered_row = None;
        self.review_add_hovered = false;
        self.hovered_render_line_index = None;
        self.hovered_hunk_index = None;
        self.visible_row_start = None;
        self.visible_row_end = None;
        self.review_enabled = false;
        // Swap in empty Arcs (only when non-empty, to avoid churning
        // allocations when this runs every frame without a document).
        if !self.hunk_positions.is_empty() {
            self.hunk_positions = Arc::default();
        }
        if !self.file_positions.is_empty() {
            self.file_positions = Arc::default();
        }
        if !self.search_match_y_positions.is_empty() {
            self.search_match_y_positions = Arc::default();
        }
        self.line_selection.clear();
        self.text_selection = None;
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
