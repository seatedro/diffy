use std::{collections::HashMap, ops::Range, sync::Arc};

use crate::actions::{Action, ContextMenuEntry};
use crate::core::compare::LayoutMode;
use crate::core::text::SyntaxTokenKind;
use crate::render::scene::{IconPrimitive, Primitive};
use crate::render::{
    FontKind, FontStyle, FontWeight, Rect, RectPrimitive, RichTextPrimitive, RichTextSpan,
    RoundedRectPrimitive, Scene, TextMetrics, TextPrimitive,
};
use crate::ui::accessibility::{AccessibilityAction, AccessibilityFrame, AccessibilityNode};
use crate::ui::design::{Alpha, Sz};
use crate::ui::element::ScrollActionBuilder;
use crate::ui::icons::lucide;
use crate::ui::state::FocusTarget;
use crate::ui::theme::{Color, Theme};
use accesskit::Role;

use super::decoration::{
    BlockActionCtx, BlockPaintCtx, BlockRegistry, FileHeaderDecoration, RowDecoration, RowPaintCtx,
    decoration_for_kind,
};
use super::display_layout::{
    DisplayLayoutConfig, DisplayLayoutMetrics, DisplayLayoutSummary, compute_gutter_digits,
    rebuild_display_rows,
};
use super::render_doc::{
    ByteRange, DisplayRow, FileHeaderMeta, INVALID_U32, RENDER_FLAG_STRUCTURAL, RenderDoc,
    RenderLine, RenderRowKind, RunRange, STYLE_FLAG_CHANGE, STYLE_FLAG_UNCHANGED_CTX, StyleRun,
    advance_display_col,
};
use super::state::{EditorState, ViewportTextPoint, ViewportTextSelection, ViewportTextSide};
use super::strip_layout::{StripLayout, build_strip_layouts, visible_strip_range};

const BASE_VIEWPORT_PADDING: f32 = 14.0;
const BASE_COLUMN_GAP: f32 = 18.0;
const BASE_GUTTER_PADDING: f32 = 8.0;
const BASE_SCROLLBAR_WIDTH: f32 = 8.0;
const BASE_SCROLLBAR_MARGIN: f32 = 6.0;
const FILE_HEADER_ROW_MULTIPLE: u16 = 1;
const HUNK_ROW_MULTIPLE: u16 = 1;
const BASE_SCROLLBAR_THUMB_MIN: f32 = 32.0;
pub(crate) const BASE_MONO_FONT_SIZE: f32 = 13.0;
const STRIP_TARGET_HEIGHT_PX: u32 = 480;
const STRIP_OVERSCAN: usize = 1;
const UNWRAPPED_RENDER_OVERSCAN_COLS: u16 = 16;
const STICKY_HEADER_Z: i32 = 10;
const INLINE_CHANGE_BG_MERGE_GAP_COLS: u32 = 2;
const INLINE_CHANGE_BG_Y_INSET_RATIO: f32 = 0.10;

fn editor_scale(text_metrics: TextMetrics) -> f32 {
    (text_metrics.mono_font_size_px / BASE_MONO_FONT_SIZE).max(0.5)
}

fn display_layout_metrics(text_metrics: TextMetrics) -> DisplayLayoutMetrics {
    let body_h = text_metrics.mono_line_height_px.round().max(1.0) as u16;
    DisplayLayoutMetrics {
        body_row_height_px: body_h,
        file_header_height_px: body_h * FILE_HEADER_ROW_MULTIPLE,
        hunk_height_px: body_h * HUNK_ROW_MULTIPLE,
    }
}

fn scaled(base: f32, scale: f32) -> f32 {
    base * scale
}

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

    pub fn prepare(
        &mut self,
        state: &mut EditorState,
        document: EditorDocument<'_>,
        bounds: Rect,
        text_metrics: TextMetrics,
    ) -> EditorLayout {
        self.text_metrics = text_metrics;
        let gutter_digits = match document {
            EditorDocument::Text { doc, .. } => compute_gutter_digits(doc),
            _ => 3,
        };
        self.layout = build_spatial_layout(bounds, state.layout, gutter_digits, text_metrics);
        state.viewport_width_px = self.layout.content_bounds.width.max(0.0).round() as u32;
        state.viewport_height_px = self.layout.content_bounds.height.max(0.0).round() as u32;

        let s = editor_scale(text_metrics);
        self.layout.font_size = text_metrics.mono_font_size_px;
        self.layout.line_height = self.metrics.body_row_height_px as f32;
        let glyphon_line_h = text_metrics.mono_font_size_px * 1.35;
        self.layout.text_y_offset = ((self.layout.line_height - glyphon_line_h) * 0.5).max(0.0);
        self.layout.gutter_padding = scaled(BASE_GUTTER_PADDING, s);
        self.layout.column_gap = scaled(BASE_COLUMN_GAP, s);

        match document {
            EditorDocument::Text {
                compare_generation,
                file_index,
                doc,
                show_file_headers,
                ..
            } => {
                let key = EditorLayoutKey {
                    compare_generation,
                    file_index,
                    show_file_headers,
                    split_mode: state.layout == LayoutMode::Split,
                    wrap_enabled: state.wrap_enabled,
                    wrap_column: state.wrap_column,
                    viewport_width_bits: self.layout.content_bounds.width.to_bits(),
                    viewport_height_bits: self.layout.content_bounds.height.to_bits(),
                    mono_char_width_bits: text_metrics.mono_char_width_px.to_bits(),
                    mono_line_height_bits: text_metrics.mono_line_height_px.to_bits(),
                    doc_line_count: doc.line_count() as u32,
                    doc_text_len: doc.text_bytes.len().min(u32::MAX as usize) as u32,
                    block_layout_signature: self
                        .blocks
                        .layout_signature(display_layout_metrics(text_metrics)),
                };

                if self.layout_key != Some(key) {
                    self.rebuild_rows(doc, state, text_metrics, show_file_headers);
                    self.clear_document_caches();
                    self.layout_key = Some(key);
                }

                state.content_height_px = self.summary.content_height_px;
                self.rebuild_navigation_positions(state);
                state.clamp_scroll();
                self.update_visible_ranges(state);

                self.layout.line_height = self.metrics.body_row_height_px as f32;
            }
            _ => {
                self.layout_key = None;
                self.rows.clear();
                self.strips.clear();
                self.clear_document_caches();
                state.clear_document();
            }
        }

        self.layout.scroll_top_px = state.scroll_top_px as f32;
        self.layout.highlighted_row = state.hovered_row;
        self.layout.scrollbar =
            compute_scrollbar_layout(&self.layout, state, self.scrollbar_override);

        self.layout
    }

    pub fn body_bounds(&self) -> Rect {
        self.layout.content_bounds
    }

    pub fn hit_test_row(&self, state: &EditorState, x: f32, y: f32) -> Option<usize> {
        if !self.layout.content_bounds.contains(x, y) {
            return None;
        }
        let content_y = (y - self.layout.content_bounds.y).max(0.0) + state.scroll_top_px as f32;
        let index = self
            .rows
            .partition_point(|row| row.bottom_px() as f32 <= content_y);
        self.rows.get(index).and_then(|row| {
            (content_y >= row.y_px as f32 && content_y < row.bottom_px() as f32).then_some(index)
        })
    }

    pub fn is_gutter_hit(&self, x: f32, _y: f32) -> bool {
        if self.layout.split_mode {
            self.layout
                .left_gutter_rect
                .contains(x, self.layout.left_gutter_rect.y)
        } else {
            self.layout
                .unified_gutter_rect
                .contains(x, self.layout.unified_gutter_rect.y)
        }
    }

    pub fn blocks_mut(&mut self) -> &mut BlockRegistry {
        &mut self.blocks
    }

    pub fn blocks(&self) -> &BlockRegistry {
        &self.blocks
    }

    /// Pins the single card-width formula used by the review-thread overlay. Mirrors
    /// the inset `build_spatial_layout` applies to produce `content_bounds.width`, so a
    /// caller can derive the width BEFORE `prepare` runs (it needs the width to measure
    /// card heights before blocks are populated). `text_metrics` is passed explicitly
    /// rather than read from `self` so it does not depend on prepare-ordering.
    pub fn content_width_for_bounds(&self, bounds: Rect, text_metrics: TextMetrics) -> f32 {
        bounds
            .inset(scaled(BASE_VIEWPORT_PADDING, editor_scale(text_metrics)))
            .width
    }

    /// Top edge band occupied by the sticky file header, if any, so overlays can avoid
    /// painting/clicking over it.
    pub fn sticky_header_rect(&self) -> Option<Rect> {
        self.sticky_header_hit.as_ref().map(|(rect, _)| *rect)
    }

    /// One `(block_index, on-screen rect)` per visible review-thread block row, so the
    /// shell can render each thread card as a `view!` overlay at its scrolled position.
    pub fn visible_review_card_rows(&self) -> Vec<(usize, Rect)> {
        let mut out = Vec::new();
        for row in &self.rows {
            if !row.is_block() {
                continue;
            }
            let rect = self.row_rect_for(row);
            if !self.row_in_viewport(&rect) {
                continue;
            }
            out.push((row.block_index as usize, rect));
        }
        out
    }

    pub fn set_hunk_expand_caps(&mut self, caps: Vec<super::expansion::HunkGapBudget>) {
        self.hunk_expand_caps = caps;
    }

    pub fn render_line_index_for_row(&self, display_row_index: usize) -> Option<u32> {
        let row = self.rows.get(display_row_index)?;
        if row.is_block() {
            return None;
        }
        Some(row.line_index)
    }

    pub fn is_block_row(&self, display_row_index: usize) -> bool {
        self.rows
            .get(display_row_index)
            .is_some_and(|row| row.is_block())
    }

    pub fn review_comment_line_for_row(
        &self,
        doc: &RenderDoc,
        display_row_index: usize,
    ) -> Option<usize> {
        let row = self.rows.get(display_row_index)?;
        if row.is_block() {
            return None;
        }
        let line = doc.lines.get(row.line_index as usize)?;
        review_comment_gutter_rect(&self.layout, line)?;
        Some(row.line_index as usize)
    }

    pub fn review_add_comment_button_at(
        &self,
        state: &EditorState,
        doc: &RenderDoc,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        let row_index = self.hit_test_row(state, x, y)?;
        let row = self.rows.get(row_index).copied()?;
        let line = doc.lines.get(row.line_index as usize)?;
        let rect = self.review_add_comment_button_rect_for_row(line, &row)?;
        rect.contains(x, y).then_some(row.line_index as usize)
    }

    pub fn block_action_for_row_at(
        &self,
        display_row_index: usize,
        x: f32,
        y: f32,
    ) -> Option<Action> {
        let row = self.rows.get(display_row_index)?;
        if !row.is_block() {
            return None;
        }
        let block = self.blocks.get(row.block_index as usize)?;
        block.on_click_at(
            &BlockActionCtx {
                layout: &self.layout,
                row_rect: self.row_rect_for(row),
            },
            x,
            y,
        )
    }

    pub fn block_context_menu_for_row(
        &self,
        display_row_index: usize,
    ) -> Option<Vec<ContextMenuEntry>> {
        let row = self.rows.get(display_row_index)?;
        if !row.is_block() {
            return None;
        }
        self.blocks
            .get(row.block_index as usize)?
            .context_menu_entries()
    }

    pub fn hunk_action_bar_rect(&self, doc: &RenderDoc) -> Option<(Rect, i16)> {
        let idx = self.layout.highlighted_row?;
        let display_row = self.rows.get(idx)?;
        if display_row.is_block() {
            return None;
        }
        let line = doc.lines.get(display_row.line_index as usize)?;
        if line.hunk_index < 0 {
            return None;
        }
        let hunk_index = line.hunk_index;

        let mut first_idx = idx;
        while first_idx > 0 {
            let prev = self.rows.get(first_idx - 1)?;
            if prev.is_block() {
                break;
            }
            let prev_line = doc.lines.get(prev.line_index as usize)?;
            if prev_line.hunk_index != hunk_index {
                break;
            }
            first_idx -= 1;
        }

        let mut last_idx = idx;
        while last_idx + 1 < self.rows.len() {
            let next = &self.rows[last_idx + 1];
            if next.is_block() {
                break;
            }
            let Some(next_line) = doc.lines.get(next.line_index as usize) else {
                break;
            };
            if next_line.hunk_index != hunk_index {
                break;
            }
            last_idx += 1;
        }

        let first_rect = self.row_rect_for(&self.rows[first_idx]);
        let last_rect = self.row_rect_for(&self.rows[last_idx]);
        let row_h = first_rect.height;
        let viewport_top = self.layout.content_bounds.y;
        let viewport_bottom = self.layout.content_bounds.bottom();

        let max_y = (last_rect.y + last_rect.height - row_h).max(first_rect.y);
        let y = first_rect.y.max(viewport_top).min(max_y);
        if y + row_h <= viewport_top || y >= viewport_bottom {
            return None;
        }

        // The bar floats on the hunk separator row, which spans the full
        // content width in both split and unified modes. Use the full text span
        // so the buttons right-align against the editor edge, not a column.
        let (x, width) = if self.layout.split_mode {
            let left = self.layout.left_text_rect.x;
            let right = self.layout.right_text_rect.x + self.layout.right_text_rect.width;
            (left, right - left)
        } else {
            (
                self.layout.unified_text_rect.x,
                self.layout.unified_text_rect.width,
            )
        };
        Some((
            Rect {
                x,
                y,
                width,
                height: row_h,
            },
            hunk_index,
        ))
    }

    pub fn line_selection_bar_rect(&self, doc: &RenderDoc, state: &EditorState) -> Option<Rect> {
        use super::render_doc::RenderRowKind;

        if state.line_selection.is_empty() {
            return None;
        }

        let is_selected = |row: &DisplayRow| -> bool {
            if row.is_block() {
                return false;
            }
            let Some(line) = doc.lines.get(row.line_index as usize) else {
                return false;
            };
            if !matches!(
                line.row_kind(),
                RenderRowKind::Added | RenderRowKind::Removed | RenderRowKind::Modified
            ) {
                return false;
            }
            line_selection_contains_line(&state.line_selection, line)
        };

        let first = self.rows.iter().find(|r| is_selected(r))?;
        let last = self.rows.iter().rev().find(|r| is_selected(r))?;

        let first_rect = self.row_rect_for(first);
        let last_rect = self.row_rect_for(last);
        let last_bottom = last_rect.y + last_rect.height;
        let viewport_top = self.layout.content_bounds.y;
        let viewport_bottom = self.layout.content_bounds.bottom();

        // Hide entirely when the selection is fully outside the viewport.
        if last_bottom <= viewport_top || first_rect.y >= viewport_bottom {
            return None;
        }

        let bar_h = first_rect.height;
        // Float the bar above the first selected row. Once the user scrolls
        // past that row the bar stays pinned to the viewport top, acting like
        // a sticky header over the selection — no jumps, no disappearing.
        let above_y = first_rect.y - bar_h;
        let y = above_y.max(viewport_top);
        // If even the sticky position would sit past the last selected row,
        // the selection no longer covers enough area to anchor the bar.
        if y >= last_bottom {
            return None;
        }

        // Span the full content width in both modes so the buttons right-align
        // against the editor edge, never pinned to a narrow column.
        let (x, width) = if self.layout.split_mode {
            let left = self.layout.left_text_rect.x;
            let right = self.layout.right_text_rect.x + self.layout.right_text_rect.width;
            (left, right - left)
        } else {
            (
                self.layout.unified_text_rect.x,
                self.layout.unified_text_rect.width,
            )
        };
        Some(Rect {
            x,
            y,
            width,
            height: bar_h,
        })
    }

    fn rebuild_rows(
        &mut self,
        doc: &RenderDoc,
        state: &EditorState,
        text_metrics: TextMetrics,
        show_file_headers: bool,
    ) {
        self.metrics = display_layout_metrics(text_metrics);
        self.config = DisplayLayoutConfig {
            split_mode: state.layout == LayoutMode::Split,
            wrap_enabled: state.wrap_enabled,
            wrap_column: state.wrap_column,
            show_file_headers,
            char_width_px: text_metrics.mono_char_width_px as f64,
            unified_text_width_px: self.layout.unified_text_rect.width as f64,
            split_text_width_px: self.layout.left_text_rect.width as f64,
        };
        self.summary =
            rebuild_display_rows(doc, self.config, self.metrics, &self.blocks, &mut self.rows);
        build_strip_layouts(&self.rows, STRIP_TARGET_HEIGHT_PX, &mut self.strips);
        self.file_header_hits.clear();
        if show_file_headers {
            for row in &self.rows {
                if row.kind != RenderRowKind::FileHeader as u8 {
                    continue;
                }
                let Some(line) = doc.lines.get(row.line_index as usize) else {
                    continue;
                };
                if let Some(meta) = doc.file_meta(line) {
                    self.file_header_hits.push(FileHeaderHit {
                        y_px: row.y_px,
                        h_px: row.h_px,
                        path: meta.path.clone(),
                    });
                }
            }
        }
    }

    fn rebuild_navigation_positions(&self, state: &mut EditorState) {
        state.hunk_positions.clear();
        state.file_positions.clear();
        for row in &self.rows {
            if row.kind == RenderRowKind::HunkSeparator as u8 {
                state.hunk_positions.push(row.y_px);
            } else if row.kind == RenderRowKind::FileHeader as u8 {
                state.file_positions.push(row.y_px);
            }
        }

        state.search_match_y_positions.clear();
        if state.search.open && !state.search.matches.is_empty() {
            for m in &state.search.matches {
                let y = self
                    .rows
                    .iter()
                    .find(|r| !r.is_block() && r.line_index == m.line_index)
                    .map(|r| r.y_px)
                    .unwrap_or(0);
                state.search_match_y_positions.push(y);
            }
        }
    }

    fn update_visible_ranges(&mut self, state: &mut EditorState) {
        let viewport_top_px = state.scroll_top_px;
        let viewport_height_px = state.viewport_height_px.max(1);
        let strip_range = visible_strip_range(
            &self.strips,
            viewport_top_px,
            viewport_height_px,
            STRIP_OVERSCAN,
        );
        self.layout.visible_row_range = if strip_range.is_empty() {
            VisibleRange::default()
        } else {
            let first = self.strips[strip_range.start].row_start;
            let last = self.strips[strip_range.end - 1].row_end;
            VisibleRange {
                start: first,
                end: last,
            }
        };

        let visible_bottom_px = viewport_top_px.saturating_add(viewport_height_px);
        let visible_start = self
            .rows
            .partition_point(|row| row.bottom_px() <= viewport_top_px);
        let visible_end = self
            .rows
            .partition_point(|row| row.y_px < visible_bottom_px);
        if visible_start < visible_end {
            state.visible_row_start = Some(visible_start);
            state.visible_row_end = Some(visible_end);
        } else {
            state.visible_row_start = None;
            state.visible_row_end = None;
        }
    }

    pub fn paint(
        &mut self,
        scene: &mut Scene,
        theme: &Theme,
        _state: &EditorState,
        document: EditorDocument<'_>,
    ) {
        scene.rect(RectPrimitive {
            rect: self.layout.content_bounds,
            color: theme.colors.canvas,
        });

        match document {
            EditorDocument::Empty => {}
            EditorDocument::Loading { path } => {
                self.paint_placeholder(scene, theme, path, "Loading diff...");
            }
            EditorDocument::Binary { path } => {
                self.paint_placeholder(
                    scene,
                    theme,
                    path,
                    "Binary file. Diffy only shows text diffs here.",
                );
            }
            EditorDocument::Text {
                compare_generation,
                path,
                doc,
                show_file_headers,
                ..
            } => {
                self.sync_theme_cache(theme);
                scene.clip(self.layout.content_bounds);

                self.paint_gutter_backgrounds(scene, theme);
                self.paint_row_backgrounds(scene, theme, path, doc);
                self.paint_inline_change_backgrounds(scene, theme, doc);
                self.paint_line_highlights(scene, theme);
                self.paint_line_selection(scene, theme, _state, doc);
                self.paint_review_add_affordance(scene, theme, _state, doc);
                self.paint_viewport_text_selection(scene, theme, _state, doc, compare_generation);
                self.paint_search_highlights(scene, theme, _state, doc);
                self.paint_gutter_diff_indicators(scene, theme, doc);
                self.paint_gutter_decorations(scene, theme);
                self.paint_gutter_text(scene, theme, doc);
                self.paint_body_text(scene, theme, path, doc);
                if show_file_headers {
                    self.paint_sticky_file_header(scene, theme, path, doc);
                }

                scene.pop_clip();
                self.paint_scrollbar(scene, theme);
            }
        }
    }

    pub fn append_accessibility(
        &self,
        frame: &mut AccessibilityFrame,
        state: &EditorState,
        document: EditorDocument<'_>,
        scroll_builder: ScrollActionBuilder,
    ) {
        let viewport_bounds = self.layout.content_bounds;
        if viewport_bounds.width <= 0.0 || viewport_bounds.height <= 0.0 {
            return;
        }

        frame.push(
            AccessibilityNode::new(
                format!("editor-viewport:{}", document_accessibility_key(document)),
                Role::ScrollView,
                viewport_bounds,
            )
            .label(document_accessibility_label(document))
            .description("Diff editor viewport")
            .action(AccessibilityAction::EditorViewport {
                focus: FocusTarget::Editor,
                scroll: scroll_builder,
            }),
        );

        match document {
            EditorDocument::Empty => {}
            EditorDocument::Loading { path } => {
                self.append_placeholder_accessibility(frame, path, "Loading diff...");
            }
            EditorDocument::Binary { path } => {
                self.append_placeholder_accessibility(
                    frame,
                    path,
                    "Binary file. Diffy only shows text diffs here.",
                );
            }
            EditorDocument::Text {
                path,
                doc,
                show_file_headers,
                ..
            } => {
                if show_file_headers {
                    self.append_sticky_file_header_accessibility(frame, doc);
                }
                self.append_visible_rows_accessibility(frame, state, path, doc);
            }
        }
    }

    fn append_placeholder_accessibility(
        &self,
        frame: &mut AccessibilityFrame,
        path: &str,
        message: &str,
    ) {
        let fs = self.text_metrics.ui_font_size_px;
        let s = editor_scale(self.text_metrics);
        let inset = self.layout.content_bounds.inset(scaled(24.0, s));
        let title_y = inset.y + inset.height * 0.35;
        frame.push(
            AccessibilityNode::new(
                format!("editor-placeholder-title:{path}"),
                Role::Heading,
                Rect {
                    x: inset.x,
                    y: title_y,
                    width: inset.width,
                    height: fs + 10.0,
                },
            )
            .label(path),
        );
        frame.push(
            AccessibilityNode::new(
                format!("editor-placeholder-message:{path}:{message}"),
                Role::Status,
                Rect {
                    x: inset.x,
                    y: title_y + fs + 16.0,
                    width: inset.width,
                    height: fs + 4.0,
                },
            )
            .label(message),
        );
    }

    fn append_sticky_file_header_accessibility(
        &self,
        frame: &mut AccessibilityFrame,
        doc: &RenderDoc,
    ) {
        let Some((rect, path)) = self.sticky_header_hit.as_ref() else {
            return;
        };
        let Some(bounds) = rect.intersection(self.layout.content_bounds) else {
            return;
        };
        let meta = doc.file_metadata.iter().find(|meta| meta.path == *path);
        frame.push(
            AccessibilityNode::new(
                format!("editor-sticky-file-header:{path}"),
                Role::Heading,
                bounds,
            )
            .label(file_header_accessibility_label(path, meta)),
        );
    }

    fn append_visible_rows_accessibility(
        &self,
        frame: &mut AccessibilityFrame,
        state: &EditorState,
        path: &str,
        doc: &RenderDoc,
    ) {
        for row_index in self.layout.visible_row_range.iter() {
            let Some(display_row) = self.rows.get(row_index).copied() else {
                continue;
            };
            let row_rect = self.row_rect_for(&display_row);
            let Some(bounds) = row_rect.intersection(self.layout.content_bounds) else {
                continue;
            };
            if display_row.is_block() {
                self.append_block_accessibility(frame, path, row_index, display_row, bounds);
                continue;
            }
            let Some(line) = doc.lines.get(display_row.line_index as usize).copied() else {
                continue;
            };
            match line.row_kind() {
                RenderRowKind::FileHeader => {
                    let meta = doc.file_meta(&line);
                    let label_path = meta.map(|meta| meta.path.as_str()).unwrap_or(path);
                    frame.push(
                        AccessibilityNode::new(
                            format!("editor-file-header:{path}:{}", display_row.line_index),
                            Role::Heading,
                            bounds,
                        )
                        .label(file_header_accessibility_label(label_path, meta)),
                    );
                }
                RenderRowKind::HunkSeparator => {
                    frame.push(
                        AccessibilityNode::new(
                            format!(
                                "editor-hunk:{}:{}:{}",
                                path, display_row.line_index, line.hunk_index
                            ),
                            Role::RowHeader,
                            bounds,
                        )
                        .label(hunk_accessibility_label(doc, &line)),
                    );
                }
                RenderRowKind::Context
                | RenderRowKind::Added
                | RenderRowKind::Removed
                | RenderRowKind::Modified => {
                    self.append_body_line_accessibility(
                        frame,
                        state,
                        path,
                        doc,
                        &line,
                        &display_row,
                        row_rect,
                    );
                }
                RenderRowKind::Block => {}
            }
        }
    }

    fn append_block_accessibility(
        &self,
        frame: &mut AccessibilityFrame,
        path: &str,
        row_index: usize,
        display_row: DisplayRow,
        bounds: Rect,
    ) {
        let Some(block) = self.blocks.get(display_row.block_index as usize) else {
            return;
        };
        let action = block.on_click();
        let label = block
            .accessibility_label()
            .or_else(|| action.as_ref().map(|_| "Editor action".to_owned()))
            .unwrap_or_else(|| "Editor annotation".to_owned());
        let role = if action.is_some() {
            Role::Button
        } else {
            Role::Comment
        };
        let mut node = AccessibilityNode::new(
            format!(
                "editor-block:{path}:{row_index}:{}",
                display_row.block_index
            ),
            role,
            bounds,
        )
        .label(label);
        if let Some(action) = action {
            node = node.action(AccessibilityAction::Click(action));
        }
        frame.push(node);
    }

    fn append_body_line_accessibility(
        &self,
        frame: &mut AccessibilityFrame,
        state: &EditorState,
        path: &str,
        doc: &RenderDoc,
        line: &RenderLine,
        display_row: &DisplayRow,
        row_rect: Rect,
    ) {
        let selected = !state.line_selection.is_empty()
            && line_selection_contains_line(&state.line_selection, line);
        for block in self.text_blocks_for_line(line, display_row, row_rect) {
            let Some(block) = block else {
                continue;
            };
            let text = doc.line_text(block.text_range);
            let text_bounds = Rect {
                x: block.text_x,
                y: block.y + self.layout.text_y_offset,
                width: block.text_width,
                height: block.segment_count.max(1) as f32 * self.layout.line_height,
            };
            let Some(bounds) = text_bounds.intersection(self.layout.content_bounds) else {
                continue;
            };
            frame.push(
                AccessibilityNode::new(
                    format!(
                        "editor-line:{path}:{}:{}:{}:{}",
                        block.line_index,
                        accessibility_side_key(block.side),
                        block.text_range.start,
                        block.text_range.len
                    ),
                    Role::Paragraph,
                    bounds,
                )
                .label(line_accessibility_label(
                    self.layout.split_mode,
                    line,
                    block.side,
                    text,
                ))
                .selected(selected),
            );
        }
    }

    fn paint_placeholder(&self, scene: &mut Scene, theme: &Theme, title: &str, message: &str) {
        let fs = self.text_metrics.ui_font_size_px;
        let s = editor_scale(self.text_metrics);
        let inset = self.layout.content_bounds.inset(scaled(24.0, s));
        scene.text(TextPrimitive {
            rect: Rect {
                x: inset.x,
                y: inset.y + inset.height * 0.35,
                width: inset.width,
                height: fs + 10.0,
            },
            text: title.into(),
            color: theme.colors.text_strong,
            font_size: fs + 4.0,
            font_kind: FontKind::Ui,
            font_weight: FontWeight::Normal,
        });
        scene.text(TextPrimitive {
            rect: Rect {
                x: inset.x,
                y: inset.y + inset.height * 0.35 + fs + 16.0,
                width: inset.width,
                height: fs + 4.0,
            },
            text: message.into(),
            color: theme.colors.text_muted,
            font_size: fs,
            font_kind: FontKind::Ui,
            font_weight: FontWeight::Normal,
        });
    }

    fn row_rect_for(&self, display_row: &DisplayRow) -> Rect {
        Rect {
            x: self.layout.content_bounds.x,
            y: self.layout.content_bounds.y + display_row.y_px as f32 - self.layout.scroll_top_px,
            width: self.layout.content_bounds.width,
            height: display_row.h_px as f32,
        }
    }

    fn row_in_viewport(&self, row_rect: &Rect) -> bool {
        row_rect.bottom() >= self.layout.content_bounds.y
            && row_rect.y <= self.layout.content_bounds.bottom()
    }

    fn review_add_comment_button_rect_for_row(
        &self,
        line: &RenderLine,
        display_row: &DisplayRow,
    ) -> Option<Rect> {
        let gutter = review_comment_gutter_rect(&self.layout, line)?;
        let row_rect = self.row_rect_for(display_row);
        if !self.row_in_viewport(&row_rect) {
            return None;
        }
        let size = (self.layout.line_height * 0.72).clamp(14.0, 20.0);
        Some(Rect {
            x: gutter.x + scaled(2.0, editor_scale(self.text_metrics)),
            y: row_rect.y + (self.layout.line_height - size).max(0.0) * 0.5,
            width: size,
            height: size,
        })
    }

    fn paint_sticky_file_header(
        &mut self,
        scene: &mut Scene,
        theme: &Theme,
        path: &str,
        doc: &RenderDoc,
    ) {
        self.sticky_header_hit = None;
        let header_h = self.metrics.file_header_height_px as f32;
        if header_h <= 0.0 {
            return;
        }
        let scroll_top = self.layout.scroll_top_px;
        let mut active: Option<&DisplayRow> = None;
        let mut next: Option<&DisplayRow> = None;
        for row in &self.rows {
            if row.kind != RenderRowKind::FileHeader as u8 {
                continue;
            }
            if (row.y_px as f32) <= scroll_top {
                active = Some(row);
            } else {
                next = Some(row);
                break;
            }
        }
        let Some(active_row) = active else {
            return;
        };
        let natural_screen_y = self.layout.content_bounds.y + active_row.y_px as f32 - scroll_top;
        if natural_screen_y >= self.layout.content_bounds.y {
            return;
        }
        let mut sticky_y = self.layout.content_bounds.y;
        if let Some(next_row) = next {
            let next_screen_y = self.layout.content_bounds.y + next_row.y_px as f32 - scroll_top;
            if next_screen_y < sticky_y + header_h {
                sticky_y = next_screen_y - header_h;
            }
        }
        let line_index = active_row.line_index as usize;
        let Some(line) = doc.lines.get(line_index) else {
            return;
        };
        let row_rect = Rect {
            x: self.layout.content_bounds.x,
            y: sticky_y,
            width: self.layout.content_bounds.width,
            height: header_h,
        };
        if let Some(meta) = doc.file_meta(line) {
            self.sticky_header_hit = Some((row_rect, meta.path.clone()));
        }
        let hovered = self
            .mouse_pos
            .is_some_and(|(mx, my)| row_rect.contains(mx, my));
        scene.push_z_index(STICKY_HEADER_Z);
        let mut ctx = RowPaintCtx {
            scene,
            theme,
            layout: &self.layout,
            row_rect,
            text_y_offset: self.layout.text_y_offset,
            font_size: self.layout.font_size,
            mono_char_width_px: self.text_metrics.mono_char_width_px,
            line,
            doc,
            path,
            is_header_hovered: hovered,
        };
        let deco = FileHeaderDecoration;
        deco.paint_background(&mut ctx);
        deco.paint_content(&mut ctx);
        scene.pop_z_index();
    }

    pub fn file_header_path_at(&self, x: f32, y: f32) -> Option<String> {
        if let Some((rect, path)) = self.sticky_header_hit.as_ref()
            && rect.contains(x, y)
        {
            return Some(path.clone());
        }
        if !self.layout.content_bounds.contains(x, y) {
            return None;
        }
        let content_y = (y - self.layout.content_bounds.y).max(0.0) + self.layout.scroll_top_px;
        for hit in &self.file_header_hits {
            let top = hit.y_px as f32;
            let bottom = top + hit.h_px as f32;
            if content_y >= top && content_y < bottom {
                return Some(hit.path.clone());
            }
        }
        None
    }

    pub fn hit_test_text_point(
        &self,
        state: &EditorState,
        doc: &RenderDoc,
        x: f32,
        y: f32,
    ) -> Option<ViewportTextPoint> {
        let row_index = self.hit_test_row(state, x, y)?;
        let display_row = self.rows.get(row_index).copied()?;
        if display_row.is_block() {
            return None;
        }
        let line = doc.lines.get(display_row.line_index as usize)?;
        if !line.row_kind().is_body() {
            return None;
        }
        let row_rect = self.row_rect_for(&display_row);
        let blocks = self.text_blocks_for_line(line, &display_row, row_rect);
        blocks
            .into_iter()
            .flatten()
            .filter(|block| {
                let bottom = block.y + block.segment_count.max(1) as f32 * self.layout.line_height;
                y >= block.y && y < bottom
            })
            .min_by(|a, b| {
                distance_to_text_block_x(*a, x)
                    .partial_cmp(&distance_to_text_block_x(*b, x))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|block| {
                let text = doc.line_text(block.text_range);
                let layout = CachedTextLayout::new(text);
                let segment = ((y - block.y) / self.layout.line_height.max(1.0))
                    .floor()
                    .max(0.0) as u32;
                let segment = segment.min(u32::from(block.segment_count.max(1).saturating_sub(1)));
                let local_col =
                    ((x - block.text_x) / self.text_metrics.mono_char_width_px.max(1.0)).max(0.0);
                let col = local_col + segment.saturating_mul(u32::from(block.segment_cols)) as f32;
                ViewportTextPoint {
                    line_index: block.line_index,
                    side: block.side,
                    byte_offset: layout.byte_for_col_nearest(col),
                }
            })
    }

    pub fn viewport_selection_text(
        &self,
        doc: &RenderDoc,
        selection: &ViewportTextSelection,
    ) -> Option<String> {
        if selection.is_collapsed() {
            return None;
        }
        let mut copied = String::new();
        let (start, end) = selection.normalized();
        for line_index in start.line_index..=end.line_index {
            let Some(line) = doc.lines.get(line_index as usize) else {
                continue;
            };
            for (side, range) in text_side_ranges_for_line(self.layout.split_mode, line)
                .into_iter()
                .flatten()
            {
                let text = doc.line_text(range);
                let Some((byte_start, byte_end)) = selection_byte_range_for_side(
                    selection,
                    self.layout.split_mode,
                    line_index,
                    side,
                    text,
                ) else {
                    continue;
                };
                if byte_end <= byte_start {
                    continue;
                }
                if !copied.is_empty() {
                    copied.push('\n');
                }
                copied.push_str(&text[byte_start..byte_end]);
            }
        }
        (!copied.is_empty()).then_some(copied)
    }

    pub fn viewport_line_text_at_point(
        &self,
        doc: &RenderDoc,
        point: ViewportTextPoint,
    ) -> Option<String> {
        let line = doc.lines.get(point.line_index as usize)?;
        let range = match point.side {
            ViewportTextSide::Left => line.left_text,
            ViewportTextSide::Right => line.right_text,
        };
        range.is_valid().then(|| doc.line_text(range).to_owned())
    }

    fn text_blocks_for_line(
        &self,
        line: &RenderLine,
        display_row: &DisplayRow,
        row_rect: Rect,
    ) -> [Option<TextBlock>; 2] {
        let mut blocks = [None, None];
        let mut next = 0_usize;
        let mut push_block = |block: TextBlock| {
            if next < blocks.len() {
                blocks[next] = Some(block);
                next += 1;
            }
        };

        let line_height = self.layout.line_height;
        if self.layout.split_mode {
            let segment_cols = self.render_cols_split();
            if line.left_text.is_valid() {
                push_block(TextBlock {
                    line_index: display_row.line_index,
                    side: ViewportTextSide::Left,
                    text_range: line.left_text,
                    text_x: self.layout.left_text_rect.x,
                    text_width: self.layout.left_text_rect.width,
                    y: row_rect.y,
                    segment_count: if self.config.wrap_enabled {
                        display_row.wrap_left.max(1)
                    } else {
                        1
                    },
                    segment_cols,
                });
            }
            if line.right_text.is_valid() {
                push_block(TextBlock {
                    line_index: display_row.line_index,
                    side: ViewportTextSide::Right,
                    text_range: line.right_text,
                    text_x: self.layout.right_text_rect.x,
                    text_width: self.layout.right_text_rect.width,
                    y: row_rect.y,
                    segment_count: if self.config.wrap_enabled {
                        display_row.wrap_right.max(1)
                    } else {
                        1
                    },
                    segment_cols,
                });
            }
            return blocks;
        }

        let segment_cols = self.render_cols_unified();
        if line.row_kind() == RenderRowKind::Modified
            && line.left_text.is_valid()
            && line.right_text.is_valid()
        {
            let left_segments = if self.config.wrap_enabled {
                display_row.wrap_left.max(1)
            } else {
                1
            };
            push_block(TextBlock {
                line_index: display_row.line_index,
                side: ViewportTextSide::Left,
                text_range: line.left_text,
                text_x: self.layout.unified_text_rect.x,
                text_width: self.layout.unified_text_rect.width,
                y: row_rect.y,
                segment_count: left_segments,
                segment_cols,
            });
            push_block(TextBlock {
                line_index: display_row.line_index,
                side: ViewportTextSide::Right,
                text_range: line.right_text,
                text_x: self.layout.unified_text_rect.x,
                text_width: self.layout.unified_text_rect.width,
                y: row_rect.y + left_segments as f32 * line_height,
                segment_count: if self.config.wrap_enabled {
                    display_row.wrap_right.max(1)
                } else {
                    1
                },
                segment_cols,
            });
        } else if let Some((side, text_range, _, _)) = unified_body_side_with_side(line) {
            push_block(TextBlock {
                line_index: display_row.line_index,
                side,
                text_range,
                text_x: self.layout.unified_text_rect.x,
                text_width: self.layout.unified_text_rect.width,
                y: row_rect.y,
                segment_count: if self.config.wrap_enabled {
                    display_row.wrap_left.max(1)
                } else {
                    1
                },
                segment_cols,
            });
        }
        blocks
    }

    fn paint_gutter_backgrounds(&self, scene: &mut Scene, theme: &Theme) {
        if self.layout.split_mode {
            scene.rect(RectPrimitive {
                rect: self.layout.left_gutter_rect,
                color: theme.colors.gutter_bg,
            });
            scene.rect(RectPrimitive {
                rect: self.layout.right_gutter_rect,
                color: theme.colors.gutter_bg,
            });
        } else {
            scene.rect(RectPrimitive {
                rect: self.layout.unified_gutter_rect,
                color: theme.colors.gutter_bg,
            });
        }
    }

    fn paint_row_backgrounds(&self, scene: &mut Scene, theme: &Theme, path: &str, doc: &RenderDoc) {
        let line_height = self.layout.line_height;
        let font_size = self.layout.font_size;
        let text_y_offset = self.layout.text_y_offset;
        for row_index in self.layout.visible_row_range.iter() {
            let Some(display_row) = self.rows.get(row_index).copied() else {
                continue;
            };
            if display_row.is_block() {
                continue;
            }
            let Some(line) = doc.lines.get(display_row.line_index as usize) else {
                continue;
            };
            let rr = self.row_rect_for(&display_row);
            if !self.row_in_viewport(&rr) {
                continue;
            }
            let kind = line.row_kind();
            if kind == RenderRowKind::Modified {
                self.paint_modified_row_background(scene, theme, rr, &display_row, line_height);
            } else if self.layout.split_mode
                && line.flags & RENDER_FLAG_STRUCTURAL == 0
                && matches!(kind, RenderRowKind::Added | RenderRowKind::Removed)
            {
                let mid = self.layout.right_gutter_rect.x;
                if kind == RenderRowKind::Added {
                    scene.rect(RectPrimitive {
                        rect: Rect {
                            x: mid,
                            y: rr.y,
                            width: rr.right() - mid,
                            height: rr.height,
                        },
                        color: dim_bg(theme.colors.line_add),
                    });
                } else {
                    scene.rect(RectPrimitive {
                        rect: Rect {
                            x: rr.x,
                            y: rr.y,
                            width: mid - rr.x,
                            height: rr.height,
                        },
                        color: dim_bg(theme.colors.line_del),
                    });
                }
            } else if let Some(deco) = decoration_for_kind(kind) {
                let is_header_hovered = kind == RenderRowKind::FileHeader
                    && self.mouse_pos.is_some_and(|(mx, my)| rr.contains(mx, my));
                let mut ctx = RowPaintCtx {
                    scene,
                    theme,
                    layout: &self.layout,
                    row_rect: rr,
                    text_y_offset,
                    font_size,
                    mono_char_width_px: self.text_metrics.mono_char_width_px,
                    line,
                    doc,
                    path,
                    is_header_hovered,
                };
                deco.paint_background(&mut ctx);
            } else {
                paint_row_background(scene, theme, rr, line);
            }
        }
    }

    fn paint_modified_row_background(
        &self,
        scene: &mut Scene,
        theme: &Theme,
        rr: Rect,
        display_row: &DisplayRow,
        line_height: f32,
    ) {
        let del_bg = theme.colors.line_del.with_alpha(Alpha::WHISPER);
        let add_bg = theme.colors.line_add.with_alpha(Alpha::WHISPER);
        if self.layout.split_mode {
            let mid = self.layout.right_gutter_rect.x;
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: rr.x,
                    y: rr.y,
                    width: mid - rr.x,
                    height: rr.height,
                },
                color: del_bg,
            });
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: mid,
                    y: rr.y,
                    width: rr.right() - mid,
                    height: rr.height,
                },
                color: add_bg,
            });
        } else {
            let del_lines = display_row.wrap_left.max(1) as f32;
            let del_h = del_lines * line_height;
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: rr.x,
                    y: rr.y,
                    width: rr.width,
                    height: del_h,
                },
                color: del_bg,
            });
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: rr.x,
                    y: rr.y + del_h,
                    width: rr.width,
                    height: rr.height - del_h,
                },
                color: add_bg,
            });
        }
    }

    fn paint_inline_change_backgrounds(
        &mut self,
        scene: &mut Scene,
        theme: &Theme,
        doc: &RenderDoc,
    ) {
        let char_w = self.text_metrics.mono_char_width_px;
        let line_height = self.layout.line_height;

        for row_index in self.layout.visible_row_range.iter() {
            let Some(display_row) = self.rows.get(row_index).copied() else {
                continue;
            };
            if display_row.is_block() {
                continue;
            }
            let Some(line) = doc.lines.get(display_row.line_index as usize).copied() else {
                continue;
            };
            let rr = self.row_rect_for(&display_row);
            if !self.row_in_viewport(&rr) {
                continue;
            }
            let kind = line.row_kind();
            let left_segments = if self.config.wrap_enabled {
                display_row.wrap_left.max(1)
            } else {
                1
            };
            let right_segments = if self.config.wrap_enabled {
                display_row.wrap_right.max(1)
            } else {
                1
            };

            if self.layout.split_mode {
                if matches!(kind, RenderRowKind::Removed | RenderRowKind::Modified)
                    && line.left_text.is_valid()
                {
                    self.paint_change_rects_for_side(
                        scene,
                        doc,
                        line.left_text,
                        line.left_runs,
                        self.layout.left_text_rect.x,
                        rr.y,
                        self.layout.left_text_rect.width,
                        char_w,
                        line_height,
                        left_segments,
                        self.render_cols_split(),
                        theme.colors.line_del_word_bg,
                    );
                }
                if matches!(kind, RenderRowKind::Added | RenderRowKind::Modified)
                    && line.right_text.is_valid()
                {
                    self.paint_change_rects_for_side(
                        scene,
                        doc,
                        line.right_text,
                        line.right_runs,
                        self.layout.right_text_rect.x,
                        rr.y,
                        self.layout.right_text_rect.width,
                        char_w,
                        line_height,
                        right_segments,
                        self.render_cols_split(),
                        theme.colors.line_add_word_bg,
                    );
                }
            } else {
                match kind {
                    RenderRowKind::Modified
                        if line.left_text.is_valid() && line.right_text.is_valid() =>
                    {
                        let add_y = rr.y + left_segments as f32 * line_height;
                        self.paint_change_rects_for_side(
                            scene,
                            doc,
                            line.left_text,
                            line.left_runs,
                            self.layout.unified_text_rect.x,
                            rr.y,
                            self.layout.unified_text_rect.width,
                            char_w,
                            line_height,
                            left_segments,
                            self.render_cols_unified(),
                            theme.colors.line_del_word_bg,
                        );
                        self.paint_change_rects_for_side(
                            scene,
                            doc,
                            line.right_text,
                            line.right_runs,
                            self.layout.unified_text_rect.x,
                            add_y,
                            self.layout.unified_text_rect.width,
                            char_w,
                            line_height,
                            right_segments,
                            self.render_cols_unified(),
                            theme.colors.line_add_word_bg,
                        );
                    }
                    RenderRowKind::Removed if line.left_text.is_valid() => {
                        self.paint_change_rects_for_side(
                            scene,
                            doc,
                            line.left_text,
                            line.left_runs,
                            self.layout.unified_text_rect.x,
                            rr.y,
                            self.layout.unified_text_rect.width,
                            char_w,
                            line_height,
                            left_segments,
                            self.render_cols_unified(),
                            theme.colors.line_del_word_bg,
                        );
                    }
                    RenderRowKind::Added if line.right_text.is_valid() => {
                        self.paint_change_rects_for_side(
                            scene,
                            doc,
                            line.right_text,
                            line.right_runs,
                            self.layout.unified_text_rect.x,
                            rr.y,
                            self.layout.unified_text_rect.width,
                            char_w,
                            line_height,
                            right_segments,
                            self.render_cols_unified(),
                            theme.colors.line_add_word_bg,
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    fn paint_change_rects_for_side(
        &mut self,
        scene: &mut Scene,
        doc: &RenderDoc,
        text_range: ByteRange,
        runs_range: RunRange,
        text_x: f32,
        row_y: f32,
        text_width: f32,
        char_w: f32,
        line_height: f32,
        segment_count: u16,
        segment_cols: u16,
        bg_color: Color,
    ) {
        if !text_range.is_valid() {
            return;
        }
        let text = doc.line_text(text_range);
        if text.is_empty() {
            return;
        }
        let text_layout = self.cached_text_layout(doc, text_range);
        let visible_segments = self.visible_segment_range(row_y, segment_count);
        if visible_segments.is_empty() {
            return;
        }
        let runs = doc.line_runs(runs_range);
        let mut ranges: Vec<(u32, u32)> = Vec::new();

        for run in runs {
            if run.flags & STYLE_FLAG_CHANGE == 0 || run.flags & STYLE_FLAG_UNCHANGED_CTX != 0 {
                continue;
            }
            let start = run.byte_start as usize;
            let end = start.saturating_add(run.byte_len as usize).min(text.len());
            if end <= start {
                continue;
            }
            let col_start = text_layout.col_for_byte(start);
            let col_end = text_layout.col_for_byte(end);
            if col_end <= col_start {
                continue;
            }
            if let Some((_, previous_end)) = ranges.last_mut()
                && col_start <= previous_end.saturating_add(INLINE_CHANGE_BG_MERGE_GAP_COLS)
            {
                *previous_end = (*previous_end).max(col_end);
                continue;
            }
            ranges.push((col_start, col_end));
        }

        let y_inset = (line_height * INLINE_CHANGE_BG_Y_INSET_RATIO).clamp(1.5, 2.5);
        for (col_start, col_end) in ranges {
            paint_column_range_rects_with_vertical_inset(
                scene,
                col_start,
                col_end,
                text_x,
                row_y,
                text_width,
                char_w,
                line_height,
                segment_cols,
                visible_segments.clone(),
                bg_color,
                3.0,
                y_inset,
            );
        }
    }

    fn paint_line_highlights(&self, scene: &mut Scene, theme: &Theme) {
        let Some(hovered) = self.layout.highlighted_row else {
            return;
        };
        let Some(display_row) = self.rows.get(hovered).copied() else {
            return;
        };
        // Block rows (review thread cards, expand chips) manage their own hover feedback
        // via element hit regions / their own paint. Blanket-highlighting the whole row
        // rect would tint the entire card, so skip them here.
        if display_row.is_block() {
            return;
        }
        let rr = self.row_rect_for(&display_row);
        if !self.row_in_viewport(&rr) {
            return;
        }

        scene.rect(RectPrimitive {
            rect: Rect {
                x: rr.x,
                y: rr.y,
                width: rr.width,
                height: rr.height,
            },
            color: theme.colors.hover_overlay,
        });
    }

    fn paint_line_selection(
        &self,
        scene: &mut Scene,
        theme: &Theme,
        state: &EditorState,
        doc: &RenderDoc,
    ) {
        use super::render_doc::RenderRowKind;

        if state.line_selection.is_empty() {
            return;
        }

        let gutter_rect = if self.layout.split_mode {
            self.layout.left_gutter_rect
        } else {
            self.layout.unified_gutter_rect
        };

        for row_idx in self.layout.visible_row_range.iter() {
            let Some(display_row) = self.rows.get(row_idx).copied() else {
                continue;
            };
            if display_row.is_block() {
                continue;
            }
            let Some(line) = doc.lines.get(display_row.line_index as usize) else {
                continue;
            };
            let kind = line.row_kind();
            if !matches!(
                kind,
                RenderRowKind::Added | RenderRowKind::Removed | RenderRowKind::Modified
            ) {
                continue;
            }
            let selected = line_selection_contains_line(&state.line_selection, line);
            if !selected {
                continue;
            }

            let rr = self.row_rect_for(&display_row);
            if !self.row_in_viewport(&rr) {
                continue;
            }

            let indicator_w = scaled(Sz::GUTTER_STRIPE_W, editor_scale(self.text_metrics));
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: gutter_rect.x,
                    y: rr.y,
                    width: gutter_rect.width,
                    height: rr.height,
                },
                color: Color {
                    a: Alpha::TINT,
                    ..theme.colors.accent
                },
            });
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: gutter_rect.x,
                    y: rr.y,
                    width: indicator_w,
                    height: rr.height,
                },
                color: theme.colors.accent,
            });
        }
    }

    fn paint_review_add_affordance(
        &self,
        scene: &mut Scene,
        theme: &Theme,
        state: &EditorState,
        doc: &RenderDoc,
    ) {
        if !state.review_enabled {
            return;
        }
        let Some(row_index) = self.layout.highlighted_row else {
            return;
        };
        let Some(display_row) = self.rows.get(row_index).copied() else {
            return;
        };
        if display_row.is_block() {
            return;
        }
        let Some(line) = doc.lines.get(display_row.line_index as usize) else {
            return;
        };
        let Some(rect) = self.review_add_comment_button_rect_for_row(line, &display_row) else {
            return;
        };
        let selected = !state.line_selection.is_empty()
            && line_selection_contains_line(&state.line_selection, line);
        let bg = if selected {
            theme.colors.accent_strong
        } else {
            theme.colors.accent
        };
        let radius = (rect.height * 0.3).round();
        scene.rounded_rect(RoundedRectPrimitive::uniform(rect, radius, bg));
        scene.push(Primitive::Icon(IconPrimitive {
            rect: rect.inset((rect.width * 0.22).round()),
            name: lucide::PLUS.to_owned(),
            color: Color::rgba(255, 255, 255, 255),
        }));
    }

    fn paint_viewport_text_selection(
        &mut self,
        scene: &mut Scene,
        theme: &Theme,
        state: &EditorState,
        doc: &RenderDoc,
        generation: u64,
    ) {
        let Some(selection) = state.text_selection.as_ref() else {
            return;
        };
        if selection.generation != generation || selection.is_collapsed() {
            return;
        }

        let char_w = self.text_metrics.mono_char_width_px;
        let line_height = self.layout.line_height;
        let color = theme.colors.selection_bg;

        for row_index in self.layout.visible_row_range.iter() {
            let Some(display_row) = self.rows.get(row_index).copied() else {
                continue;
            };
            if display_row.is_block() {
                continue;
            }
            let Some(line) = doc.lines.get(display_row.line_index as usize).copied() else {
                continue;
            };
            if !line.row_kind().is_body() {
                continue;
            }
            let row_rect = self.row_rect_for(&display_row);
            if !self.row_in_viewport(&row_rect) {
                continue;
            }

            for block in self.text_blocks_for_line(&line, &display_row, row_rect) {
                let Some(block) = block else {
                    continue;
                };
                let text = doc.line_text(block.text_range);
                let Some((byte_start, byte_end)) = selection_byte_range_for_side(
                    selection,
                    self.layout.split_mode,
                    block.line_index,
                    block.side,
                    text,
                ) else {
                    continue;
                };
                if byte_end <= byte_start {
                    continue;
                }

                let text_layout = self.cached_text_layout(doc, block.text_range);
                let col_start = text_layout.col_for_byte(byte_start);
                let col_end = text_layout.col_for_byte(byte_end);
                if col_end <= col_start {
                    continue;
                }
                let visible_segments = self.visible_segment_range(block.y, block.segment_count);
                if visible_segments.is_empty() {
                    continue;
                }
                paint_column_range_rects(
                    scene,
                    col_start,
                    col_end,
                    block.text_x,
                    block.y,
                    block.text_width,
                    char_w,
                    line_height,
                    block.segment_cols,
                    visible_segments,
                    color,
                    None,
                );
            }
        }
    }

    fn paint_search_highlights(
        &mut self,
        scene: &mut Scene,
        theme: &Theme,
        state: &EditorState,
        doc: &RenderDoc,
    ) {
        use crate::ui::editor::state::MatchSide;

        if !state.search.open || state.search.matches.is_empty() {
            return;
        }

        let char_w = self.text_metrics.mono_char_width_px;
        let line_height = self.layout.line_height;
        let active_idx = state.search.active_index;

        let vis = self.layout.visible_row_range;
        if vis.is_empty() {
            return;
        }
        let vis_min_line = self
            .rows
            .get(vis.start)
            .map(|r| r.line_index)
            .unwrap_or(u32::MAX);
        let vis_max_line = self
            .rows
            .get(vis.end.saturating_sub(1))
            .map(|r| r.line_index)
            .unwrap_or(0);

        for (match_idx, m) in state.search.matches.iter().enumerate() {
            let line_idx = m.line_index as usize;

            if m.line_index < vis_min_line || m.line_index > vis_max_line {
                continue;
            }

            for row_index in vis.iter() {
                let Some(display_row) = self.rows.get(row_index).copied() else {
                    continue;
                };
                if display_row.is_block() {
                    continue;
                }
                if display_row.line_index as usize != line_idx {
                    continue;
                }
                let Some(line) = doc.lines.get(line_idx) else {
                    continue;
                };
                let rr = self.row_rect_for(&display_row);
                if !self.row_in_viewport(&rr) {
                    continue;
                }

                let text_range = match m.side {
                    MatchSide::Left => line.left_text,
                    MatchSide::Right => line.right_text,
                };
                if !text_range.is_valid() {
                    continue;
                }

                let full_text = doc.line_text(text_range);
                let byte_start = m.byte_start as usize;
                let byte_end = byte_start + m.byte_len as usize;
                if byte_end > full_text.len() {
                    continue;
                }
                let text_layout = self.cached_text_layout(doc, text_range);
                let col_start = text_layout.col_for_byte(byte_start);
                let col_end = text_layout.col_for_byte(byte_end);
                if col_end <= col_start {
                    continue;
                }

                let (text_rect_x, text_rect_w, block_y, segment_count, segment_cols) =
                    if self.layout.split_mode {
                        match m.side {
                            MatchSide::Left => (
                                self.layout.left_text_rect.x,
                                self.layout.left_text_rect.width,
                                rr.y + self.layout.text_y_offset,
                                if self.config.wrap_enabled {
                                    display_row.wrap_left.max(1)
                                } else {
                                    1
                                },
                                self.render_cols_split(),
                            ),
                            MatchSide::Right => (
                                self.layout.right_text_rect.x,
                                self.layout.right_text_rect.width,
                                rr.y + self.layout.text_y_offset,
                                if self.config.wrap_enabled {
                                    display_row.wrap_right.max(1)
                                } else {
                                    1
                                },
                                self.render_cols_split(),
                            ),
                        }
                    } else if line.row_kind() == RenderRowKind::Modified
                        && line.left_text.is_valid()
                        && line.right_text.is_valid()
                        && m.side == MatchSide::Right
                    {
                        (
                            self.layout.unified_text_rect.x,
                            self.layout.unified_text_rect.width,
                            rr.y + display_row.wrap_left.max(1) as f32 * line_height
                                + self.layout.text_y_offset,
                            if self.config.wrap_enabled {
                                display_row.wrap_right.max(1)
                            } else {
                                1
                            },
                            self.render_cols_unified(),
                        )
                    } else {
                        (
                            self.layout.unified_text_rect.x,
                            self.layout.unified_text_rect.width,
                            rr.y + self.layout.text_y_offset,
                            if self.config.wrap_enabled {
                                display_row.wrap_left.max(1)
                            } else {
                                1
                            },
                            self.render_cols_unified(),
                        )
                    };

                let is_active = active_idx == Some(match_idx);
                let color = if is_active {
                    theme.colors.search_match_active_bg
                } else {
                    theme.colors.search_match_bg
                };
                let visible_segments = self.visible_segment_range(block_y, segment_count);
                if visible_segments.is_empty() {
                    continue;
                }
                paint_column_range_rects(
                    scene,
                    col_start,
                    col_end,
                    text_rect_x,
                    block_y,
                    text_rect_w,
                    char_w,
                    line_height,
                    segment_cols,
                    visible_segments,
                    color,
                    None,
                );
            }
        }
    }

    fn paint_gutter_diff_indicators(&self, scene: &mut Scene, theme: &Theme, doc: &RenderDoc) {
        let line_height = self.layout.line_height;
        let s = editor_scale(self.text_metrics);
        let strip_w = scaled(3.0, s);

        for row_index in self.layout.visible_row_range.iter() {
            let Some(display_row) = self.rows.get(row_index).copied() else {
                continue;
            };
            if display_row.is_block() {
                continue;
            }
            let Some(line) = doc.lines.get(display_row.line_index as usize).copied() else {
                continue;
            };
            let rr = self.row_rect_for(&display_row);
            if !self.row_in_viewport(&rr) {
                continue;
            }
            let kind = line.row_kind();
            if !matches!(
                kind,
                RenderRowKind::Added | RenderRowKind::Removed | RenderRowKind::Modified
            ) {
                continue;
            }

            if self.layout.split_mode {
                if matches!(kind, RenderRowKind::Removed | RenderRowKind::Modified) {
                    scene.rect(RectPrimitive {
                        rect: Rect {
                            x: self.layout.left_gutter_rect.right() - strip_w,
                            y: rr.y,
                            width: strip_w,
                            height: rr.height,
                        },
                        color: theme.colors.line_del_text,
                    });
                }
                if matches!(kind, RenderRowKind::Added | RenderRowKind::Modified) {
                    scene.rect(RectPrimitive {
                        rect: Rect {
                            x: self.layout.right_gutter_rect.right() - strip_w,
                            y: rr.y,
                            width: strip_w,
                            height: rr.height,
                        },
                        color: theme.colors.line_add_text,
                    });
                }
            } else {
                if kind == RenderRowKind::Modified
                    && line.left_text.is_valid()
                    && line.right_text.is_valid()
                {
                    let del_h = display_row.wrap_left.max(1) as f32 * line_height;
                    scene.rect(RectPrimitive {
                        rect: Rect {
                            x: self.layout.unified_gutter_rect.right() - strip_w,
                            y: rr.y,
                            width: strip_w,
                            height: del_h,
                        },
                        color: theme.colors.line_del_text,
                    });
                    scene.rect(RectPrimitive {
                        rect: Rect {
                            x: self.layout.unified_gutter_rect.right() - strip_w,
                            y: rr.y + del_h,
                            width: strip_w,
                            height: rr.height - del_h,
                        },
                        color: theme.colors.line_add_text,
                    });
                } else {
                    let c = match kind {
                        RenderRowKind::Added => theme.colors.line_add_text,
                        RenderRowKind::Removed => theme.colors.line_del_text,
                        _ => continue,
                    };
                    scene.rect(RectPrimitive {
                        rect: Rect {
                            x: self.layout.unified_gutter_rect.right() - strip_w,
                            y: rr.y,
                            width: strip_w,
                            height: rr.height,
                        },
                        color: c,
                    });
                }
            }
        }
    }

    fn paint_gutter_decorations(&self, scene: &mut Scene, theme: &Theme) {
        let cb = self.layout.content_bounds;
        if self.layout.split_mode {
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: self.layout.left_gutter_rect.right() - 1.0,
                    y: cb.y,
                    width: 1.0,
                    height: cb.height,
                },
                color: theme.colors.border_soft,
            });
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: self.layout.right_gutter_rect.right() - 1.0,
                    y: cb.y,
                    width: 1.0,
                    height: cb.height,
                },
                color: theme.colors.border_soft,
            });
        } else {
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: self.layout.unified_gutter_rect.right() - 1.0,
                    y: cb.y,
                    width: 1.0,
                    height: cb.height,
                },
                color: theme.colors.border_soft,
            });
        }
    }

    fn paint_gutter_text(&mut self, scene: &mut Scene, theme: &Theme, doc: &RenderDoc) {
        let font_size = self.layout.font_size;
        let line_height = self.layout.line_height;
        let ty = self.layout.text_y_offset;

        for row_index in self.layout.visible_row_range.iter() {
            let Some(display_row) = self.rows.get(row_index).copied() else {
                continue;
            };
            if display_row.is_block() {
                continue;
            }
            let Some(line) = doc.lines.get(display_row.line_index as usize).copied() else {
                continue;
            };
            let rr = self.row_rect_for(&display_row);
            if !self.row_in_viewport(&rr) {
                continue;
            }

            match line.row_kind() {
                RenderRowKind::FileHeader | RenderRowKind::HunkSeparator => {}
                _ if self.layout.split_mode => {
                    scene.text(TextPrimitive {
                        rect: Rect {
                            x: self.layout.left_gutter_rect.x + self.layout.gutter_padding,
                            y: rr.y + ty,
                            width: self.layout.left_gutter_rect.width
                                - self.layout.gutter_padding * 2.0,
                            height: line_height,
                        },
                        text: self.cached_gutter_text(GutterTextCacheKey {
                            old_line_no: line.old_line_no,
                            new_line_no: INVALID_U32,
                            digits: self.summary.gutter_digits,
                            kind: GutterTextKind::SplitLeft,
                        }),
                        color: theme.colors.gutter_text,
                        font_size,
                        font_kind: FontKind::Mono,
                        font_weight: FontWeight::Normal,
                    });
                    scene.text(TextPrimitive {
                        rect: Rect {
                            x: self.layout.right_gutter_rect.x + self.layout.gutter_padding,
                            y: rr.y + ty,
                            width: self.layout.right_gutter_rect.width
                                - self.layout.gutter_padding * 2.0,
                            height: line_height,
                        },
                        text: self.cached_gutter_text(GutterTextCacheKey {
                            old_line_no: INVALID_U32,
                            new_line_no: line.new_line_no,
                            digits: self.summary.gutter_digits,
                            kind: GutterTextKind::SplitRight,
                        }),
                        color: theme.colors.gutter_text,
                        font_size,
                        font_kind: FontKind::Mono,
                        font_weight: FontWeight::Normal,
                    });
                }
                RenderRowKind::Modified
                    if line.left_text.is_valid() && line.right_text.is_valid() =>
                {
                    scene.text(TextPrimitive {
                        rect: Rect {
                            x: self.layout.unified_gutter_rect.x + self.layout.gutter_padding,
                            y: rr.y + ty,
                            width: self.layout.unified_gutter_rect.width
                                - self.layout.gutter_padding * 2.0,
                            height: line_height,
                        },
                        text: self.cached_gutter_text(GutterTextCacheKey {
                            old_line_no: line.old_line_no,
                            new_line_no: INVALID_U32,
                            digits: self.summary.gutter_digits,
                            kind: GutterTextKind::UnifiedOldOnly,
                        }),
                        color: theme.colors.gutter_text,
                        font_size,
                        font_kind: FontKind::Mono,
                        font_weight: FontWeight::Normal,
                    });
                    let added_y = rr.y + display_row.wrap_left.max(1) as f32 * line_height;
                    scene.text(TextPrimitive {
                        rect: Rect {
                            x: self.layout.unified_gutter_rect.x + self.layout.gutter_padding,
                            y: added_y + ty,
                            width: self.layout.unified_gutter_rect.width
                                - self.layout.gutter_padding * 2.0,
                            height: line_height,
                        },
                        text: self.cached_gutter_text(GutterTextCacheKey {
                            old_line_no: INVALID_U32,
                            new_line_no: line.new_line_no,
                            digits: self.summary.gutter_digits,
                            kind: GutterTextKind::UnifiedNewOnly,
                        }),
                        color: theme.colors.gutter_text,
                        font_size,
                        font_kind: FontKind::Mono,
                        font_weight: FontWeight::Normal,
                    });
                }
                _ => {
                    scene.text(TextPrimitive {
                        rect: Rect {
                            x: self.layout.unified_gutter_rect.x + self.layout.gutter_padding,
                            y: rr.y + ty,
                            width: self.layout.unified_gutter_rect.width
                                - self.layout.gutter_padding * 2.0,
                            height: line_height,
                        },
                        text: self.cached_gutter_text(GutterTextCacheKey {
                            old_line_no: line.old_line_no,
                            new_line_no: line.new_line_no,
                            digits: self.summary.gutter_digits,
                            kind: GutterTextKind::Unified,
                        }),
                        color: theme.colors.gutter_text,
                        font_size,
                        font_kind: FontKind::Mono,
                        font_weight: FontWeight::Normal,
                    });
                }
            }
        }
    }

    fn paint_body_text(&mut self, scene: &mut Scene, theme: &Theme, path: &str, doc: &RenderDoc) {
        let font_size = self.layout.font_size;
        let line_height = self.layout.line_height;
        let ty = self.layout.text_y_offset;

        for row_index in self.layout.visible_row_range.iter() {
            let Some(display_row) = self.rows.get(row_index).copied() else {
                continue;
            };
            let rr = self.row_rect_for(&display_row);
            if !self.row_in_viewport(&rr) {
                continue;
            }
            if display_row.is_block() {
                if let Some(block) = self.blocks.get(display_row.block_index as usize) {
                    let hovered = self.layout.highlighted_row == Some(row_index);
                    let mut ctx = BlockPaintCtx {
                        scene,
                        theme,
                        layout: &self.layout,
                        row_rect: rr,
                        text_y_offset: ty,
                        font_size,
                        hovered,
                    };
                    block.paint(&mut ctx);
                }
                continue;
            }
            let Some(line) = doc.lines.get(display_row.line_index as usize).copied() else {
                continue;
            };

            let kind = line.row_kind();
            if let Some(deco) = decoration_for_kind(kind) {
                let is_header_hovered = kind == RenderRowKind::FileHeader
                    && self.mouse_pos.is_some_and(|(mx, my)| rr.contains(mx, my));
                let mut ctx = RowPaintCtx {
                    scene,
                    theme,
                    layout: &self.layout,
                    row_rect: rr,
                    text_y_offset: ty,
                    font_size,
                    mono_char_width_px: self.text_metrics.mono_char_width_px,
                    line: &line,
                    doc,
                    path,
                    is_header_hovered,
                };
                deco.paint_content(&mut ctx);
                continue;
            }

            match kind {
                _ if self.layout.split_mode => {
                    self.paint_split_body_spans(
                        scene,
                        theme,
                        rr,
                        &line,
                        &display_row,
                        doc,
                        font_size,
                        line_height,
                    );
                }
                RenderRowKind::Modified
                    if line.left_text.is_valid() && line.right_text.is_valid() =>
                {
                    self.paint_unified_modified_spans(
                        scene,
                        theme,
                        rr,
                        &line,
                        &display_row,
                        doc,
                        font_size,
                        line_height,
                    );
                }
                _ => {
                    if let Some((text_range, runs, tone)) = unified_body_side(&line) {
                        let segment_count = if self.config.wrap_enabled {
                            display_row.wrap_left.max(1)
                        } else {
                            1
                        };
                        let render_cols = self.render_cols_unified();
                        let block_y = rr.y + ty;
                        for seg in self.visible_segment_range(block_y, segment_count) {
                            if let Some(spans) = self.cached_wrapped_rich_text(
                                doc,
                                text_range,
                                runs,
                                seg,
                                render_cols,
                                tone,
                                theme,
                            ) {
                                scene.rich_text(RichTextPrimitive {
                                    rect: Rect {
                                        x: self.layout.unified_text_rect.x,
                                        y: block_y + seg as f32 * line_height,
                                        width: self.layout.unified_text_rect.width,
                                        height: line_height,
                                    },
                                    spans,
                                    default_color: tone.default_text(theme),
                                    font_size,
                                    font_kind: FontKind::Mono,
                                    font_weight: FontWeight::Normal,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn paint_scrollbar(&self, scene: &mut Scene, theme: &Theme) {
        let Some(sb) = self.layout.scrollbar else {
            return;
        };
        scene.rounded_rect(RoundedRectPrimitive::uniform(
            sb.track,
            4.0,
            Color::rgba(128, 128, 128, 10),
        ));
        scene.rounded_rect(RoundedRectPrimitive::uniform(
            sb.thumb,
            3.0,
            theme.colors.scrollbar_thumb,
        ));
    }

    fn paint_split_body_spans(
        &mut self,
        scene: &mut Scene,
        theme: &Theme,
        rr: Rect,
        line: &RenderLine,
        display_row: &DisplayRow,
        doc: &RenderDoc,
        font_size: f32,
        line_height: f32,
    ) {
        let ty = self.layout.text_y_offset;
        let render_cols = self.render_cols_split();
        let left_segment_count = if self.config.wrap_enabled {
            display_row.wrap_left.max(1)
        } else {
            1
        };
        for seg in self.visible_segment_range(rr.y + ty, left_segment_count) {
            let rect = Rect {
                x: self.layout.left_text_rect.x,
                y: rr.y + seg as f32 * line_height + ty,
                width: self.layout.left_text_rect.width,
                height: line_height,
            };
            if let Some(spans) = self.cached_wrapped_rich_text(
                doc,
                line.left_text,
                line.left_runs,
                seg,
                render_cols,
                tone_for_left_side(line),
                theme,
            ) {
                scene.rich_text(RichTextPrimitive {
                    rect,
                    spans,
                    default_color: tone_for_left_side(line).default_text(theme),
                    font_size,
                    font_kind: FontKind::Mono,
                    font_weight: FontWeight::Normal,
                });
            }
        }
        let right_segment_count = if self.config.wrap_enabled {
            display_row.wrap_right.max(1)
        } else {
            1
        };
        for seg in self.visible_segment_range(rr.y + ty, right_segment_count) {
            let rect = Rect {
                x: self.layout.right_text_rect.x,
                y: rr.y + seg as f32 * line_height + ty,
                width: self.layout.right_text_rect.width,
                height: line_height,
            };
            if let Some(spans) = self.cached_wrapped_rich_text(
                doc,
                line.right_text,
                line.right_runs,
                seg,
                render_cols,
                tone_for_right_side(line),
                theme,
            ) {
                scene.rich_text(RichTextPrimitive {
                    rect,
                    spans,
                    default_color: tone_for_right_side(line).default_text(theme),
                    font_size,
                    font_kind: FontKind::Mono,
                    font_weight: FontWeight::Normal,
                });
            }
        }
    }

    fn paint_unified_modified_spans(
        &mut self,
        scene: &mut Scene,
        theme: &Theme,
        rr: Rect,
        line: &RenderLine,
        display_row: &DisplayRow,
        doc: &RenderDoc,
        font_size: f32,
        line_height: f32,
    ) {
        let ty = self.layout.text_y_offset;
        let render_cols = self.render_cols_unified();
        let left_segment_count = if self.config.wrap_enabled {
            display_row.wrap_left.max(1)
        } else {
            1
        };
        for seg in self.visible_segment_range(rr.y + ty, left_segment_count) {
            let y = rr.y + seg as f32 * line_height + ty;
            let rect = Rect {
                x: self.layout.unified_text_rect.x,
                y,
                width: self.layout.unified_text_rect.width,
                height: line_height,
            };
            if let Some(spans) = self.cached_wrapped_rich_text(
                doc,
                line.left_text,
                line.left_runs,
                seg,
                render_cols,
                tone_for_left_side(line),
                theme,
            ) {
                scene.rich_text(RichTextPrimitive {
                    rect,
                    spans,
                    default_color: tone_for_left_side(line).default_text(theme),
                    font_size,
                    font_kind: FontKind::Mono,
                    font_weight: FontWeight::Normal,
                });
            }
        }
        let right_block_y = rr.y + display_row.wrap_left.max(1) as f32 * line_height + ty;
        let right_segment_count = if self.config.wrap_enabled {
            display_row.wrap_right.max(1)
        } else {
            1
        };
        for seg in self.visible_segment_range(right_block_y, right_segment_count) {
            let y = rr.y
                + display_row.wrap_left.max(1) as f32 * line_height
                + seg as f32 * line_height
                + ty;
            let rect = Rect {
                x: self.layout.unified_text_rect.x,
                y,
                width: self.layout.unified_text_rect.width,
                height: line_height,
            };
            if let Some(spans) = self.cached_wrapped_rich_text(
                doc,
                line.right_text,
                line.right_runs,
                seg,
                render_cols,
                tone_for_right_side(line),
                theme,
            ) {
                scene.rich_text(RichTextPrimitive {
                    rect,
                    spans,
                    default_color: tone_for_right_side(line).default_text(theme),
                    font_size,
                    font_kind: FontKind::Mono,
                    font_weight: FontWeight::Normal,
                });
            }
        }
    }

    fn clear_document_caches(&mut self) {
        self.wrapped_text_cache.clear();
        self.text_layout_cache.clear();
        self.gutter_text_cache.clear();
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

    fn render_cols_unified(&self) -> u16 {
        render_cols_for_width(
            self.config.wrap_enabled,
            self.config.wrap_column,
            self.config.char_width_px as f32,
            self.layout.unified_text_rect.width,
        )
    }

    fn render_cols_split(&self) -> u16 {
        render_cols_for_width(
            self.config.wrap_enabled,
            self.config.wrap_column,
            self.config.char_width_px as f32,
            self.layout.left_text_rect.width,
        )
    }

    fn visible_segment_range(&self, block_y: f32, segment_count: u16) -> Range<u16> {
        visible_segment_range_for_block(
            block_y,
            segment_count.max(1),
            self.layout.line_height,
            self.layout.content_bounds.y,
            self.layout.content_bounds.bottom(),
        )
    }
}

fn line_selection_contains_line(
    selection: &super::state::LineSelection,
    line: &RenderLine,
) -> bool {
    let Ok(hunk_id) = u32::try_from(line.hunk_index) else {
        return false;
    };
    (line.old_line_index >= 0
        && selection.contains(hunk_id, carbon::DiffSide::Old, line.old_line_index as u32))
        || (line.new_line_index >= 0
            && selection.contains(hunk_id, carbon::DiffSide::New, line.new_line_index as u32))
}

fn review_comment_gutter_rect(layout: &EditorLayout, line: &RenderLine) -> Option<Rect> {
    if line.hunk_index < 0 {
        return None;
    }
    match line.row_kind() {
        RenderRowKind::Added | RenderRowKind::Modified if layout.split_mode => {
            Some(layout.right_gutter_rect)
        }
        RenderRowKind::Removed if layout.split_mode => Some(layout.left_gutter_rect),
        RenderRowKind::Added | RenderRowKind::Removed | RenderRowKind::Modified => {
            Some(layout.unified_gutter_rect)
        }
        _ => None,
    }
}

fn distance_to_text_block_x(block: TextBlock, x: f32) -> f32 {
    if x < block.text_x {
        block.text_x - x
    } else if x > block.text_x + block.text_width {
        x - (block.text_x + block.text_width)
    } else {
        0.0
    }
}

fn text_side_ranges_for_line(
    split_mode: bool,
    line: &RenderLine,
) -> [Option<(ViewportTextSide, ByteRange)>; 2] {
    let mut ranges = [None, None];
    if !line.row_kind().is_body() {
        return ranges;
    }
    let mut next = 0_usize;
    let mut push = |side, range: ByteRange| {
        if range.is_valid() && next < ranges.len() {
            ranges[next] = Some((side, range));
            next += 1;
        }
    };

    if split_mode {
        push(ViewportTextSide::Left, line.left_text);
        push(ViewportTextSide::Right, line.right_text);
    } else if line.row_kind() == RenderRowKind::Modified
        && line.left_text.is_valid()
        && line.right_text.is_valid()
    {
        push(ViewportTextSide::Left, line.left_text);
        push(ViewportTextSide::Right, line.right_text);
    } else if let Some((side, range, _, _)) = unified_body_side_with_side(line) {
        push(side, range);
    }
    ranges
}

fn selection_byte_range_for_side(
    selection: &ViewportTextSelection,
    split_mode: bool,
    line_index: u32,
    side: ViewportTextSide,
    text: &str,
) -> Option<(usize, usize)> {
    let (start, end) = selection_bounds_for_side(selection, split_mode, side)?;
    let text_len = text.len();
    let text_len_u32 = text_len.min(u32::MAX as usize) as u32;
    let side_start = ViewportTextPoint {
        line_index,
        side,
        byte_offset: 0,
    };
    let side_end = ViewportTextPoint {
        line_index,
        side,
        byte_offset: text_len_u32,
    };
    if side_end <= start || side_start >= end {
        return None;
    }

    let byte_start = if start.line_index == line_index && start.side == side {
        start.byte_offset.min(text_len_u32)
    } else {
        0
    };
    let byte_end = if end.line_index == line_index && end.side == side {
        end.byte_offset.min(text_len_u32)
    } else {
        text_len_u32
    };
    let byte_start = previous_char_boundary(text, byte_start as usize);
    let byte_end = previous_char_boundary(text, byte_end as usize);
    (byte_end > byte_start).then_some((byte_start, byte_end))
}

fn selection_bounds_for_side(
    selection: &ViewportTextSelection,
    split_mode: bool,
    side: ViewportTextSide,
) -> Option<(ViewportTextPoint, ViewportTextPoint)> {
    if !split_mode {
        return Some(selection.normalized());
    }
    if selection.anchor.side != side {
        return None;
    }
    let anchor = selection.anchor;
    let focus = ViewportTextPoint {
        side,
        ..selection.focus
    };
    Some(if anchor <= focus {
        (anchor, focus)
    } else {
        (focus, anchor)
    })
}

fn previous_char_boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RowTone {
    Neutral,
    Added,
    Removed,
    ModifiedOld,
    ModifiedNew,
}

impl RowTone {
    fn default_text(self, theme: &Theme) -> Color {
        match self {
            Self::Neutral => theme.colors.text_strong,
            Self::Added => theme.colors.line_add_text,
            Self::Removed => theme.colors.line_del_text,
            Self::ModifiedOld => theme.colors.text_strong,
            Self::ModifiedNew => theme.colors.text_strong,
        }
    }
}

fn compute_scrollbar_layout(
    layout: &EditorLayout,
    state: &EditorState,
    override_metrics: Option<ScrollbarOverride>,
) -> Option<ScrollbarLayout> {
    if state.viewport_height_px == 0 {
        return None;
    }
    let (content_height_px, scroll_top_px, max_scroll_top_px) = match override_metrics {
        Some(o) => (o.total_height_px, o.scroll_top_px, o.max_scroll_top_px),
        None => (
            state.content_height_px,
            state.scroll_top_px,
            state.max_scroll_top_px(),
        ),
    };
    if content_height_px <= state.viewport_height_px {
        return None;
    }
    let s = layout.font_size / BASE_MONO_FONT_SIZE;
    let sb_width = scaled(BASE_SCROLLBAR_WIDTH, s);
    let sb_margin = scaled(BASE_SCROLLBAR_MARGIN, s);
    let cb = layout.content_bounds;
    let track = Rect {
        x: cb.right() - sb_width,
        y: cb.y + sb_margin,
        width: sb_width,
        height: (cb.height - sb_margin * 2.0).max(0.0),
    };
    let ratio = state.viewport_height_px as f32 / content_height_px as f32;
    let thumb_min = scaled(BASE_SCROLLBAR_THUMB_MIN, s);
    let thumb_height = (track.height * ratio).max(thumb_min).min(track.height);
    let scroll_range = max_scroll_top_px.max(1) as f32;
    let top_ratio = (scroll_top_px as f32 / scroll_range).clamp(0.0, 1.0);
    let thumb_y = track.y + (track.height - thumb_height) * top_ratio;
    Some(ScrollbarLayout {
        track,
        thumb_top: thumb_y,
        thumb_height,
        thumb: Rect {
            x: track.x + 1.0,
            y: thumb_y + 1.0,
            width: track.width - 2.0,
            height: thumb_height - 2.0,
        },
    })
}

fn build_spatial_layout(
    bounds: Rect,
    layout: LayoutMode,
    gutter_digits: u32,
    text_metrics: TextMetrics,
) -> EditorLayout {
    let s = editor_scale(text_metrics);
    let viewport_padding = scaled(BASE_VIEWPORT_PADDING, s);
    let column_gap = scaled(BASE_COLUMN_GAP, s);
    let gutter_padding = scaled(BASE_GUTTER_PADDING, s);
    let scrollbar_width = scaled(BASE_SCROLLBAR_WIDTH, s);
    let scrollbar_margin = scaled(BASE_SCROLLBAR_MARGIN, s);

    let content_bounds = bounds.inset(viewport_padding);
    let usable_width = (content_bounds.width - scrollbar_width - scrollbar_margin).max(0.0);
    let gutter_width =
        gutter_digits as f32 * text_metrics.mono_char_width_px + gutter_padding * 2.0;
    let unified_gutter_width = gutter_digits as f32 * text_metrics.mono_char_width_px * 2.0
        + text_metrics.mono_char_width_px
        + gutter_padding * 2.0;

    if layout == LayoutMode::Split {
        let col_width = ((usable_width - gutter_width * 2.0 - column_gap) / 2.0).max(60.0);
        let left_gutter_rect = Rect {
            x: content_bounds.x,
            y: content_bounds.y,
            width: gutter_width,
            height: content_bounds.height,
        };
        let text_left_pad = gutter_padding;
        let left_text_rect = Rect {
            x: left_gutter_rect.right() + text_left_pad,
            y: content_bounds.y,
            width: (col_width - text_left_pad).max(60.0),
            height: content_bounds.height,
        };
        let right_gutter_rect = Rect {
            x: left_gutter_rect.right() + col_width + column_gap,
            y: content_bounds.y,
            width: gutter_width,
            height: content_bounds.height,
        };
        let right_text_rect = Rect {
            x: right_gutter_rect.right() + text_left_pad,
            y: content_bounds.y,
            width: (content_bounds.right()
                - scrollbar_width
                - scrollbar_margin
                - right_gutter_rect.right()
                - text_left_pad)
                .max(60.0),
            height: content_bounds.height,
        };
        EditorLayout {
            outer_bounds: bounds,
            content_bounds,
            split_mode: true,
            gutter_digits,
            unified_gutter_rect: Rect::default(),
            unified_text_rect: Rect::default(),
            left_gutter_rect,
            left_text_rect,
            right_gutter_rect,
            right_text_rect,
            ..EditorLayout::default()
        }
    } else {
        let unified_gutter_rect = Rect {
            x: content_bounds.x,
            y: content_bounds.y,
            width: unified_gutter_width,
            height: content_bounds.height,
        };
        let text_left_pad = gutter_padding;
        let unified_text_rect = Rect {
            x: unified_gutter_rect.right() + text_left_pad,
            y: content_bounds.y,
            width: (usable_width - unified_gutter_width - text_left_pad).max(60.0),
            height: content_bounds.height,
        };
        EditorLayout {
            outer_bounds: bounds,
            content_bounds,
            split_mode: false,
            gutter_digits,
            unified_gutter_rect,
            unified_text_rect,
            ..EditorLayout::default()
        }
    }
}

fn dim_bg(c: Color) -> Color {
    Color {
        a: ((c.a as u16 * 200) / 255) as u8,
        ..c
    }
}

fn paint_row_background(scene: &mut Scene, theme: &Theme, row_rect: Rect, line: &RenderLine) {
    let color = if line.flags & RENDER_FLAG_STRUCTURAL != 0 {
        theme.colors.canvas
    } else {
        match line.row_kind() {
            RenderRowKind::Context => theme.colors.canvas,
            RenderRowKind::Added => dim_bg(theme.colors.line_add),
            RenderRowKind::Removed => dim_bg(theme.colors.line_del),
            RenderRowKind::Modified => theme.colors.line_modified.with_alpha(Alpha::WHISPER),
            RenderRowKind::FileHeader | RenderRowKind::HunkSeparator | RenderRowKind::Block => {
                return;
            }
        }
    };
    scene.rect(RectPrimitive {
        rect: row_rect,
        color,
    });
}

fn format_line_number_string(line_no: u32, digits: u32) -> String {
    if line_no == INVALID_U32 {
        " ".repeat(digits as usize)
    } else {
        format!("{line_no:>width$}", width = digits as usize)
    }
}

fn unified_body_side_with_side(
    line: &RenderLine,
) -> Option<(ViewportTextSide, ByteRange, RunRange, RowTone)> {
    let structural = line.flags & RENDER_FLAG_STRUCTURAL != 0;
    match line.row_kind() {
        RenderRowKind::Context => Some((
            ViewportTextSide::Right,
            line.right_text,
            line.right_runs,
            RowTone::Neutral,
        )),
        RenderRowKind::Added if structural => Some((
            ViewportTextSide::Right,
            line.right_text,
            line.right_runs,
            RowTone::Neutral,
        )),
        RenderRowKind::Added => Some((
            ViewportTextSide::Right,
            line.right_text,
            line.right_runs,
            RowTone::Added,
        )),
        RenderRowKind::Removed if structural => Some((
            ViewportTextSide::Left,
            line.left_text,
            line.left_runs,
            RowTone::Neutral,
        )),
        RenderRowKind::Removed => Some((
            ViewportTextSide::Left,
            line.left_text,
            line.left_runs,
            RowTone::Removed,
        )),
        _ => None,
    }
}

fn unified_body_side(line: &RenderLine) -> Option<(ByteRange, RunRange, RowTone)> {
    unified_body_side_with_side(line).map(|(_, text, runs, tone)| (text, runs, tone))
}

fn tone_for_left_side(line: &RenderLine) -> RowTone {
    if line.flags & RENDER_FLAG_STRUCTURAL != 0 {
        return RowTone::Neutral;
    }
    match line.row_kind() {
        RenderRowKind::Modified => RowTone::ModifiedOld,
        RenderRowKind::Removed => RowTone::Removed,
        _ => RowTone::Neutral,
    }
}

fn tone_for_right_side(line: &RenderLine) -> RowTone {
    if line.flags & RENDER_FLAG_STRUCTURAL != 0 {
        return RowTone::Neutral;
    }
    match line.row_kind() {
        RenderRowKind::Modified => RowTone::ModifiedNew,
        RenderRowKind::Added => RowTone::Added,
        _ => RowTone::Neutral,
    }
}

fn document_accessibility_key(document: EditorDocument<'_>) -> String {
    match document {
        EditorDocument::Empty => "empty".to_owned(),
        EditorDocument::Loading { path } => format!("loading:{path}"),
        EditorDocument::Binary { path } => format!("binary:{path}"),
        EditorDocument::Text {
            compare_generation,
            file_index,
            path,
            ..
        } => format!(
            "text:{compare_generation}:{file_index}:{}",
            if path.is_empty() { "continuous" } else { path }
        ),
    }
}

fn document_accessibility_label(document: EditorDocument<'_>) -> String {
    match document {
        EditorDocument::Empty => "Diff editor".to_owned(),
        EditorDocument::Loading { path } => format!("Diff editor, loading {path}"),
        EditorDocument::Binary { path } => format!("Diff editor, binary file {path}"),
        EditorDocument::Text { path, doc, .. } if path.is_empty() => {
            format!(
                "Diff editor, visible compare document, {} rows",
                doc.line_count()
            )
        }
        EditorDocument::Text { path, doc, .. } => {
            format!("Diff editor, {path}, {} rows", doc.line_count())
        }
    }
}

fn file_header_accessibility_label(path: &str, meta: Option<&FileHeaderMeta>) -> String {
    let mut label = format!("File {path}");
    if let Some(meta) = meta {
        if let Some(old_path) = meta.old_path.as_deref()
            && old_path != path
        {
            label.push_str(", renamed from ");
            label.push_str(old_path);
        }
        if meta.is_binary {
            label.push_str(", binary");
        }
        label.push_str(&format!(
            ", {} additions, {} deletions",
            meta.additions, meta.deletions
        ));
    }
    label
}

fn hunk_accessibility_label(doc: &RenderDoc, line: &RenderLine) -> String {
    let mut label = if line.hunk_index >= 0 {
        format!("Hunk {}", i32::from(line.hunk_index) + 1)
    } else {
        "Hunk".to_owned()
    };
    let right_text = doc.line_text(line.right_text).trim();
    let text = if right_text.is_empty() {
        doc.line_text(line.left_text).trim()
    } else {
        right_text
    };
    if !text.is_empty() {
        label.push_str(": ");
        label.push_str(text);
    }
    label
}

fn line_accessibility_label(
    split_mode: bool,
    line: &RenderLine,
    side: ViewportTextSide,
    text: &str,
) -> String {
    let kind = line_accessibility_kind(line, side);
    let location = line_accessibility_location(split_mode, line, side);
    let text = if text.is_empty() { "empty line" } else { text };
    if location.is_empty() {
        format!("{kind}: {text}")
    } else {
        format!("{kind}, {location}: {text}")
    }
}

fn line_accessibility_kind(line: &RenderLine, side: ViewportTextSide) -> &'static str {
    if line.flags & RENDER_FLAG_STRUCTURAL != 0 {
        return "Context";
    }
    match (line.row_kind(), side) {
        (RenderRowKind::Added, _) => "Added",
        (RenderRowKind::Removed, _) => "Removed",
        (RenderRowKind::Modified, ViewportTextSide::Left) => "Modified old",
        (RenderRowKind::Modified, ViewportTextSide::Right) => "Modified new",
        (RenderRowKind::Context, ViewportTextSide::Left) => "Context old",
        (RenderRowKind::Context, ViewportTextSide::Right) => "Context new",
        _ => "Line",
    }
}

fn line_accessibility_location(
    split_mode: bool,
    line: &RenderLine,
    side: ViewportTextSide,
) -> String {
    if split_mode || line.row_kind() == RenderRowKind::Modified {
        return match side {
            ViewportTextSide::Left => accessible_line_number("old line", line.old_line_no),
            ViewportTextSide::Right => accessible_line_number("new line", line.new_line_no),
        }
        .unwrap_or_default();
    }

    let old = accessible_line_number("old line", line.old_line_no);
    let new = accessible_line_number("new line", line.new_line_no);
    match (old, new) {
        (Some(old), Some(new)) => format!("{old}, {new}"),
        (Some(old), None) => old,
        (None, Some(new)) => new,
        (None, None) => String::new(),
    }
}

fn accessible_line_number(prefix: &str, line_no: u32) -> Option<String> {
    (line_no != INVALID_U32).then(|| format!("{prefix} {line_no}"))
}

fn accessibility_side_key(side: ViewportTextSide) -> &'static str {
    match side {
        ViewportTextSide::Left => "left",
        ViewportTextSide::Right => "right",
    }
}

fn wrap_cols_for_width(
    wrap_enabled: bool,
    wrap_column: u32,
    char_width_px: f32,
    width_px: f32,
) -> u16 {
    if !wrap_enabled {
        return u16::MAX;
    }
    let width_cols = (width_px / char_width_px.max(1.0)).floor() as u32;
    let cols = if wrap_column > 0 {
        width_cols.min(wrap_column)
    } else {
        width_cols
    };
    cols.max(1).min(u16::MAX as u32) as u16
}

fn render_cols_for_width(
    wrap_enabled: bool,
    wrap_column: u32,
    char_width_px: f32,
    width_px: f32,
) -> u16 {
    if wrap_enabled {
        return wrap_cols_for_width(true, wrap_column, char_width_px, width_px);
    }

    let visible_cols = (width_px / char_width_px.max(1.0)).ceil() as u32;
    visible_cols
        .saturating_add(u32::from(UNWRAPPED_RENDER_OVERSCAN_COLS))
        .max(1)
        .min(u16::MAX as u32) as u16
}

fn visible_segment_range_for_block(
    block_y: f32,
    segment_count: u16,
    line_height: f32,
    viewport_top: f32,
    viewport_bottom: f32,
) -> Range<u16> {
    if segment_count == 0 || line_height <= 0.0 {
        return 0..0;
    }

    let max_segments = u32::from(segment_count);
    let start = ((viewport_top - block_y) / line_height).floor().max(0.0) as u32;
    let end = ((viewport_bottom - block_y) / line_height).ceil().max(0.0) as u32;
    let start = start.min(max_segments);
    let end = end.max(start).min(max_segments);
    start as u16..end as u16
}

fn paint_column_range_rects_with_vertical_inset(
    scene: &mut Scene,
    col_start: u32,
    col_end: u32,
    text_x: f32,
    row_y: f32,
    text_width: f32,
    char_w: f32,
    line_height: f32,
    segment_cols: u16,
    visible_segments: Range<u16>,
    color: Color,
    corner_radius: f32,
    y_inset: f32,
) {
    if col_end <= col_start {
        return;
    }

    let segment_cols = u32::from(segment_cols.max(1));
    let first_segment = (col_start / segment_cols) as u16;
    let last_segment = ((col_end - 1) / segment_cols).saturating_add(1) as u16;
    let start = first_segment.max(visible_segments.start);
    let end = last_segment.min(visible_segments.end);
    let y_inset = y_inset.min((line_height * 0.35).max(0.0));
    let height = (line_height - y_inset * 2.0).max(1.0);

    for seg in start..end {
        let segment_start_col = u32::from(seg) * segment_cols;
        let local_start = col_start.max(segment_start_col) - segment_start_col;
        let local_end =
            col_end.min(segment_start_col.saturating_add(segment_cols)) - segment_start_col;
        if local_end <= local_start {
            continue;
        }

        let x = text_x + local_start as f32 * char_w;
        let width = (local_end - local_start) as f32 * char_w;
        let clamped_width = width.min((text_x + text_width - x).max(0.0));
        if clamped_width <= 0.0 {
            continue;
        }

        scene.rounded_rect(RoundedRectPrimitive::uniform(
            Rect {
                x,
                y: row_y + seg as f32 * line_height + y_inset,
                width: clamped_width,
                height,
            },
            corner_radius,
            color,
        ));
    }
}

fn paint_column_range_rects(
    scene: &mut Scene,
    col_start: u32,
    col_end: u32,
    text_x: f32,
    row_y: f32,
    text_width: f32,
    char_w: f32,
    line_height: f32,
    segment_cols: u16,
    visible_segments: Range<u16>,
    color: Color,
    corner_radius: Option<f32>,
) {
    if col_end <= col_start {
        return;
    }

    let segment_cols = u32::from(segment_cols.max(1));
    let first_segment = (col_start / segment_cols) as u16;
    let last_segment = ((col_end - 1) / segment_cols).saturating_add(1) as u16;
    let start = first_segment.max(visible_segments.start);
    let end = last_segment.min(visible_segments.end);

    for seg in start..end {
        let segment_start_col = u32::from(seg) * segment_cols;
        let local_start = col_start.max(segment_start_col) - segment_start_col;
        let local_end =
            col_end.min(segment_start_col.saturating_add(segment_cols)) - segment_start_col;
        if local_end <= local_start {
            continue;
        }

        let x = text_x + local_start as f32 * char_w;
        let width = (local_end - local_start) as f32 * char_w;
        let clamped_width = width.min((text_x + text_width - x).max(0.0));
        if clamped_width <= 0.0 {
            continue;
        }

        let rect = Rect {
            x,
            y: row_y + seg as f32 * line_height,
            width: clamped_width,
            height: line_height,
        };
        if let Some(radius) = corner_radius {
            scene.rounded_rect(RoundedRectPrimitive::uniform(rect, radius, color));
        } else {
            scene.rect(RectPrimitive { rect, color });
        }
    }
}

fn build_wrapped_rich_text(
    doc: &RenderDoc,
    text_layout: &CachedTextLayout,
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
    let full_text = doc.line_text(text_range);
    let spans: Arc<[RichTextSpan]> = if full_text.is_empty() {
        Arc::from(Vec::new())
    } else {
        let (start_col, end_col) = wrapped_col_slice(text_layout, wrap_cols, segment_index)?;
        Arc::from(build_segment_spans(
            full_text,
            text_layout,
            start_col,
            end_col,
            doc.line_runs(runs),
            tone,
            theme,
        ))
    };
    Some(spans)
}

fn wrapped_col_slice(
    text_layout: &CachedTextLayout,
    wrap_cols: u16,
    segment_index: u16,
) -> Option<(u32, u32)> {
    if wrap_cols == u16::MAX {
        return (segment_index == 0).then(|| (0, text_layout.total_cols()));
    }

    let total_cols = text_layout.total_cols();
    let segment_cols = u32::from(wrap_cols.max(1));
    let start_col = u32::from(segment_index).saturating_mul(segment_cols);
    if start_col >= total_cols {
        return None;
    }
    let end_col = start_col.saturating_add(segment_cols).min(total_cols);
    Some((start_col, end_col))
}

#[cfg(test)]
fn wrapped_byte_slice(
    text_layout: &CachedTextLayout,
    wrap_cols: u16,
    segment_index: u16,
) -> Option<(usize, usize)> {
    let (start_col, end_col) = wrapped_col_slice(text_layout, wrap_cols, segment_index)?;
    Some(text_layout.byte_range_for_cols(start_col, end_col))
}

fn build_segment_spans(
    full_text: &str,
    text_layout: &CachedTextLayout,
    segment_start_col: u32,
    segment_end_col: u32,
    runs: &[StyleRun],
    tone: RowTone,
    theme: &Theme,
) -> Vec<RichTextSpan> {
    let mut spans = Vec::new();
    let (char_start, char_end) =
        text_layout.char_range_for_cols(segment_start_col, segment_end_col);
    if char_start >= char_end {
        return spans;
    }

    let mut current_text = String::new();
    let mut current_style = None;
    let mut run_index = runs.partition_point(|run| {
        let run_end = run.byte_start.saturating_add(run.byte_len);
        run_end as usize <= text_layout.char_boundaries[char_start] as usize
    });

    for char_index in char_start..char_end {
        let start = text_layout.char_boundaries[char_index] as usize;
        let end = text_layout.char_boundaries[char_index + 1] as usize;
        if end <= start {
            continue;
        }

        while let Some(run) = runs.get(run_index) {
            let run_end = run.byte_start.saturating_add(run.byte_len) as usize;
            if start < run_end {
                break;
            }
            run_index += 1;
        }

        let col_start = text_layout.col_boundaries[char_index];
        let col_end = text_layout.col_boundaries[char_index + 1];
        let visible_start = segment_start_col.max(col_start);
        let visible_end = segment_end_col.min(col_end);
        if visible_end <= visible_start {
            continue;
        }

        let style = runs
            .get(run_index)
            .map(|run| style_run_text_style(*run, tone, theme))
            .unwrap_or_else(|| CodeTextStyle::default_for_tone(tone, theme));
        let text = if &full_text[start..end] == "\t" {
            " ".repeat((visible_end - visible_start) as usize)
        } else {
            full_text[start..end].to_owned()
        };

        if current_style == Some(style) {
            current_text.push_str(&text);
            continue;
        }

        if !current_text.is_empty() {
            let style =
                current_style.unwrap_or_else(|| CodeTextStyle::default_for_tone(tone, theme));
            spans.push(RichTextSpan {
                text: current_text.into(),
                color: style.color,
                font_weight: style.font_weight,
                font_style: style.font_style,
            });
            current_text = String::new();
        }

        current_style = Some(style);
        current_text.push_str(&text);
    }

    if !current_text.is_empty() {
        let style = current_style.unwrap_or_else(|| CodeTextStyle::default_for_tone(tone, theme));
        spans.push(RichTextSpan {
            text: current_text.into(),
            color: style.color,
            font_weight: style.font_weight,
            font_style: style.font_style,
        });
    }

    spans
}

#[derive(Clone, Copy, PartialEq)]
struct CodeTextStyle {
    color: Color,
    font_weight: Option<FontWeight>,
    font_style: Option<FontStyle>,
}

impl CodeTextStyle {
    fn default_for_tone(tone: RowTone, theme: &Theme) -> Self {
        Self {
            color: tone.default_text(theme),
            font_weight: None,
            font_style: None,
        }
    }
}

fn style_run_text_style(run: StyleRun, tone: RowTone, theme: &Theme) -> CodeTextStyle {
    let syntax_kind = syntax_kind_from_style_id(run.style_id);
    let color = match syntax_kind {
        SyntaxTokenKind::Keyword | SyntaxTokenKind::Builtin => theme.colors.syntax_keyword,
        SyntaxTokenKind::String => theme.colors.syntax_string,
        SyntaxTokenKind::Comment | SyntaxTokenKind::Label | SyntaxTokenKind::Preprocessor => {
            theme.colors.syntax_comment
        }
        SyntaxTokenKind::Function => theme.colors.syntax_function,
        SyntaxTokenKind::Number | SyntaxTokenKind::Constant => theme.colors.syntax_number,
        SyntaxTokenKind::Type | SyntaxTokenKind::Namespace | SyntaxTokenKind::Tag => {
            theme.colors.syntax_type
        }
        SyntaxTokenKind::Attribute | SyntaxTokenKind::Property => theme.colors.syntax_property,
        SyntaxTokenKind::Operator | SyntaxTokenKind::Punctuation => theme.colors.syntax_operator,
        SyntaxTokenKind::Variable | SyntaxTokenKind::Normal => tone.default_text(theme),
    };
    let (font_weight, font_style) = syntax_kind_font_style(syntax_kind);
    CodeTextStyle {
        color,
        font_weight,
        font_style,
    }
}

fn syntax_kind_font_style(syntax_kind: SyntaxTokenKind) -> (Option<FontWeight>, Option<FontStyle>) {
    match syntax_kind {
        SyntaxTokenKind::Comment => (None, Some(FontStyle::Italic)),
        SyntaxTokenKind::Keyword | SyntaxTokenKind::Builtin => (Some(FontWeight::Semibold), None),
        SyntaxTokenKind::Type
        | SyntaxTokenKind::Function
        | SyntaxTokenKind::Constant
        | SyntaxTokenKind::Attribute
        | SyntaxTokenKind::Tag
        | SyntaxTokenKind::Property
        | SyntaxTokenKind::Namespace
        | SyntaxTokenKind::Label
        | SyntaxTokenKind::Preprocessor => (Some(FontWeight::Medium), None),
        SyntaxTokenKind::Normal
        | SyntaxTokenKind::String
        | SyntaxTokenKind::Number
        | SyntaxTokenKind::Operator
        | SyntaxTokenKind::Punctuation
        | SyntaxTokenKind::Variable => (None, None),
    }
}

fn syntax_kind_from_style_id(style_id: u16) -> SyntaxTokenKind {
    match style_id as u8 {
        1 => SyntaxTokenKind::Keyword,
        2 => SyntaxTokenKind::String,
        3 => SyntaxTokenKind::Comment,
        4 => SyntaxTokenKind::Number,
        5 => SyntaxTokenKind::Type,
        6 => SyntaxTokenKind::Function,
        7 => SyntaxTokenKind::Operator,
        8 => SyntaxTokenKind::Punctuation,
        9 => SyntaxTokenKind::Variable,
        10 => SyntaxTokenKind::Constant,
        11 => SyntaxTokenKind::Builtin,
        12 => SyntaxTokenKind::Attribute,
        13 => SyntaxTokenKind::Tag,
        14 => SyntaxTokenKind::Property,
        15 => SyntaxTokenKind::Namespace,
        16 => SyntaxTokenKind::Label,
        17 => SyntaxTokenKind::Preprocessor,
        _ => SyntaxTokenKind::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CachedTextLayout, EditorDocument, EditorElement, build_wrapped_rich_text,
        render_cols_for_width, visible_segment_range_for_block, wrapped_byte_slice,
    };
    use crate::core::compare::LayoutMode;
    use crate::render::{FontStyle, FontWeight, Rect, TextMetrics};
    use crate::ui::editor::render_doc::{
        ByteRange, RenderDoc, RenderLine, RenderRowKind, RunRange,
    };
    use crate::ui::editor::state::{
        EditorState, ViewportTextPoint, ViewportTextSelection, ViewportTextSide,
    };
    use crate::ui::theme::Theme;

    #[test]
    fn wrapped_byte_slice_breaks_monospaced_text_by_columns() {
        let layout = CachedTextLayout::new("abcdefghij");
        assert_eq!(wrapped_byte_slice(&layout, 4, 0), Some((0, 4)));
        assert_eq!(wrapped_byte_slice(&layout, 4, 1), Some((4, 8)));
        assert_eq!(wrapped_byte_slice(&layout, 4, 2), Some((8, 10)));
        assert_eq!(wrapped_byte_slice(&layout, 4, 3), None);
    }

    #[test]
    fn cached_text_layout_tracks_visual_columns_for_tabs() {
        let layout = CachedTextLayout::new("\ta\t");
        assert_eq!(layout.total_cols(), 16);
        assert_eq!(layout.col_for_byte(0), 0);
        assert_eq!(layout.col_for_byte(1), 8);
        assert_eq!(layout.col_for_byte(2), 9);
        assert_eq!(layout.col_for_byte(3), 16);
    }

    #[test]
    fn rich_text_builder_returns_spans_for_requested_segment() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: b"keyword // comment".to_vec(),
            style_runs: vec![
                crate::ui::editor::render_doc::StyleRun {
                    byte_start: 0,
                    byte_len: 7,
                    style_id: 1,
                    flags: 0,
                },
                crate::ui::editor::render_doc::StyleRun {
                    byte_start: 7,
                    byte_len: 1,
                    style_id: 0,
                    flags: 0,
                },
                crate::ui::editor::render_doc::StyleRun {
                    byte_start: 8,
                    byte_len: 10,
                    style_id: 3,
                    flags: 0,
                },
            ],
            lines: vec![RenderLine {
                kind: RenderRowKind::Context as u8,
                right_text: ByteRange { start: 0, len: 18 },
                right_runs: RunRange { start: 0, len: 3 },
                right_cols: 18,
                ..RenderLine::default()
            }],
        };

        let text_layout = CachedTextLayout::new("keyword // comment");
        let spans = build_wrapped_rich_text(
            &doc,
            &text_layout,
            doc.lines[0].right_text,
            doc.lines[0].right_runs,
            0,
            u16::MAX,
            super::RowTone::Neutral,
            &Theme::default_dark(),
        )
        .expect("spans");

        assert!(!spans.is_empty());
        assert_eq!(
            spans
                .iter()
                .map(|span| span.text.as_ref())
                .collect::<String>(),
            "keyword // comment"
        );
        assert_eq!(spans[0].text.as_ref(), "keyword");
        assert_eq!(spans[0].font_weight, Some(FontWeight::Semibold));
        let comment = spans
            .iter()
            .find(|span| span.text.as_ref() == "// comment")
            .expect("comment span");
        assert_eq!(comment.font_style, Some(FontStyle::Italic));
    }

    #[test]
    fn rich_text_builder_expands_tabs_across_wrapped_segments() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: b"\tabc".to_vec(),
            style_runs: vec![crate::ui::editor::render_doc::StyleRun {
                byte_start: 0,
                byte_len: 4,
                style_id: 0,
                flags: 0,
            }],
            lines: vec![RenderLine {
                kind: RenderRowKind::Context as u8,
                right_text: ByteRange { start: 0, len: 4 },
                right_runs: RunRange { start: 0, len: 1 },
                right_cols: 11,
                ..RenderLine::default()
            }],
        };

        let text_layout = CachedTextLayout::new("\tabc");
        let theme = Theme::default_dark();

        let seg0 = build_wrapped_rich_text(
            &doc,
            &text_layout,
            doc.lines[0].right_text,
            doc.lines[0].right_runs,
            0,
            4,
            super::RowTone::Neutral,
            &theme,
        )
        .expect("segment 0");
        let seg1 = build_wrapped_rich_text(
            &doc,
            &text_layout,
            doc.lines[0].right_text,
            doc.lines[0].right_runs,
            1,
            4,
            super::RowTone::Neutral,
            &theme,
        )
        .expect("segment 1");
        let seg2 = build_wrapped_rich_text(
            &doc,
            &text_layout,
            doc.lines[0].right_text,
            doc.lines[0].right_runs,
            2,
            4,
            super::RowTone::Neutral,
            &theme,
        )
        .expect("segment 2");

        assert_eq!(
            seg0.iter()
                .map(|span| span.text.as_ref())
                .collect::<String>(),
            "    "
        );
        assert_eq!(
            seg1.iter()
                .map(|span| span.text.as_ref())
                .collect::<String>(),
            "    "
        );
        assert_eq!(
            seg2.iter()
                .map(|span| span.text.as_ref())
                .collect::<String>(),
            "abc"
        );
    }

    #[test]
    fn render_cols_cap_unwrapped_rows_to_viewport_budget() {
        assert_eq!(render_cols_for_width(false, 0, 8.0, 80.0), 26);
        assert_eq!(render_cols_for_width(true, 0, 8.0, 80.0), 10);
    }

    #[test]
    fn visible_segment_range_limits_wrapped_blocks_to_viewport() {
        assert_eq!(
            visible_segment_range_for_block(100.0, 10, 20.0, 120.0, 170.0),
            1..4
        );
        assert_eq!(
            visible_segment_range_for_block(100.0, 10, 20.0, 0.0, 50.0),
            0..0
        );
    }

    #[test]
    fn prepare_populates_visible_range_and_hit_testing() {
        let mut state = EditorState {
            layout: LayoutMode::Unified,
            ..EditorState::default()
        };
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: b"demo.txt@@ -1 +1 @@line".to_vec(),
            style_runs: Vec::new(),
            lines: vec![
                RenderLine {
                    kind: RenderRowKind::FileHeader as u8,
                    left_text: ByteRange { start: 0, len: 8 },
                    left_cols: 8,
                    ..RenderLine::default()
                },
                RenderLine {
                    kind: RenderRowKind::HunkSeparator as u8,
                    left_text: ByteRange { start: 8, len: 11 },
                    left_cols: 11,
                    ..RenderLine::default()
                },
                RenderLine {
                    kind: RenderRowKind::Context as u8,
                    old_line_no: 1,
                    new_line_no: 1,
                    right_text: ByteRange { start: 19, len: 4 },
                    right_cols: 4,
                    ..RenderLine::default()
                },
            ],
        };

        let mut runtime = EditorElement::default();
        runtime.prepare(
            &mut state,
            EditorDocument::Text {
                compare_generation: 1,
                file_index: 0,
                path: "demo.txt",
                doc: &doc,
                show_file_headers: false,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            TextMetrics::default(),
        );

        assert_eq!(state.visible_row_start, Some(0));
        // FileHeader lines are skipped in layout, so only 2 display rows exist.
        assert!(state.visible_row_end.expect("visible end") >= 2);
        let body = runtime.body_bounds();
        assert_eq!(
            runtime.hit_test_row(&state, body.x + 20.0, body.y + 5.0),
            Some(0)
        );
    }

    #[test]
    fn hit_test_text_point_maps_viewport_columns_to_line_bytes() {
        let mut state = EditorState {
            layout: LayoutMode::Unified,
            ..EditorState::default()
        };
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: b"hello".to_vec(),
            style_runs: Vec::new(),
            lines: vec![RenderLine {
                kind: RenderRowKind::Context as u8,
                old_line_no: 1,
                new_line_no: 1,
                right_text: ByteRange { start: 0, len: 5 },
                right_cols: 5,
                ..RenderLine::default()
            }],
        };
        let mut runtime = EditorElement::default();
        runtime.prepare(
            &mut state,
            EditorDocument::Text {
                compare_generation: 1,
                file_index: 0,
                path: "demo.txt",
                doc: &doc,
                show_file_headers: false,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            TextMetrics::default(),
        );

        let x =
            runtime.layout.unified_text_rect.x + TextMetrics::default().mono_char_width_px * 3.1;
        let y = runtime.body_bounds().y + runtime.layout.line_height * 0.5;
        let point = runtime
            .hit_test_text_point(&state, &doc, x, y)
            .expect("text point");

        assert_eq!(
            point,
            ViewportTextPoint {
                line_index: 0,
                side: ViewportTextSide::Right,
                byte_offset: 3,
            }
        );
    }

    #[test]
    fn viewport_selection_text_copies_visible_line_segments() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: b"alphaBRAVO".to_vec(),
            style_runs: Vec::new(),
            lines: vec![
                RenderLine {
                    kind: RenderRowKind::Context as u8,
                    old_line_no: 1,
                    new_line_no: 1,
                    right_text: ByteRange { start: 0, len: 5 },
                    right_cols: 5,
                    ..RenderLine::default()
                },
                RenderLine {
                    kind: RenderRowKind::Context as u8,
                    old_line_no: 2,
                    new_line_no: 2,
                    right_text: ByteRange { start: 5, len: 5 },
                    right_cols: 5,
                    ..RenderLine::default()
                },
            ],
        };
        let selection = ViewportTextSelection {
            generation: 7,
            anchor: ViewportTextPoint {
                line_index: 0,
                side: ViewportTextSide::Right,
                byte_offset: 1,
            },
            focus: ViewportTextPoint {
                line_index: 1,
                side: ViewportTextSide::Right,
                byte_offset: 3,
            },
        };
        let runtime = EditorElement::default();

        assert_eq!(
            runtime.viewport_selection_text(&doc, &selection).as_deref(),
            Some("lpha\nBRA")
        );
    }

    #[test]
    fn split_viewport_selection_text_stays_on_selected_side() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: b"old-aNEW-Aold-bNEW-B".to_vec(),
            style_runs: Vec::new(),
            lines: vec![
                RenderLine {
                    kind: RenderRowKind::Modified as u8,
                    old_line_no: 1,
                    new_line_no: 1,
                    left_text: ByteRange { start: 0, len: 5 },
                    right_text: ByteRange { start: 5, len: 5 },
                    left_cols: 5,
                    right_cols: 5,
                    ..RenderLine::default()
                },
                RenderLine {
                    kind: RenderRowKind::Modified as u8,
                    old_line_no: 2,
                    new_line_no: 2,
                    left_text: ByteRange { start: 10, len: 5 },
                    right_text: ByteRange { start: 15, len: 5 },
                    left_cols: 5,
                    right_cols: 5,
                    ..RenderLine::default()
                },
            ],
        };
        let selection = ViewportTextSelection {
            generation: 7,
            anchor: ViewportTextPoint {
                line_index: 0,
                side: ViewportTextSide::Left,
                byte_offset: 1,
            },
            focus: ViewportTextPoint {
                line_index: 1,
                side: ViewportTextSide::Left,
                byte_offset: 4,
            },
        };
        let mut runtime = EditorElement::default();
        runtime.layout.split_mode = true;

        assert_eq!(
            runtime.viewport_selection_text(&doc, &selection).as_deref(),
            Some("ld-a\nold-")
        );
    }

    #[test]
    fn viewport_text_selection_paints_square_rectangles() {
        use crate::render::{Primitive, Scene};

        let mut state = EditorState {
            layout: LayoutMode::Unified,
            text_selection: Some(ViewportTextSelection {
                generation: 1,
                anchor: ViewportTextPoint {
                    line_index: 0,
                    side: ViewportTextSide::Right,
                    byte_offset: 1,
                },
                focus: ViewportTextPoint {
                    line_index: 1,
                    side: ViewportTextSide::Right,
                    byte_offset: 4,
                },
            }),
            ..EditorState::default()
        };
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: b"alphabravo".to_vec(),
            style_runs: Vec::new(),
            lines: vec![
                RenderLine {
                    kind: RenderRowKind::Context as u8,
                    old_line_no: 1,
                    new_line_no: 1,
                    right_text: ByteRange { start: 0, len: 5 },
                    right_cols: 5,
                    ..RenderLine::default()
                },
                RenderLine {
                    kind: RenderRowKind::Context as u8,
                    old_line_no: 2,
                    new_line_no: 2,
                    right_text: ByteRange { start: 5, len: 5 },
                    right_cols: 5,
                    ..RenderLine::default()
                },
            ],
        };
        let mut runtime = EditorElement::default();
        let document = EditorDocument::Text {
            compare_generation: 1,
            file_index: 0,
            path: "demo.txt",
            doc: &doc,
            show_file_headers: false,
        };
        runtime.prepare(
            &mut state,
            document,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            TextMetrics::default(),
        );

        let theme = Theme::default_dark();
        let selection_bg = theme.colors.selection_bg;
        let mut scene = Scene::default();
        runtime.paint(&mut scene, &theme, &state, document);

        assert!(
            scene
                .primitives
                .iter()
                .any(|p| matches!(p, Primitive::Rect(r) if r.color == selection_bg))
        );
        assert!(
            !scene
                .primitives
                .iter()
                .any(|p| matches!(p, Primitive::RoundedRect(r) if r.color == selection_bg))
        );
    }

    #[test]
    fn block_paint_emits_primitive_for_registered_decoration() {
        use super::super::decoration::{BlockDecoration, BlockPaintCtx, BlockPlacement};
        use crate::render::{Primitive, RectPrimitive, Scene};
        use crate::ui::theme::Color;

        #[derive(Debug)]
        struct StubBlock {
            color: Color,
        }

        impl BlockDecoration for StubBlock {
            fn height(&self, _metrics: &super::super::display_layout::DisplayLayoutMetrics) -> u16 {
                20
            }

            fn paint(&self, ctx: &mut BlockPaintCtx) {
                let _ = ctx.hovered;
                ctx.scene.rect(RectPrimitive {
                    rect: ctx.row_rect,
                    color: self.color,
                });
            }
        }

        let mut state = EditorState {
            layout: LayoutMode::Unified,
            ..EditorState::default()
        };
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: b"@@ hdr @@".to_vec(),
            style_runs: Vec::new(),
            lines: vec![RenderLine {
                kind: RenderRowKind::HunkSeparator as u8,
                left_text: ByteRange { start: 0, len: 9 },
                left_cols: 9,
                ..RenderLine::default()
            }],
        };

        let marker = Color {
            r: 11,
            g: 22,
            b: 33,
            a: 255,
        };

        let mut runtime = EditorElement::default();
        runtime.blocks_mut().push(
            BlockPlacement::Above(0),
            Box::new(StubBlock { color: marker }),
        );

        let document = EditorDocument::Text {
            compare_generation: 7,
            file_index: 0,
            path: "demo.txt",
            doc: &doc,
            show_file_headers: false,
        };
        runtime.prepare(
            &mut state,
            document,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            TextMetrics::default(),
        );

        let theme = Theme::default_dark();
        let mut scene = Scene::default();
        runtime.paint(&mut scene, &theme, &state, document);

        let has_marker = scene.primitives.iter().any(|p| match p {
            Primitive::Rect(r) => r.color == marker,
            _ => false,
        });
        assert!(has_marker, "block paint() should emit its own primitives");
    }

    #[test]
    fn hunk_separator_decoration_emits_background_rect() {
        use crate::render::{Primitive, Scene};

        let mut state = EditorState {
            layout: LayoutMode::Unified,
            ..EditorState::default()
        };
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: b"@@ hdr @@".to_vec(),
            style_runs: Vec::new(),
            lines: vec![RenderLine {
                kind: RenderRowKind::HunkSeparator as u8,
                left_text: ByteRange { start: 0, len: 9 },
                left_cols: 9,
                ..RenderLine::default()
            }],
        };

        let mut runtime = EditorElement::default();
        let document = EditorDocument::Text {
            compare_generation: 1,
            file_index: 0,
            path: "demo.txt",
            doc: &doc,
            show_file_headers: false,
        };
        runtime.prepare(
            &mut state,
            document,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            TextMetrics::default(),
        );

        let theme = Theme::default_dark();
        let mut scene = Scene::default();
        runtime.paint(&mut scene, &theme, &state, document);

        let hunk_bg = theme.colors.hunk_header_bg;
        let has_hunk_bg = scene.primitives.iter().any(|p| match p {
            Primitive::Rect(r) => r.color == hunk_bg,
            _ => false,
        });
        assert!(
            has_hunk_bg,
            "expected a rect with hunk_header_bg color to be emitted"
        );
    }
}
