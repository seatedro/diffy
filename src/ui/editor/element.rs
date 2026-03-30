use std::{collections::HashMap, ops::Range, sync::Arc};

use crate::core::compare::LayoutMode;
use crate::core::text::SyntaxTokenKind;
use crate::render::{
    FontKind, FontWeight, Rect, RectPrimitive, RichTextPrimitive, RichTextSpan,
    RoundedRectPrimitive, Scene, TextMetrics, TextPrimitive,
};
use crate::ui::theme::{Color, Theme};

use super::display_layout::{
    DisplayLayoutConfig, DisplayLayoutMetrics, DisplayLayoutSummary, compute_gutter_digits,
    rebuild_display_rows,
};
use super::render_doc::{
    ByteRange, DisplayRow, INVALID_U32, RenderDoc, RenderLine, RenderRowKind, RunRange,
    STYLE_FLAG_CHANGE, StyleRun,
};
use super::state::EditorState;
use super::strip_layout::{StripLayout, build_strip_layouts, visible_strip_range};

const BASE_VIEWPORT_PADDING: f32 = 14.0;
const BASE_COLUMN_GAP: f32 = 18.0;
const BASE_GUTTER_PADDING: f32 = 8.0;
const BASE_SCROLLBAR_WIDTH: f32 = 8.0;
const BASE_SCROLLBAR_MARGIN: f32 = 6.0;
const BASE_FILE_HEADER_EXTRA: f32 = 10.0;
const BASE_HUNK_EXTRA: f32 = 6.0;
const BASE_SCROLLBAR_THUMB_MIN: f32 = 32.0;
const BASE_MONO_FONT_SIZE: f32 = 13.0;
const STRIP_TARGET_HEIGHT_PX: u32 = 480;
const STRIP_OVERSCAN: usize = 1;

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
    pub gutter_padding: f32,
    pub column_gap: f32,
    pub scroll_top_px: f32,
    pub visible_row_range: VisibleRange,
    pub highlighted_row: Option<usize>,
    pub scrollbar: Option<ScrollbarLayout>,
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

#[derive(Debug, Clone)]
pub struct EditorElement {
    layout_key: Option<EditorLayoutKey>,
    layout: EditorLayout,
    config: DisplayLayoutConfig,
    metrics: DisplayLayoutMetrics,
    summary: DisplayLayoutSummary,
    rows: Vec<DisplayRow>,
    strips: Vec<StripLayout>,
    theme_cache_key: Option<EditorThemeKey>,
    wrapped_text_cache: HashMap<WrappedTextCacheKey, Arc<[RichTextSpan]>>,
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
            theme_cache_key: None,
            wrapped_text_cache: HashMap::new(),
            gutter_text_cache: HashMap::new(),
            text_metrics: TextMetrics::default(),
        }
    }
}

impl EditorElement {
    pub fn scrollbar_rect(&self) -> Rect {
        self.layout
            .scrollbar
            .map(|sb| sb.track)
            .unwrap_or_default()
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
            compute_scrollbar_layout(&self.layout, state);

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

    fn rebuild_rows(
        &mut self,
        doc: &RenderDoc,
        state: &EditorState,
        text_metrics: TextMetrics,
    ) {
        let s = editor_scale(text_metrics);
        self.metrics = DisplayLayoutMetrics {
            body_row_height_px: text_metrics.mono_line_height_px.round().max(1.0) as u16,
            file_header_height_px: (text_metrics.mono_line_height_px
                + scaled(BASE_FILE_HEADER_EXTRA, s))
            .round()
            .max(1.0) as u16,
            hunk_height_px: (text_metrics.mono_line_height_px + scaled(BASE_HUNK_EXTRA, s))
                .round()
                .max(1.0) as u16,
        };
        self.config = DisplayLayoutConfig {
            split_mode: state.layout == LayoutMode::Split,
            wrap_enabled: state.wrap_enabled,
            wrap_column: state.wrap_column,
            char_width_px: text_metrics.mono_char_width_px as f64,
            unified_text_width_px: self.layout.unified_text_rect.width as f64,
            split_text_width_px: self.layout.left_text_rect.width as f64,
        };
        self.summary = rebuild_display_rows(doc, self.config, self.metrics, &mut self.rows);
        build_strip_layouts(&self.rows, STRIP_TARGET_HEIGHT_PX, &mut self.strips);
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
            EditorDocument::Empty => {
                self.paint_placeholder(scene, theme, "No file selected",
                    "Choose a file from the list to render the native viewport.");
            }
            EditorDocument::Binary { path } => {
                self.paint_placeholder(scene, theme, path,
                    "Binary file. The native viewport only renders text diffs in this phase.");
            }
            EditorDocument::Text { path, doc, .. } => {
                self.sync_theme_cache(theme);
                scene.clip(self.layout.content_bounds);

                self.paint_gutter_backgrounds(scene, theme);
                self.paint_row_backgrounds(scene, theme, doc);
                self.paint_line_highlights(scene, theme);
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
            rect: Rect { x: inset.x, y: inset.y + inset.height * 0.35, width: inset.width, height: fs + 10.0 },
            text: title.into(),
            color: theme.colors.text_strong,
            font_size: fs + 4.0,
            font_kind: FontKind::Ui,
            font_weight: FontWeight::Normal,
        });
        scene.text(TextPrimitive {
            rect: Rect { x: inset.x, y: inset.y + inset.height * 0.35 + fs + 16.0, width: inset.width, height: fs + 4.0 },
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

    // -- Phase 1: Gutter backgrounds (full viewport height) --

    fn paint_gutter_backgrounds(&self, scene: &mut Scene, theme: &Theme) {
        if self.layout.split_mode {
            scene.rect(RectPrimitive { rect: self.layout.left_gutter_rect, color: theme.colors.gutter_bg });
            scene.rect(RectPrimitive { rect: self.layout.right_gutter_rect, color: theme.colors.gutter_bg });
        } else {
            scene.rect(RectPrimitive { rect: self.layout.unified_gutter_rect, color: theme.colors.gutter_bg });
        }
    }

    // -- Phase 2: Row backgrounds (diff colors) --

    fn paint_row_backgrounds(&self, scene: &mut Scene, theme: &Theme, doc: &RenderDoc) {
        for row_index in self.layout.visible_row_range.iter() {
            let Some(display_row) = self.rows.get(row_index).copied() else { continue };
            let Some(line) = doc.lines.get(display_row.line_index as usize) else { continue };
            let rr = self.row_rect_for(&display_row);
            if !self.row_in_viewport(&rr) { continue; }
            paint_row_background(scene, theme, rr, line.row_kind());
        }
    }

    // -- Phase 3: Line highlights (hover) --

    fn paint_line_highlights(&self, scene: &mut Scene, theme: &Theme) {
        let Some(hovered) = self.layout.highlighted_row else { return };
        let Some(display_row) = self.rows.get(hovered).copied() else { return };
        let rr = self.row_rect_for(&display_row);
        if !self.row_in_viewport(&rr) { return; }

        let text_highlight = if self.layout.split_mode {
            Rect { x: self.layout.left_text_rect.x, y: rr.y,
                width: self.layout.content_bounds.right() - self.layout.left_text_rect.x, height: rr.height }
        } else {
            Rect { x: self.layout.unified_text_rect.x, y: rr.y,
                width: self.layout.unified_text_rect.width, height: rr.height }
        };
        scene.rect(RectPrimitive { rect: text_highlight, color: theme.colors.hover_overlay });

        let gutter_highlight = if self.layout.split_mode {
            Rect { x: self.layout.left_gutter_rect.x, y: rr.y,
                width: self.layout.left_gutter_rect.width + self.layout.right_gutter_rect.width + self.layout.column_gap,
                height: rr.height }
        } else {
            Rect { x: self.layout.unified_gutter_rect.x, y: rr.y,
                width: self.layout.unified_gutter_rect.width, height: rr.height }
        };
        scene.rect(RectPrimitive { rect: gutter_highlight, color: theme.colors.hover_overlay });
    }

    // -- Phase 4: Gutter decorations (separator lines) --

    fn paint_gutter_decorations(&self, scene: &mut Scene, theme: &Theme) {
        let cb = self.layout.content_bounds;
        if self.layout.split_mode {
            scene.rect(RectPrimitive {
                rect: Rect { x: self.layout.left_gutter_rect.right() - 1.0, y: cb.y, width: 1.0, height: cb.height },
                color: theme.colors.border_soft,
            });
            scene.rect(RectPrimitive {
                rect: Rect { x: self.layout.right_gutter_rect.right() - 1.0, y: cb.y, width: 1.0, height: cb.height },
                color: theme.colors.border_soft,
            });
        } else {
            scene.rect(RectPrimitive {
                rect: Rect { x: self.layout.unified_gutter_rect.right() - 1.0, y: cb.y, width: 1.0, height: cb.height },
                color: theme.colors.border_soft,
            });
        }
    }

    // -- Phase 5: Gutter text (line numbers) --

    fn paint_gutter_text(&mut self, scene: &mut Scene, theme: &Theme, doc: &RenderDoc) {
        let font_size = self.layout.font_size;
        let line_height = self.layout.line_height;

        for row_index in self.layout.visible_row_range.iter() {
            let Some(display_row) = self.rows.get(row_index).copied() else { continue };
            let Some(line) = doc.lines.get(display_row.line_index as usize).copied() else { continue };
            let rr = self.row_rect_for(&display_row);
            if !self.row_in_viewport(&rr) { continue; }

            match line.row_kind() {
                RenderRowKind::FileHeader | RenderRowKind::HunkSeparator => {}
                _ if self.layout.split_mode => {
                    scene.text(TextPrimitive {
                        rect: Rect { x: self.layout.left_gutter_rect.x + self.layout.gutter_padding, y: rr.y,
                            width: self.layout.left_gutter_rect.width - self.layout.gutter_padding * 2.0, height: line_height },
                        text: self.cached_gutter_text(GutterTextCacheKey {
                            old_line_no: line.old_line_no, new_line_no: INVALID_U32,
                            digits: self.summary.gutter_digits, kind: GutterTextKind::SplitLeft }),
                        color: theme.colors.gutter_text, font_size, font_kind: FontKind::Mono, font_weight: FontWeight::Normal,
                    });
                    scene.text(TextPrimitive {
                        rect: Rect { x: self.layout.right_gutter_rect.x + self.layout.gutter_padding, y: rr.y,
                            width: self.layout.right_gutter_rect.width - self.layout.gutter_padding * 2.0, height: line_height },
                        text: self.cached_gutter_text(GutterTextCacheKey {
                            old_line_no: INVALID_U32, new_line_no: line.new_line_no,
                            digits: self.summary.gutter_digits, kind: GutterTextKind::SplitRight }),
                        color: theme.colors.gutter_text, font_size, font_kind: FontKind::Mono, font_weight: FontWeight::Normal,
                    });
                }
                RenderRowKind::Modified if line.left_text.is_valid() && line.right_text.is_valid() => {
                    scene.text(TextPrimitive {
                        rect: Rect { x: self.layout.unified_gutter_rect.x + self.layout.gutter_padding, y: rr.y,
                            width: self.layout.unified_gutter_rect.width - self.layout.gutter_padding * 2.0, height: line_height },
                        text: self.cached_gutter_text(GutterTextCacheKey {
                            old_line_no: line.old_line_no, new_line_no: INVALID_U32,
                            digits: self.summary.gutter_digits, kind: GutterTextKind::UnifiedOldOnly }),
                        color: theme.colors.gutter_text, font_size, font_kind: FontKind::Mono, font_weight: FontWeight::Normal,
                    });
                    let added_y = rr.y + display_row.wrap_left.max(1) as f32 * line_height;
                    scene.text(TextPrimitive {
                        rect: Rect { x: self.layout.unified_gutter_rect.x + self.layout.gutter_padding, y: added_y,
                            width: self.layout.unified_gutter_rect.width - self.layout.gutter_padding * 2.0, height: line_height },
                        text: self.cached_gutter_text(GutterTextCacheKey {
                            old_line_no: INVALID_U32, new_line_no: line.new_line_no,
                            digits: self.summary.gutter_digits, kind: GutterTextKind::UnifiedNewOnly }),
                        color: theme.colors.gutter_text, font_size, font_kind: FontKind::Mono, font_weight: FontWeight::Normal,
                    });
                }
                _ => {
                    scene.text(TextPrimitive {
                        rect: Rect { x: self.layout.unified_gutter_rect.x + self.layout.gutter_padding, y: rr.y,
                            width: self.layout.unified_gutter_rect.width - self.layout.gutter_padding * 2.0, height: line_height },
                        text: self.cached_gutter_text(GutterTextCacheKey {
                            old_line_no: line.old_line_no, new_line_no: line.new_line_no,
                            digits: self.summary.gutter_digits, kind: GutterTextKind::Unified }),
                        color: theme.colors.gutter_text, font_size, font_kind: FontKind::Mono, font_weight: FontWeight::Normal,
                    });
                }
            }
        }
    }

    // -- Phase 6: Body text (code content with syntax highlighting) --

    fn paint_body_text(&mut self, scene: &mut Scene, theme: &Theme, path: &str, doc: &RenderDoc) {
        let font_size = self.layout.font_size;
        let line_height = self.layout.line_height;

        for row_index in self.layout.visible_row_range.iter() {
            let Some(display_row) = self.rows.get(row_index).copied() else { continue };
            let Some(line) = doc.lines.get(display_row.line_index as usize).copied() else { continue };
            let rr = self.row_rect_for(&display_row);
            if !self.row_in_viewport(&rr) { continue; }

            match line.row_kind() {
                RenderRowKind::FileHeader => {
                    scene.text(TextPrimitive {
                        rect: Rect { x: self.text_origin_x(), y: rr.y, width: self.text_width(), height: rr.height },
                        text: path.into(),
                        color: theme.colors.text_strong,
                        font_size: font_size + 1.0,
                        font_kind: FontKind::Ui,
                        font_weight: FontWeight::Medium,
                    });
                }
                RenderRowKind::HunkSeparator => {
                    scene.text(TextPrimitive {
                        rect: Rect { x: self.text_origin_x(), y: rr.y, width: self.text_width(), height: rr.height },
                        text: doc.line_text(line.left_text).into(),
                        color: theme.colors.text_muted,
                        font_size,
                        font_kind: FontKind::Mono,
                        font_weight: FontWeight::Normal,
                    });
                }
                _ if self.layout.split_mode => {
                    self.paint_split_body_spans(scene, theme, rr, &line, &display_row, doc, font_size, line_height);
                }
                RenderRowKind::Modified if line.left_text.is_valid() && line.right_text.is_valid() => {
                    self.paint_unified_modified_spans(scene, theme, rr, &line, &display_row, doc, font_size, line_height);
                }
                _ => {
                    if let Some((text_range, runs, tone)) = unified_body_side(&line) {
                        if let Some(spans) = self.cached_wrapped_rich_text(doc, text_range, runs, 0, self.wrap_cols_unified(), tone, theme) {
                            scene.rich_text(RichTextPrimitive {
                                rect: Rect { x: self.layout.unified_text_rect.x, y: rr.y,
                                    width: self.layout.unified_text_rect.width, height: line_height },
                                spans, default_color: tone.default_text(theme), font_size,
                                font_kind: FontKind::Mono, font_weight: FontWeight::Normal,
                            });
                        }
                    }
                }
            }
        }
    }

    // -- Phase 7: Scrollbar --

    fn paint_scrollbar(&self, scene: &mut Scene, theme: &Theme) {
        let Some(sb) = self.layout.scrollbar else { return };
        scene.rounded_rect(RoundedRectPrimitive::uniform(sb.track, 4.0, Color::rgba(128, 128, 128, 10)));
        scene.rounded_rect(RoundedRectPrimitive::uniform(sb.thumb, 3.0, theme.colors.scrollbar_thumb));
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
        for seg in 0..display_row.wrap_left.max(1) {
            let rect = Rect { x: self.layout.left_text_rect.x,
                y: rr.y + seg as f32 * line_height, width: self.layout.left_text_rect.width, height: line_height };
            if let Some(spans) = self.cached_wrapped_rich_text(
                doc, line.left_text, line.left_runs, seg, self.wrap_cols_split(), tone_for_left_side(line.row_kind()), theme) {
                scene.rich_text(RichTextPrimitive { rect, spans,
                    default_color: tone_for_left_side(line.row_kind()).default_text(theme),
                    font_size, font_kind: FontKind::Mono, font_weight: FontWeight::Normal });
            }
        }
        for seg in 0..display_row.wrap_right.max(1) {
            let rect = Rect { x: self.layout.right_text_rect.x,
                y: rr.y + seg as f32 * line_height, width: self.layout.right_text_rect.width, height: line_height };
            if let Some(spans) = self.cached_wrapped_rich_text(
                doc, line.right_text, line.right_runs, seg, self.wrap_cols_split(), tone_for_right_side(line.row_kind()), theme) {
                scene.rich_text(RichTextPrimitive { rect, spans,
                    default_color: tone_for_right_side(line.row_kind()).default_text(theme),
                    font_size, font_kind: FontKind::Mono, font_weight: FontWeight::Normal });
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
        for seg in 0..display_row.wrap_left.max(1) {
            let y = rr.y + seg as f32 * line_height;
            let rect = Rect { x: self.layout.unified_text_rect.x, y, width: self.layout.unified_text_rect.width, height: line_height };
            if let Some(spans) = self.cached_wrapped_rich_text(
                doc, line.left_text, line.left_runs, seg, self.wrap_cols_unified(), RowTone::Removed, theme) {
                scene.rich_text(RichTextPrimitive { rect, spans, default_color: theme.colors.line_del_text,
                    font_size, font_kind: FontKind::Mono, font_weight: FontWeight::Normal });
            }
        }
        for seg in 0..display_row.wrap_right.max(1) {
            let y = rr.y + display_row.wrap_left.max(1) as f32 * line_height + seg as f32 * line_height;
            let rect = Rect { x: self.layout.unified_text_rect.x, y, width: self.layout.unified_text_rect.width, height: line_height };
            if let Some(spans) = self.cached_wrapped_rich_text(
                doc, line.right_text, line.right_runs, seg, self.wrap_cols_unified(), RowTone::Added, theme) {
                scene.rich_text(RichTextPrimitive { rect, spans, default_color: theme.colors.line_add_text,
                    font_size, font_kind: FontKind::Mono, font_weight: FontWeight::Normal });
            }
        }
    }

    fn clear_document_caches(&mut self) {
        self.wrapped_text_cache.clear();
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

        let spans = build_wrapped_rich_text(
            doc,
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


    fn wrap_cols_unified(&self) -> u16 {
        wrap_cols_for_width(
            self.config.wrap_enabled,
            self.config.wrap_column,
            self.config.char_width_px as f32,
            self.layout.unified_text_rect.width,
        )
    }

    fn wrap_cols_split(&self) -> u16 {
        wrap_cols_for_width(
            self.config.wrap_enabled,
            self.config.wrap_column,
            self.config.char_width_px as f32,
            self.layout.left_text_rect.width,
        )
    }

    fn text_origin_x(&self) -> f32 {
        if self.layout.split_mode {
            self.layout.left_text_rect.x
        } else {
            self.layout.unified_text_rect.x
        }
    }

    fn text_width(&self) -> f32 {
        if self.layout.split_mode {
            self.layout.left_text_rect.width
        } else {
            self.layout.unified_text_rect.width
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RowTone {
    Neutral,
    Added,
    Removed,
}

impl RowTone {
    fn default_text(self, theme: &Theme) -> Color {
        match self {
            Self::Neutral => theme.colors.text_strong,
            Self::Added => theme.colors.line_add_text,
            Self::Removed => theme.colors.line_del_text,
        }
    }
}

fn compute_scrollbar_layout(
    layout: &EditorLayout,
    state: &EditorState,
) -> Option<ScrollbarLayout> {
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
        let left_text_rect = Rect {
            x: left_gutter_rect.right(),
            y: content_bounds.y,
            width: col_width,
            height: content_bounds.height,
        };
        let right_gutter_rect = Rect {
            x: left_text_rect.right() + column_gap,
            y: content_bounds.y,
            width: gutter_width,
            height: content_bounds.height,
        };
        let right_text_rect = Rect {
            x: right_gutter_rect.right(),
            y: content_bounds.y,
            width: (content_bounds.right()
                - scrollbar_width
                - scrollbar_margin
                - right_gutter_rect.right())
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
        let unified_text_rect = Rect {
            x: unified_gutter_rect.right(),
            y: content_bounds.y,
            width: (usable_width - unified_gutter_width).max(60.0),
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

fn paint_row_background(scene: &mut Scene, theme: &Theme, row_rect: Rect, kind: RenderRowKind) {
    let color = match kind {
        RenderRowKind::FileHeader => theme.colors.file_header_bg,
        RenderRowKind::HunkSeparator => theme.colors.hunk_header_bg,
        RenderRowKind::Context => theme.colors.canvas,
        RenderRowKind::Added => theme.colors.line_add,
        RenderRowKind::Removed => theme.colors.line_del,
        RenderRowKind::Modified => theme.colors.line_modified,
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
        RenderRowKind::Removed | RenderRowKind::Modified => RowTone::Removed,
        _ => RowTone::Neutral,
    }
}

fn tone_for_right_side(kind: RenderRowKind) -> RowTone {
    match kind {
        RenderRowKind::Added | RenderRowKind::Modified => RowTone::Added,
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

fn build_wrapped_rich_text(
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
    let full_text = doc.line_text(text_range);
    let spans: Arc<[RichTextSpan]> = if full_text.is_empty() {
        Arc::from(Vec::new())
    } else {
        let (start, end) = wrapped_byte_slice(full_text, wrap_cols, segment_index)?;
        Arc::from(build_segment_spans(
            full_text,
            start,
            end,
            doc.line_runs(runs),
            tone,
            theme,
        ))
    };
    Some(spans)
}

fn wrapped_byte_slice(text: &str, wrap_cols: u16, segment_index: u16) -> Option<(usize, usize)> {
    if wrap_cols == u16::MAX {
        return (segment_index == 0).then_some((0, text.len()));
    }

    let mut breaks = vec![0_usize];
    let mut count = 0_u16;
    for (byte_index, _) in text.char_indices() {
        if byte_index == 0 {
            continue;
        }
        count = count.saturating_add(1);
        if count >= wrap_cols.max(1) {
            breaks.push(byte_index);
            count = 0;
        }
    }
    breaks.push(text.len());

    let segment_index = segment_index as usize;
    let start = *breaks.get(segment_index)?;
    let end = *breaks.get(segment_index + 1)?;
    Some((start, end))
}

fn build_segment_spans(
    full_text: &str,
    segment_start: usize,
    segment_end: usize,
    runs: &[StyleRun],
    tone: RowTone,
    theme: &Theme,
) -> Vec<RichTextSpan> {
    let mut spans = Vec::new();
    let mut cursor = segment_start;

    for run in runs {
        let run_start = run.byte_start as usize;
        let run_end = run_start.saturating_add(run.byte_len as usize);
        let start = run_start.max(segment_start);
        let end = run_end.min(segment_end);
        if end <= start {
            continue;
        }

        if cursor < start {
            spans.push(RichTextSpan {
                text: full_text[cursor..start].into(),
                color: tone.default_text(theme),
            });
        }

        spans.push(RichTextSpan {
            text: full_text[start..end].into(),
            color: style_run_color(*run, tone, theme),
        });
        cursor = end;
    }

    if cursor < segment_end {
        spans.push(RichTextSpan {
            text: full_text[cursor..segment_end].into(),
            color: tone.default_text(theme),
        });
    }

    if spans.is_empty() {
        spans.push(RichTextSpan {
            text: full_text[segment_start..segment_end].into(),
            color: tone.default_text(theme),
        });
    }

    spans
}

fn style_run_color(run: StyleRun, tone: RowTone, theme: &Theme) -> Color {
    let is_changed = run.flags & STYLE_FLAG_CHANGE != 0;
    if is_changed {
        return match tone {
            RowTone::Neutral => theme.colors.accent,
            RowTone::Added => theme.colors.line_add_text,
            RowTone::Removed => theme.colors.line_del_text,
        };
    }

    match syntax_kind_from_style_id(run.style_id) {
        SyntaxTokenKind::Keyword | SyntaxTokenKind::Builtin => theme.colors.accent,
        SyntaxTokenKind::String => match tone {
            RowTone::Added => theme.colors.line_add_text,
            RowTone::Removed => theme.colors.line_del_text,
            RowTone::Neutral => Color::rgba(0xcb, 0xe4, 0xa7, 0xff),
        },
        SyntaxTokenKind::Comment | SyntaxTokenKind::Label | SyntaxTokenKind::Preprocessor => {
            theme.colors.text_muted
        }
        SyntaxTokenKind::Number | SyntaxTokenKind::Constant => Color::rgba(0xf5, 0xc2, 0x8b, 0xff),
        SyntaxTokenKind::Type | SyntaxTokenKind::Namespace | SyntaxTokenKind::Tag => {
            Color::rgba(0x8f, 0xd3, 0xd7, 0xff)
        }
        SyntaxTokenKind::Function | SyntaxTokenKind::Attribute | SyntaxTokenKind::Property => {
            Color::rgba(0xf8, 0xe1, 0x9a, 0xff)
        }
        SyntaxTokenKind::Operator | SyntaxTokenKind::Punctuation => theme.colors.text_muted,
        SyntaxTokenKind::Variable | SyntaxTokenKind::Normal => tone.default_text(theme),
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
        EditorElement, EditorDocument, build_wrapped_rich_text, wrapped_byte_slice,
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
        assert_eq!(wrapped_byte_slice("abcdefghij", 4, 0), Some((0, 4)));
        assert_eq!(wrapped_byte_slice("abcdefghij", 4, 1), Some((4, 8)));
        assert_eq!(wrapped_byte_slice("abcdefghij", 4, 2), Some((8, 10)));
        assert_eq!(wrapped_byte_slice("abcdefghij", 4, 3), None);
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

        let spans = build_wrapped_rich_text(
            &doc,
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
        assert!(state.visible_row_end.expect("visible end") >= 3);
        let body = runtime.body_bounds();
        assert_eq!(
            runtime.hit_test_row(&state, body.x + 20.0, body.y + 5.0),
            Some(0)
        );
    }
}
