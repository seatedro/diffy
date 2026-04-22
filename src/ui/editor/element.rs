use std::{collections::HashMap, ops::Range, sync::Arc};

use crate::core::compare::LayoutMode;
use crate::core::text::SyntaxTokenKind;
use crate::render::{
    FontKind, FontWeight, Rect, RectPrimitive, RichTextPrimitive, RichTextSpan,
    RoundedRectPrimitive, Scene, TextMetrics, TextPrimitive,
};
use crate::ui::design::{Alpha, Sz};
use crate::ui::theme::{Color, Theme};

use super::decoration::{BlockPaintCtx, BlockRegistry, RowPaintCtx, decoration_for_kind};
use super::display_layout::{
    DisplayLayoutConfig, DisplayLayoutMetrics, DisplayLayoutSummary, compute_gutter_digits,
    rebuild_display_rows,
};
use super::render_doc::{
    ByteRange, DisplayRow, INVALID_U32, RenderDoc, RenderLine, RenderRowKind, RunRange,
    STYLE_FLAG_NOVEL_WORD, StyleRun, advance_display_col,
};
use super::state::EditorState;
use super::strip_layout::{StripLayout, build_strip_layouts, visible_strip_range};

const BASE_VIEWPORT_PADDING: f32 = 14.0;
const BASE_COLUMN_GAP: f32 = 18.0;
const BASE_GUTTER_PADDING: f32 = 8.0;
const BASE_SCROLLBAR_WIDTH: f32 = 8.0;
const BASE_SCROLLBAR_MARGIN: f32 = 6.0;
const FILE_HEADER_ROW_MULTIPLE: u16 = 2;
const HUNK_ROW_MULTIPLE: u16 = 1;
const BASE_SCROLLBAR_THUMB_MIN: f32 = 32.0;
const BASE_MONO_FONT_SIZE: f32 = 13.0;
const STRIP_TARGET_HEIGHT_PX: u32 = 480;
const STRIP_OVERSCAN: usize = 1;
const UNWRAPPED_RENDER_OVERSCAN_COLS: u16 = 16;

fn editor_scale(text_metrics: TextMetrics) -> f32 {
    (text_metrics.mono_font_size_px / BASE_MONO_FONT_SIZE).max(0.5)
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
}

#[derive(Debug, Clone, Copy)]
pub enum EditorDocument<'a> {
    Empty,
    Binary {
        path: &'a str,
    },
    Text {
        compare_generation: u64,
        file_index: usize,
        path: &'a str,
        doc: &'a RenderDoc,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct EditorLayoutKey {
    compare_generation: u64,
    file_index: usize,
    split_mode: bool,
    wrap_enabled: bool,
    wrap_column: u32,
    viewport_width_bits: u32,
    viewport_height_bits: u32,
    mono_char_width_bits: u32,
    mono_line_height_bits: u32,
    doc_line_count: u32,
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
        }
    }
}

impl EditorElement {
    pub fn scrollbar_rect(&self) -> Rect {
        self.layout.scrollbar.map(|sb| sb.track).unwrap_or_default()
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
                ..
            } => {
                let key = EditorLayoutKey {
                    compare_generation,
                    file_index,
                    split_mode: state.layout == LayoutMode::Split,
                    wrap_enabled: state.wrap_enabled,
                    wrap_column: state.wrap_column,
                    viewport_width_bits: self.layout.content_bounds.width.to_bits(),
                    viewport_height_bits: self.layout.content_bounds.height.to_bits(),
                    mono_char_width_bits: text_metrics.mono_char_width_px.to_bits(),
                    mono_line_height_bits: text_metrics.mono_line_height_px.to_bits(),
                    doc_line_count: doc.line_count() as u32,
                };

                if self.layout_key != Some(key) {
                    self.rebuild_rows(doc, state, text_metrics);
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
        self.layout.scrollbar = compute_scrollbar_layout(&self.layout, state);

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

    pub fn set_hunk_expand_caps(&mut self, caps: Vec<super::expansion::HunkGapBudget>) {
        self.hunk_expand_caps = caps;
    }

    fn row_gutter_rect(&self, row_rect: Rect) -> Rect {
        let gutter = if self.layout.split_mode {
            self.layout.left_gutter_rect
        } else {
            self.layout.unified_gutter_rect
        };
        Rect {
            x: gutter.x,
            y: row_rect.y,
            width: gutter.width,
            height: row_rect.height,
        }
    }

    fn paint_hunk_expand_icon(
        &self,
        scene: &mut Scene,
        theme: &Theme,
        row_rect: Rect,
        display_row_index: usize,
    ) {
        use crate::render::scene::{IconPrimitive, Primitive};
        let gutter = self.row_gutter_rect(row_rect);
        let hovered = self.layout.highlighted_row == Some(display_row_index);
        if hovered {
            scene.rect(RectPrimitive {
                rect: gutter,
                color: theme.colors.element_hover,
            });
        }
        let color = if hovered {
            theme.colors.text_strong
        } else {
            theme.colors.text_muted
        };
        let icon_size = self.layout.line_height.min(gutter.width).max(8.0) * 0.75;
        let icon_x = gutter.x + (gutter.width - icon_size) * 0.5;
        let icon_y = gutter.y + (gutter.height - icon_size) * 0.5;
        scene.push(Primitive::Icon(IconPrimitive {
            rect: Rect {
                x: icon_x.round(),
                y: icon_y.round(),
                width: icon_size.round(),
                height: icon_size.round(),
            },
            name: crate::ui::icons::lucide::CHEVRON_UP.to_owned(),
            color,
        }));
    }

    pub fn hunk_expand_action_for_row(
        &self,
        display_row_index: usize,
        doc: &RenderDoc,
    ) -> Option<crate::actions::Action> {
        let row = self.rows.get(display_row_index)?;
        if row.kind != RenderRowKind::HunkSeparator as u8 {
            return None;
        }
        let line = doc.lines.get(row.line_index as usize)?;
        let hunk_index = usize::try_from(line.hunk_index).ok()?;
        let cap = self.hunk_expand_caps.get(hunk_index)?;
        if cap.above_cap == 0 {
            return None;
        }
        let step = cap.above_cap.min(20).max(1);
        Some(crate::actions::Action::ExpandContextAbove(hunk_index, step))
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

    pub fn block_action_for_row(&self, display_row_index: usize) -> Option<crate::actions::Action> {
        let row = self.rows.get(display_row_index)?;
        if !row.is_block() {
            return None;
        }
        self.blocks.get(row.block_index as usize)?.on_click()
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
            (line.old_line_index >= 0
                && state
                    .line_selection
                    .contains(line.hunk_index, line.old_line_index))
                || (line.new_line_index >= 0
                    && state
                        .line_selection
                        .contains(line.hunk_index, line.new_line_index))
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

    fn rebuild_rows(&mut self, doc: &RenderDoc, state: &EditorState, text_metrics: TextMetrics) {
        let body_h = text_metrics.mono_line_height_px.round().max(1.0) as u16;
        self.metrics = DisplayLayoutMetrics {
            body_row_height_px: body_h,
            file_header_height_px: body_h * FILE_HEADER_ROW_MULTIPLE,
            hunk_height_px: body_h * HUNK_ROW_MULTIPLE,
        };
        self.config = DisplayLayoutConfig {
            split_mode: state.layout == LayoutMode::Split,
            wrap_enabled: state.wrap_enabled,
            wrap_column: state.wrap_column,
            char_width_px: text_metrics.mono_char_width_px as f64,
            unified_text_width_px: self.layout.unified_text_rect.width as f64,
            split_text_width_px: self.layout.left_text_rect.width as f64,
        };
        self.summary =
            rebuild_display_rows(doc, self.config, self.metrics, &self.blocks, &mut self.rows);
        build_strip_layouts(&self.rows, STRIP_TARGET_HEIGHT_PX, &mut self.strips);
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
            EditorDocument::Binary { path } => {
                self.paint_placeholder(
                    scene,
                    theme,
                    path,
                    "Binary file. The native viewport only renders text diffs in this phase.",
                );
            }
            EditorDocument::Text { path, doc, .. } => {
                self.sync_theme_cache(theme);
                scene.clip(self.layout.content_bounds);

                self.paint_gutter_backgrounds(scene, theme);
                self.paint_row_backgrounds(scene, theme, path, doc);
                self.paint_inline_change_backgrounds(scene, theme, doc);
                self.paint_line_highlights(scene, theme);
                self.paint_line_selection(scene, theme, _state, doc);
                self.paint_search_highlights(scene, theme, _state, doc);
                self.paint_gutter_diff_indicators(scene, theme, doc);
                self.paint_gutter_decorations(scene, theme);
                self.paint_gutter_text(scene, theme, doc);
                self.paint_body_text(scene, theme, path, doc);

                scene.pop_clip();
                self.paint_scrollbar(scene, theme);
            }
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
                let mut ctx = RowPaintCtx {
                    scene,
                    theme,
                    layout: &self.layout,
                    row_rect: rr,
                    text_y_offset,
                    font_size,
                    line,
                    doc,
                    path,
                };
                deco.paint_background(&mut ctx);
            } else {
                paint_row_background(scene, theme, rr, kind);
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
        let del_bg = dim_bg(theme.colors.line_del);
        let add_bg = dim_bg(theme.colors.line_add);
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
            if kind != RenderRowKind::Modified {
                continue;
            }

            if self.layout.split_mode {
                if line.left_text.is_valid() {
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
                        if self.config.wrap_enabled {
                            display_row.wrap_left.max(1)
                        } else {
                            1
                        },
                        self.render_cols_split(),
                        theme.colors.line_del_word_bg,
                    );
                }
                if line.right_text.is_valid() {
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
                        if self.config.wrap_enabled {
                            display_row.wrap_right.max(1)
                        } else {
                            1
                        },
                        self.render_cols_split(),
                        theme.colors.line_add_word_bg,
                    );
                }
            } else if line.left_text.is_valid() && line.right_text.is_valid() {
                let del_y = rr.y;
                let add_y = rr.y + display_row.wrap_left.max(1) as f32 * line_height;
                self.paint_change_rects_for_side(
                    scene,
                    doc,
                    line.left_text,
                    line.left_runs,
                    self.layout.unified_text_rect.x,
                    del_y,
                    self.layout.unified_text_rect.width,
                    char_w,
                    line_height,
                    if self.config.wrap_enabled {
                        display_row.wrap_left.max(1)
                    } else {
                        1
                    },
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
                    if self.config.wrap_enabled {
                        display_row.wrap_right.max(1)
                    } else {
                        1
                    },
                    self.render_cols_unified(),
                    theme.colors.line_add_word_bg,
                );
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

        for run in runs {
            if run.flags & STYLE_FLAG_NOVEL_WORD == 0 {
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
            paint_column_range_rects(
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
                Some(3.0),
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
            let selected = (line.old_line_index >= 0
                && state
                    .line_selection
                    .contains(line.hunk_index, line.old_line_index))
                || (line.new_line_index >= 0
                    && state
                        .line_selection
                        .contains(line.hunk_index, line.new_line_index));
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
                let mut ctx = RowPaintCtx {
                    scene,
                    theme,
                    layout: &self.layout,
                    row_rect: rr,
                    text_y_offset: ty,
                    font_size,
                    line: &line,
                    doc,
                    path,
                };
                deco.paint_content(&mut ctx);
                if kind == RenderRowKind::HunkSeparator
                    && let Ok(hunk_index) = usize::try_from(line.hunk_index)
                    && self
                        .hunk_expand_caps
                        .get(hunk_index)
                        .is_some_and(|c| c.above_cap > 0)
                {
                    self.paint_hunk_expand_icon(scene, theme, rr, row_index);
                }
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
                tone_for_left_side(line.row_kind()),
                theme,
            ) {
                scene.rich_text(RichTextPrimitive {
                    rect,
                    spans,
                    default_color: tone_for_left_side(line.row_kind()).default_text(theme),
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
                tone_for_right_side(line.row_kind()),
                theme,
            ) {
                scene.rich_text(RichTextPrimitive {
                    rect,
                    spans,
                    default_color: tone_for_right_side(line.row_kind()).default_text(theme),
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
                RowTone::ModifiedOld,
                theme,
            ) {
                scene.rich_text(RichTextPrimitive {
                    rect,
                    spans,
                    default_color: RowTone::ModifiedOld.default_text(theme),
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
                RowTone::ModifiedNew,
                theme,
            ) {
                scene.rich_text(RichTextPrimitive {
                    rect,
                    spans,
                    default_color: RowTone::ModifiedNew.default_text(theme),
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

fn compute_scrollbar_layout(layout: &EditorLayout, state: &EditorState) -> Option<ScrollbarLayout> {
    if state.content_height_px <= state.viewport_height_px || state.viewport_height_px == 0 {
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
    let ratio = state.viewport_height_px as f32 / state.content_height_px as f32;
    let thumb_min = scaled(BASE_SCROLLBAR_THUMB_MIN, s);
    let thumb_height = (track.height * ratio).max(thumb_min).min(track.height);
    let scroll_range = state.max_scroll_top_px().max(1) as f32;
    let top_ratio = state.scroll_top_px as f32 / scroll_range;
    let thumb_y = track.y + (track.height - thumb_height) * top_ratio;
    Some(ScrollbarLayout {
        track,
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

fn paint_row_background(scene: &mut Scene, theme: &Theme, row_rect: Rect, kind: RenderRowKind) {
    let color = match kind {
        RenderRowKind::Context => theme.colors.canvas,
        RenderRowKind::Added => dim_bg(theme.colors.line_add),
        RenderRowKind::Removed => dim_bg(theme.colors.line_del),
        RenderRowKind::Modified => dim_bg(theme.colors.line_modified),
        RenderRowKind::FileHeader | RenderRowKind::HunkSeparator | RenderRowKind::Block => return,
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

fn unified_body_side(line: &RenderLine) -> Option<(ByteRange, RunRange, RowTone)> {
    match line.row_kind() {
        RenderRowKind::Context => Some((line.right_text, line.right_runs, RowTone::Neutral)),
        RenderRowKind::Added => Some((line.right_text, line.right_runs, RowTone::Added)),
        RenderRowKind::Removed => Some((line.left_text, line.left_runs, RowTone::Removed)),
        _ => None,
    }
}

fn tone_for_left_side(kind: RenderRowKind) -> RowTone {
    match kind {
        RenderRowKind::Modified => RowTone::ModifiedOld,
        RenderRowKind::Removed => RowTone::Removed,
        _ => RowTone::Neutral,
    }
}

fn tone_for_right_side(kind: RenderRowKind) -> RowTone {
    match kind {
        RenderRowKind::Modified => RowTone::ModifiedNew,
        RenderRowKind::Added => RowTone::Added,
        _ => RowTone::Neutral,
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
    let mut current_color = None;
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

        let color = runs
            .get(run_index)
            .map(|run| style_run_color(*run, tone, theme))
            .unwrap_or_else(|| tone.default_text(theme));
        let text = if &full_text[start..end] == "\t" {
            " ".repeat((visible_end - visible_start) as usize)
        } else {
            full_text[start..end].to_owned()
        };

        if current_color == Some(color) {
            current_text.push_str(&text);
            continue;
        }

        if !current_text.is_empty() {
            spans.push(RichTextSpan {
                text: current_text.into(),
                color: current_color.unwrap_or_else(|| tone.default_text(theme)),
            });
            current_text = String::new();
        }

        current_color = Some(color);
        current_text.push_str(&text);
    }

    if !current_text.is_empty() {
        spans.push(RichTextSpan {
            text: current_text.into(),
            color: current_color.unwrap_or_else(|| tone.default_text(theme)),
        });
    }

    spans
}

fn style_run_color(run: StyleRun, tone: RowTone, theme: &Theme) -> Color {
    let base = match syntax_kind_from_style_id(run.style_id) {
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
    base
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
    use crate::render::{Rect, TextMetrics};
    use crate::ui::editor::render_doc::{
        ByteRange, RenderDoc, RenderLine, RenderRowKind, RunRange,
    };
    use crate::ui::editor::state::EditorState;
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
            text_bytes: b"keyword value".to_vec(),
            style_runs: vec![crate::ui::editor::render_doc::StyleRun {
                byte_start: 0,
                byte_len: 7,
                style_id: 1,
                flags: 0,
            }],
            lines: vec![RenderLine {
                kind: RenderRowKind::Context as u8,
                right_text: ByteRange { start: 0, len: 13 },
                right_runs: RunRange { start: 0, len: 1 },
                right_cols: 13,
                ..RenderLine::default()
            }],
        };

        let text_layout = CachedTextLayout::new("keyword value");
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
            "keyword value"
        );
    }

    #[test]
    fn rich_text_builder_expands_tabs_across_wrapped_segments() {
        let doc = RenderDoc {
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
