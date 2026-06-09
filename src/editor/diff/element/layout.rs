use std::ops::Range;
use std::sync::Arc;

use crate::core::compare::LayoutMode;
use crate::editor::diff::anchor::{EditorOverlayKind, ResolvedEditorOverlay};
use crate::editor::diff::display_layout::{
    DisplayLayoutConfig, DisplayLayoutMetrics, compute_gutter_digits, rebuild_display_rows,
};
use crate::editor::diff::render_doc::{DisplayRow, RenderDoc, RenderRowKind};
use crate::editor::diff::state::EditorState;
use crate::editor::diff::strip_layout::{build_strip_layouts, visible_strip_range};
use crate::render::{Rect, TextMetrics};

use super::{
    BASE_MONO_FONT_SIZE, EditorDocument, EditorElement, EditorLayout, EditorLayoutKey,
    FileHeaderHit, ScrollbarLayout, ScrollbarOverride, VisibleRange,
};

const BASE_VIEWPORT_PADDING: f32 = 14.0;
const BASE_COLUMN_GAP: f32 = 18.0;
const BASE_GUTTER_PADDING: f32 = 8.0;
const BASE_SCROLLBAR_WIDTH: f32 = 8.0;
const BASE_SCROLLBAR_MARGIN: f32 = 6.0;
const FILE_HEADER_ROW_MULTIPLE: u16 = 1;
const HUNK_ROW_MULTIPLE: u16 = 1;
const BASE_SCROLLBAR_THUMB_MIN: f32 = 32.0;

const STRIP_TARGET_HEIGHT_PX: u32 = 480;
const STRIP_OVERSCAN: usize = 1;
const UNWRAPPED_RENDER_OVERSCAN_COLS: u16 = 16;

pub(super) fn editor_scale(text_metrics: TextMetrics) -> f32 {
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

pub(super) fn scaled(base: f32, scale: f32) -> f32 {
    base * scale
}

fn content_bounds_for_viewport(bounds: Rect, text_metrics: TextMetrics) -> Rect {
    bounds.inset(scaled(BASE_VIEWPORT_PADDING, editor_scale(text_metrics)))
}

pub(super) fn editor_bottom_padding_px(metrics: DisplayLayoutMetrics) -> u32 {
    u32::from(metrics.body_row_height_px)
}

impl EditorElement {
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

                // Stamp the generation this geometry belongs to. When a new
                // compare generation replaces the document, the carried-over
                // scroll offset may point past geometry that no longer
                // exists; the `clamp_scroll` below re-clamps it against the
                // freshly built layout. Scroll is intentionally not reset
                // here: per-file resets (and continuous-scroll restore) are
                // owned by the reducer so a recompare of the same file keeps
                // the user's place.
                state.doc_generation = compare_generation;

                state.content_height_px = self
                    .summary
                    .content_height_px
                    .saturating_add(editor_bottom_padding_px(self.metrics));
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

    /// Pins the single card-width formula used by the review-thread overlay. Mirrors
    /// the inset `build_spatial_layout` applies to produce `content_bounds.width`, so a
    /// caller can derive the width BEFORE `prepare` runs (it needs the width to measure
    /// card heights before blocks are populated). `text_metrics` is passed explicitly
    /// rather than read from `self` so it does not depend on prepare-ordering.
    pub fn content_width_for_bounds(&self, bounds: Rect, text_metrics: TextMetrics) -> f32 {
        content_bounds_for_viewport(bounds, text_metrics).width
    }

    pub fn content_height_for_bounds(&self, bounds: Rect, text_metrics: TextMetrics) -> f32 {
        content_bounds_for_viewport(bounds, text_metrics).height
    }

    /// Top edge band occupied by the sticky file header, if any, so overlays can avoid
    /// painting/clicking over it.
    pub fn sticky_header_rect(&self) -> Option<Rect> {
        self.sticky_header_hit.as_ref().map(|(rect, _)| *rect)
    }

    pub fn overlay_clip_rect(&self, viewport_bounds: Rect) -> Rect {
        match self.sticky_header_rect() {
            Some(header) => Rect {
                x: viewport_bounds.x,
                y: header.bottom(),
                width: viewport_bounds.width,
                height: (viewport_bounds.bottom() - header.bottom()).max(0.0),
            },
            None => viewport_bounds,
        }
    }

    /// Visible review block rows resolved into the same overlay rects used for both
    /// rendering and clipping their element hit regions.
    pub fn visible_review_block_overlays(
        &self,
        overlay_width: f32,
        clip: Rect,
    ) -> Vec<ResolvedEditorOverlay> {
        let mut out = Vec::new();
        for row in &self.rows {
            if !row.is_block() {
                continue;
            }
            let rect = self.row_rect_for(row);
            if !self.row_in_viewport(&rect) {
                continue;
            }
            let block_index = row.block_index as usize;
            let Some(block) = self.blocks.get(block_index) else {
                continue;
            };
            let kind = if block.is_composer() {
                EditorOverlayKind::ReviewComposerBlock { block_index }
            } else if block.review_card().is_some() {
                EditorOverlayKind::ReviewThreadBlock { block_index }
            } else {
                continue;
            };
            // Start the card/composer at the code column so the line-number gutter
            // stays visible beside it (like GitHub), rather than covering it.
            let code_x = if self.layout.split_mode {
                self.layout.left_text_rect.x
            } else {
                self.layout.unified_text_rect.x
            };
            let overlay_rect = Rect {
                x: code_x.max(rect.x),
                y: rect.y,
                width: overlay_width,
                height: rect.height,
            };
            if let Some(overlay) = ResolvedEditorOverlay::new(kind, overlay_rect, clip) {
                out.push(overlay);
            }
        }
        out
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

    fn rebuild_navigation_positions(&mut self, state: &mut EditorState) {
        if !self.nav_positions_valid {
            let mut hunk_positions = Vec::new();
            let mut file_positions = Vec::new();
            for row in &self.rows {
                if row.kind == RenderRowKind::HunkSeparator as u8 {
                    hunk_positions.push(row.y_px);
                } else if row.kind == RenderRowKind::FileHeader as u8 {
                    file_positions.push(row.y_px);
                }
            }
            self.nav_hunk_positions = Arc::new(hunk_positions);
            self.nav_file_positions = Arc::new(file_positions);
            // Search Y positions are derived from row geometry too.
            self.nav_search_matches = None;
            self.nav_positions_valid = true;
        }
        state.hunk_positions = Arc::clone(&self.nav_hunk_positions);
        state.file_positions = Arc::clone(&self.nav_file_positions);

        if state.search.open && !state.search.matches.is_empty() {
            let cached = self
                .nav_search_matches
                .as_ref()
                .is_some_and(|matches| Arc::ptr_eq(matches, &state.search.matches));
            if !cached {
                let mut y_positions = Vec::with_capacity(state.search.matches.len());
                for m in state.search.matches.iter() {
                    let y = self
                        .rows
                        .iter()
                        .find(|r| !r.is_block() && r.line_index == m.line_index)
                        .map(|r| r.y_px)
                        .unwrap_or(0);
                    y_positions.push(y);
                }
                self.nav_search_y_positions = Arc::new(y_positions);
                self.nav_search_matches = Some(Arc::clone(&state.search.matches));
            }
            state.search_match_y_positions = Arc::clone(&self.nav_search_y_positions);
        } else {
            self.nav_search_matches = None;
            if !state.search_match_y_positions.is_empty() {
                state.search_match_y_positions = Arc::default();
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

    pub(super) fn row_rect_for(&self, display_row: &DisplayRow) -> Rect {
        Rect {
            x: self.layout.content_bounds.x,
            y: self.layout.content_bounds.y + display_row.y_px as f32 - self.layout.scroll_top_px,
            width: self.layout.content_bounds.width,
            height: display_row.h_px as f32,
        }
    }

    pub(super) fn row_in_viewport(&self, row_rect: &Rect) -> bool {
        row_rect.bottom() >= self.layout.content_bounds.y
            && row_rect.y <= self.layout.content_bounds.bottom()
    }

    pub(super) fn render_cols_unified(&self) -> u16 {
        render_cols_for_width(
            self.config.wrap_enabled,
            self.config.wrap_column,
            self.config.char_width_px as f32,
            self.layout.unified_text_rect.width,
        )
    }

    pub(super) fn render_cols_split(&self) -> u16 {
        render_cols_for_width(
            self.config.wrap_enabled,
            self.config.wrap_column,
            self.config.char_width_px as f32,
            self.layout.left_text_rect.width,
        )
    }

    pub(super) fn visible_segment_range(&self, block_y: f32, segment_count: u16) -> Range<u16> {
        visible_segment_range_for_block(
            block_y,
            segment_count.max(1),
            self.layout.line_height,
            self.layout.content_bounds.y,
            self.layout.content_bounds.bottom(),
        )
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
    let column_gap = scaled(BASE_COLUMN_GAP, s);
    let gutter_padding = scaled(BASE_GUTTER_PADDING, s);
    let scrollbar_width = scaled(BASE_SCROLLBAR_WIDTH, s);
    let scrollbar_margin = scaled(BASE_SCROLLBAR_MARGIN, s);

    let content_bounds = content_bounds_for_viewport(bounds, text_metrics);
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

pub(super) fn render_cols_for_width(
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

pub(super) fn visible_segment_range_for_block(
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
