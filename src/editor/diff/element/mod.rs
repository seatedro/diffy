use std::{collections::HashMap, ops::Range, sync::Arc};

use crate::render::{Rect, RichTextSpan, TextMetrics};
use crate::ui::theme::{Color, Theme};

use super::decoration::BlockRegistry;
use super::display_layout::{DisplayLayoutConfig, DisplayLayoutMetrics, DisplayLayoutSummary};
use super::render_doc::{
    ByteRange, DisplayRow, INVALID_U32, RenderDoc, RunRange, advance_display_col,
};
use super::state::{SearchMatch, ViewportTextSide};
use super::strip_layout::StripLayout;

mod hit_test;
mod layout;
mod paint;
#[cfg(test)]
mod tests;

use paint::{RowTone, build_wrapped_rich_text};

pub(crate) const BASE_MONO_FONT_SIZE: f32 = 13.0;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EditorLayout {
    pub outer_bounds: Rect,
    pub content_bounds: Rect,
    pub split_mode: bool,
    pub gutter_digits: u32,
    pub unified_gutter_rect: Rect,
    pub unified_text_rect: Rect,
    pub left_gutter_rect: Rect,
    pub left_text_rect: Rect,
    pub right_gutter_rect: Rect,
    pub right_text_rect: Rect,

    pub line_height: f32,
    pub font_size: f32,
    pub text_y_offset: f32,
    pub gutter_padding: f32,
    pub column_gap: f32,
    pub scroll_top_px: f32,
    pub visible_row_range: VisibleRange,
    pub highlighted_row: Option<usize>,
    pub scrollbar: Option<ScrollbarLayout>,
    pub show_staging_controls: bool,
    pub file_is_staged: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VisibleRange {
    pub start: usize,
    pub end: usize,
}

impl VisibleRange {
    pub fn iter(&self) -> Range<usize> {
        self.start..self.end
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScrollbarLayout {
    pub track: Rect,
    pub thumb: Rect,
    pub thumb_top: f32,
    pub thumb_height: f32,
}

#[derive(Debug, Clone, Copy)]
pub enum EditorDocument<'a> {
    Empty,
    Loading {
        path: &'a str,
    },
    Binary {
        path: &'a str,
    },
    Text {
        compare_generation: u64,
        file_index: usize,
        path: &'a str,
        doc: &'a RenderDoc,
        show_file_headers: bool,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct EditorLayoutKey {
    compare_generation: u64,
    file_index: usize,
    show_file_headers: bool,
    split_mode: bool,
    wrap_enabled: bool,
    wrap_column: u32,
    viewport_width_bits: u32,
    viewport_height_bits: u32,
    mono_char_width_bits: u32,
    mono_line_height_bits: u32,
    doc_line_count: u32,
    doc_text_len: u32,
    block_layout_signature: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EditorThemeKey {
    text_strong: Color,
    text_muted: Color,
    accent: Color,
    line_add_text: Color,
    line_del_text: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct WrappedTextCacheKey {
    text_start: u32,
    text_len: u32,
    runs_start: u32,
    runs_len: u32,
    segment_index: u16,
    wrap_cols: u16,
    tone: RowTone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GutterTextCacheKey {
    old_line_no: u32,
    new_line_no: u32,
    digits: u32,
    kind: GutterTextKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum GutterTextKind {
    SplitLeft,
    SplitRight,
    Unified,
    UnifiedOldOnly,
    UnifiedNewOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TextLayoutCacheKey {
    text_start: u32,
    text_len: u32,
}

#[derive(Debug, Clone)]
struct CachedTextLayout {
    char_boundaries: Arc<[u32]>,
    col_boundaries: Arc<[u32]>,
}

impl CachedTextLayout {
    fn new(text: &str) -> Self {
        let mut char_boundaries = Vec::with_capacity(text.chars().count().saturating_add(1));
        let mut col_boundaries = Vec::with_capacity(text.chars().count().saturating_add(1));
        let mut cols = 0_u32;

        for (idx, ch) in text.char_indices() {
            char_boundaries.push(idx as u32);
            col_boundaries.push(cols);
            cols = advance_display_col(cols, ch);
        }
        char_boundaries.push(text.len() as u32);
        col_boundaries.push(cols);
        Self {
            char_boundaries: Arc::from(char_boundaries),
            col_boundaries: Arc::from(col_boundaries),
        }
    }

    fn char_count(&self) -> u32 {
        self.char_boundaries.len().saturating_sub(1) as u32
    }

    fn total_cols(&self) -> u32 {
        self.col_boundaries.last().copied().unwrap_or(0)
    }

    fn char_range_for_cols(&self, start_col: u32, end_col: u32) -> (usize, usize) {
        let total_cols = self.total_cols();
        let start_col = start_col.min(total_cols);
        let end_col = end_col.min(total_cols).max(start_col);
        let char_count = self.char_count() as usize;
        let start = self
            .col_boundaries
            .partition_point(|boundary| *boundary <= start_col)
            .saturating_sub(1)
            .min(char_count);
        let end = self
            .col_boundaries
            .partition_point(|boundary| *boundary < end_col)
            .min(char_count);
        (start, end.max(start))
    }

    #[cfg(test)]
    fn byte_range_for_cols(&self, start_col: u32, end_col: u32) -> (usize, usize) {
        let (start, end) = self.char_range_for_cols(start_col, end_col);
        (
            self.char_boundaries[start] as usize,
            self.char_boundaries[end] as usize,
        )
    }

    fn col_for_byte(&self, byte: usize) -> u32 {
        let byte = (byte as u32).min(self.char_boundaries.last().copied().unwrap_or(0));
        let idx = self
            .char_boundaries
            .partition_point(|boundary| *boundary <= byte)
            .saturating_sub(1);
        self.col_boundaries.get(idx).copied().unwrap_or(0)
    }

    fn byte_for_col_nearest(&self, col: f32) -> u32 {
        let Some(&last_byte) = self.char_boundaries.last() else {
            return 0;
        };
        let Some(&last_col) = self.col_boundaries.last() else {
            return 0;
        };
        if col <= 0.0 {
            return 0;
        }
        if col >= last_col as f32 {
            return last_byte;
        }

        let upper = self
            .col_boundaries
            .partition_point(|boundary| (*boundary as f32) < col)
            .min(self.col_boundaries.len().saturating_sub(1));
        let lower = upper.saturating_sub(1);
        let lower_col = self.col_boundaries[lower] as f32;
        let upper_col = self.col_boundaries[upper] as f32;
        if (col - lower_col).abs() <= (upper_col - col).abs() {
            self.char_boundaries[lower]
        } else {
            self.char_boundaries[upper]
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbarOverride {
    pub total_height_px: u32,
    pub scroll_top_px: u32,
    pub max_scroll_top_px: u32,
}

#[derive(Debug)]
pub struct EditorElement {
    layout_key: Option<EditorLayoutKey>,
    pub layout: EditorLayout,
    config: DisplayLayoutConfig,
    metrics: DisplayLayoutMetrics,
    summary: DisplayLayoutSummary,
    rows: Vec<DisplayRow>,
    strips: Vec<StripLayout>,
    blocks: BlockRegistry,
    hunk_expand_caps: Vec<super::expansion::HunkGapBudget>,
    theme_cache_key: Option<EditorThemeKey>,
    wrapped_text_cache: HashMap<WrappedTextCacheKey, Arc<[RichTextSpan]>>,
    text_layout_cache: HashMap<TextLayoutCacheKey, Arc<CachedTextLayout>>,
    gutter_text_cache: HashMap<GutterTextCacheKey, Arc<str>>,
    text_metrics: TextMetrics,
    scrollbar_override: Option<ScrollbarOverride>,
    sticky_header_hit: Option<(Rect, String)>,
    file_header_hits: Vec<FileHeaderHit>,
    mouse_pos: Option<(f32, f32)>,
    /// Memoized navigation positions. Hunk/file positions depend only on
    /// `rows` (rebuilt when `layout_key` changes); search Y positions
    /// additionally depend on the search match set. Recomputing these every
    /// frame meant iterating every row per frame, so cache and hand out
    /// shared Arcs instead.
    nav_positions_valid: bool,
    nav_hunk_positions: Arc<Vec<u32>>,
    nav_file_positions: Arc<Vec<u32>>,
    nav_search_matches: Option<Arc<Vec<SearchMatch>>>,
    nav_search_y_positions: Arc<Vec<u32>>,
}

#[derive(Debug, Clone)]
struct FileHeaderHit {
    y_px: u32,
    h_px: u16,
    path: String,
}

#[derive(Debug, Clone, Copy)]
struct TextBlock {
    line_index: u32,
    side: ViewportTextSide,
    text_range: ByteRange,
    text_x: f32,
    text_width: f32,
    y: f32,
    segment_count: u16,
    segment_cols: u16,
}

impl Default for EditorElement {
    fn default() -> Self {
        Self {
            layout_key: None,
            layout: EditorLayout::default(),
            config: DisplayLayoutConfig::default(),
            metrics: DisplayLayoutMetrics::default(),
            summary: DisplayLayoutSummary::default(),
            rows: Vec::new(),
            strips: Vec::new(),
            blocks: BlockRegistry::new(),
            hunk_expand_caps: Vec::new(),
            theme_cache_key: None,
            wrapped_text_cache: HashMap::new(),
            text_layout_cache: HashMap::new(),
            gutter_text_cache: HashMap::new(),
            text_metrics: TextMetrics::default(),
            scrollbar_override: None,
            sticky_header_hit: None,
            file_header_hits: Vec::new(),
            mouse_pos: None,
            nav_positions_valid: false,
            nav_hunk_positions: Arc::default(),
            nav_file_positions: Arc::default(),
            nav_search_matches: None,
            nav_search_y_positions: Arc::default(),
        }
    }
}

impl EditorElement {
    pub fn set_scrollbar_override(&mut self, value: Option<ScrollbarOverride>) {
        self.scrollbar_override = value;
    }

    pub fn set_mouse_pos(&mut self, pos: Option<(f32, f32)>) {
        self.mouse_pos = pos;
    }

    pub fn scrollbar_layout(&self) -> Option<ScrollbarLayout> {
        self.layout.scrollbar
    }

    pub fn scroll_line_height_px(&self) -> f32 {
        let lh = self.layout.line_height;
        if lh > 0.0 { lh } else { 20.0 }
    }

    pub fn blocks_mut(&mut self) -> &mut BlockRegistry {
        &mut self.blocks
    }

    pub fn blocks(&self) -> &BlockRegistry {
        &self.blocks
    }

    pub fn set_hunk_expand_caps(&mut self, caps: Vec<super::expansion::HunkGapBudget>) {
        self.hunk_expand_caps = caps;
    }

    fn clear_document_caches(&mut self) {
        self.wrapped_text_cache.clear();
        self.text_layout_cache.clear();
        self.gutter_text_cache.clear();
        // Rows changed (or went away) — navigation positions must be
        // recomputed from the new row geometry.
        self.nav_positions_valid = false;
        self.nav_search_matches = None;
    }

    fn sync_theme_cache(&mut self, theme: &Theme) {
        let key = EditorThemeKey {
            text_strong: theme.colors.text_strong,
            text_muted: theme.colors.text_muted,
            accent: theme.colors.accent,
            line_add_text: theme.colors.line_add_text,
            line_del_text: theme.colors.line_del_text,
        };
        if self.theme_cache_key != Some(key) {
            self.wrapped_text_cache.clear();
            self.theme_cache_key = Some(key);
        }
    }

    fn cached_wrapped_rich_text(
        &mut self,
        doc: &RenderDoc,
        text_range: ByteRange,
        runs: RunRange,
        segment_index: u16,
        wrap_cols: u16,
        tone: RowTone,
        theme: &Theme,
    ) -> Option<Arc<[RichTextSpan]>> {
        if !text_range.is_valid() {
            return None;
        }
        let key = WrappedTextCacheKey {
            text_start: text_range.start,
            text_len: text_range.len,
            runs_start: runs.start,
            runs_len: runs.len,
            segment_index,
            wrap_cols,
            tone,
        };
        if let Some(cached) = self.wrapped_text_cache.get(&key) {
            return Some(cached.clone());
        }

        let text_layout = self.cached_text_layout(doc, text_range);
        let spans = build_wrapped_rich_text(
            doc,
            text_layout.as_ref(),
            text_range,
            runs,
            segment_index,
            wrap_cols,
            tone,
            theme,
        )?;
        self.wrapped_text_cache.insert(key, spans.clone());
        Some(spans)
    }

    fn cached_text_layout(
        &mut self,
        doc: &RenderDoc,
        text_range: ByteRange,
    ) -> Arc<CachedTextLayout> {
        let key = TextLayoutCacheKey {
            text_start: text_range.start,
            text_len: text_range.len,
        };
        if let Some(cached) = self.text_layout_cache.get(&key) {
            return cached.clone();
        }

        let layout = Arc::new(CachedTextLayout::new(doc.line_text(text_range)));
        self.text_layout_cache.insert(key, layout.clone());
        layout
    }

    fn cached_gutter_text(&mut self, key: GutterTextCacheKey) -> Arc<str> {
        if let Some(cached) = self.gutter_text_cache.get(&key) {
            return cached.clone();
        }

        let spaces = " ".repeat(key.digits as usize);
        let text: Arc<str> = match key.kind {
            GutterTextKind::SplitLeft => format_line_number_string(key.old_line_no, key.digits),
            GutterTextKind::SplitRight => format_line_number_string(key.new_line_no, key.digits),
            GutterTextKind::Unified => format!(
                "{} {}",
                format_line_number_string(key.old_line_no, key.digits),
                format_line_number_string(key.new_line_no, key.digits)
            ),
            GutterTextKind::UnifiedOldOnly => format!(
                "{} {}",
                format_line_number_string(key.old_line_no, key.digits),
                spaces
            ),
            GutterTextKind::UnifiedNewOnly => format!(
                "{} {}",
                spaces,
                format_line_number_string(key.new_line_no, key.digits)
            ),
        }
        .into();
        self.gutter_text_cache.insert(key, text.clone());
        text
    }
}

fn format_line_number_string(line_no: u32, digits: u32) -> String {
    if line_no == INVALID_U32 {
        " ".repeat(digits as usize)
    } else {
        format!("{line_no:>width$}", width = digits as usize)
    }
}
