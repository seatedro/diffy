use std::ops::Range;

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
use super::state::DiffViewportState;
use super::strip_layout::{StripLayout, build_strip_layouts, visible_strip_range};

const VIEWPORT_PADDING_PX: f32 = 14.0;
const COLUMN_GAP_PX: f32 = 18.0;
const GUTTER_PADDING_PX: f32 = 8.0;
const SCROLLBAR_WIDTH_PX: f32 = 8.0;
const SCROLLBAR_MARGIN_PX: f32 = 6.0;
const STRIP_TARGET_HEIGHT_PX: u32 = 480;
const STRIP_OVERSCAN: usize = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ViewportLayout {
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
    pub scrollbar_rect: Rect,
}

#[derive(Debug, Clone, Copy)]
pub enum ViewportDocument<'a> {
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
struct ViewportLayoutKey {
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

#[derive(Debug, Clone)]
pub struct DiffViewportRuntime {
    layout_key: Option<ViewportLayoutKey>,
    layout: ViewportLayout,
    config: DisplayLayoutConfig,
    metrics: DisplayLayoutMetrics,
    summary: DisplayLayoutSummary,
    rows: Vec<DisplayRow>,
    strips: Vec<StripLayout>,
    paint_row_range: Range<usize>,
}

impl Default for DiffViewportRuntime {
    fn default() -> Self {
        Self {
            layout_key: None,
            layout: ViewportLayout::default(),
            config: DisplayLayoutConfig::default(),
            metrics: DisplayLayoutMetrics::default(),
            summary: DisplayLayoutSummary::default(),
            rows: Vec::new(),
            strips: Vec::new(),
            paint_row_range: 0..0,
        }
    }
}

impl DiffViewportRuntime {
    pub fn scrollbar_rect(&self) -> Rect {
        self.layout.scrollbar_rect
    }

    pub fn scroll_line_height_px(&self) -> f32 {
        let line_height = self.metrics.body_row_height_px as f32;
        if line_height > 0.0 { line_height } else { 20.0 }
    }

    pub fn prepare(
        &mut self,
        state: &mut DiffViewportState,
        document: ViewportDocument<'_>,
        bounds: Rect,
        text_metrics: TextMetrics,
    ) -> ViewportLayout {
        let gutter_digits = match document {
            ViewportDocument::Text { doc, .. } => compute_gutter_digits(doc),
            _ => 3,
        };
        self.layout = build_layout(bounds, state.layout, gutter_digits, text_metrics);
        state.viewport_width_px = self.layout.content_bounds.width.max(0.0).round() as u32;
        state.viewport_height_px = self.layout.content_bounds.height.max(0.0).round() as u32;

        match document {
            ViewportDocument::Text {
                compare_generation,
                file_index,
                doc,
                ..
            } => {
                let key = ViewportLayoutKey {
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
                    self.layout_key = Some(key);
                }

                state.content_height_px = self.summary.content_height_px;
                state.clamp_scroll();
                self.update_visible_ranges(state);
            }
            _ => {
                self.layout_key = None;
                self.rows.clear();
                self.strips.clear();
                self.paint_row_range = 0..0;
                state.clear_document();
            }
        }

        self.layout
    }

    pub fn body_bounds(&self) -> Rect {
        self.layout.content_bounds
    }

    pub fn hit_test_row(&self, state: &DiffViewportState, x: f32, y: f32) -> Option<usize> {
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
        state: &DiffViewportState,
        text_metrics: TextMetrics,
    ) {
        self.metrics = DisplayLayoutMetrics {
            body_row_height_px: text_metrics.mono_line_height_px.round().max(1.0) as u16,
            file_header_height_px: (text_metrics.mono_line_height_px + 10.0).round().max(1.0)
                as u16,
            hunk_height_px: (text_metrics.mono_line_height_px + 6.0).round().max(1.0) as u16,
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

    fn update_visible_ranges(&mut self, state: &mut DiffViewportState) {
        let viewport_top_px = state.scroll_top_px;
        let viewport_height_px = state.viewport_height_px.max(1);
        let strip_range = visible_strip_range(
            &self.strips,
            viewport_top_px,
            viewport_height_px,
            STRIP_OVERSCAN,
        );
        self.paint_row_range = if strip_range.is_empty() {
            0..0
        } else {
            let first = self.strips[strip_range.start].row_start;
            let last = self.strips[strip_range.end - 1].row_end;
            first..last
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
        &self,
        scene: &mut Scene,
        theme: &Theme,
        state: &DiffViewportState,
        document: ViewportDocument<'_>,
    ) {
        scene.rect(RectPrimitive {
            rect: self.layout.content_bounds,
            color: theme.colors.canvas,
        });

        match document {
            ViewportDocument::Empty => {
                self.paint_placeholder(
                    scene,
                    theme,
                    "No file selected",
                    "Choose a file from the list to render the native viewport.",
                );
            }
            ViewportDocument::Binary { path } => {
                self.paint_placeholder(
                    scene,
                    theme,
                    path,
                    "Binary file. The native viewport only renders text diffs in this phase.",
                );
            }
            ViewportDocument::Text { path, doc, .. } => {
                self.paint_rows(scene, theme, state, path, doc);
            }
        }
    }

    fn paint_placeholder(&self, scene: &mut Scene, theme: &Theme, title: &str, message: &str) {
        let inset = self.layout.content_bounds.inset(24.0);
        scene.text(TextPrimitive {
            rect: Rect {
                x: inset.x,
                y: inset.y + inset.height * 0.35,
                width: inset.width,
                height: 28.0,
            },
            text: title.to_owned(),
            color: theme.colors.text_strong,
            font_size: 18.0,
            font_kind: FontKind::Ui,
            font_weight: FontWeight::Normal,
        });
        scene.text(TextPrimitive {
            rect: Rect {
                x: inset.x,
                y: inset.y + inset.height * 0.35 + 32.0,
                width: inset.width,
                height: 22.0,
            },
            text: message.to_owned(),
            color: theme.colors.text_muted,
            font_size: 13.0,
            font_kind: FontKind::Ui,
            font_weight: FontWeight::Normal,
        });
    }

    fn paint_rows(
        &self,
        scene: &mut Scene,
        theme: &Theme,
        state: &DiffViewportState,
        path: &str,
        doc: &RenderDoc,
    ) {
        scene.clip(self.layout.content_bounds);
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

        for row_index in self.paint_row_range.clone() {
            let Some(display_row) = self.rows.get(row_index) else {
                continue;
            };
            let Some(line) = doc.lines.get(display_row.line_index as usize) else {
                continue;
            };
            let row_rect = Rect {
                x: self.layout.content_bounds.x,
                y: self.layout.content_bounds.y + display_row.y_px as f32
                    - state.scroll_top_px as f32,
                width: self.layout.content_bounds.width,
                height: display_row.h_px as f32,
            };
            if row_rect.bottom() < self.layout.content_bounds.y
                || row_rect.y > self.layout.content_bounds.bottom()
            {
                continue;
            }

            paint_row_background(scene, theme, row_rect, line.row_kind());
            match line.row_kind() {
                RenderRowKind::FileHeader => {
                    scene.text(TextPrimitive {
                        rect: Rect {
                            x: self.text_origin_x(),
                            y: row_rect.y + 6.0,
                            width: self.text_width(),
                            height: row_rect.height - 8.0,
                        },
                        text: path.to_owned(),
                        color: theme.colors.text_strong,
                        font_size: 15.0,
                        font_kind: FontKind::Ui,
            font_weight: FontWeight::Normal,
                    });
                }
                RenderRowKind::HunkSeparator => {
                    scene.text(TextPrimitive {
                        rect: Rect {
                            x: self.text_origin_x(),
                            y: row_rect.y + 4.0,
                            width: self.text_width(),
                            height: row_rect.height - 6.0,
                        },
                        text: doc.line_text(line.left_text).to_owned(),
                        color: theme.colors.text_muted,
                        font_size: 13.0,
                        font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
                    });
                }
                _ if self.layout.split_mode => {
                    self.paint_split_body_row(scene, theme, row_rect, line, display_row, doc);
                }
                _ => {
                    self.paint_unified_body_row(scene, theme, row_rect, line, display_row, doc);
                }
            }

            if state.hovered_row == Some(row_index) {
                scene.rect(RectPrimitive {
                    rect: row_rect,
                    color: theme.colors.hover_overlay,
                });
            }
        }
        scene.pop_clip();

        self.paint_scrollbar(scene, theme, state);
    }

    fn paint_split_body_row(
        &self,
        scene: &mut Scene,
        theme: &Theme,
        row_rect: Rect,
        line: &RenderLine,
        display_row: &DisplayRow,
        doc: &RenderDoc,
    ) {
        let row_height = self.metrics.body_row_height_px as f32;
        scene.text(TextPrimitive {
            rect: Rect {
                x: self.layout.left_gutter_rect.x + GUTTER_PADDING_PX,
                y: row_rect.y + 3.0,
                width: self.layout.left_gutter_rect.width - GUTTER_PADDING_PX * 2.0,
                height: row_height,
            },
            text: format_line_number(line.old_line_no, self.summary.gutter_digits),
            color: theme.colors.gutter_text,
            font_size: 12.0,
            font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
        });
        scene.text(TextPrimitive {
            rect: Rect {
                x: self.layout.right_gutter_rect.x + GUTTER_PADDING_PX,
                y: row_rect.y + 3.0,
                width: self.layout.right_gutter_rect.width - GUTTER_PADDING_PX * 2.0,
                height: row_height,
            },
            text: format_line_number(line.new_line_no, self.summary.gutter_digits),
            color: theme.colors.gutter_text,
            font_size: 12.0,
            font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
        });

        for segment_index in 0..display_row.wrap_left.max(1) {
            let rect = Rect {
                x: self.layout.left_text_rect.x,
                y: row_rect.y + segment_index as f32 * row_height + 2.0,
                width: self.layout.left_text_rect.width,
                height: row_height,
            };
            if let Some(spans) = build_wrapped_rich_text(
                doc,
                line.left_text,
                line.left_runs,
                segment_index,
                self.wrap_cols_split(),
                tone_for_left_side(line.row_kind()),
                theme,
            ) {
                scene.rich_text(RichTextPrimitive {
                    rect,
                    spans,
                    default_color: tone_for_left_side(line.row_kind()).default_text(theme),
                    font_size: 13.0,
                    font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
                });
            }
        }

        for segment_index in 0..display_row.wrap_right.max(1) {
            let rect = Rect {
                x: self.layout.right_text_rect.x,
                y: row_rect.y + segment_index as f32 * row_height + 2.0,
                width: self.layout.right_text_rect.width,
                height: row_height,
            };
            if let Some(spans) = build_wrapped_rich_text(
                doc,
                line.right_text,
                line.right_runs,
                segment_index,
                self.wrap_cols_split(),
                tone_for_right_side(line.row_kind()),
                theme,
            ) {
                scene.rich_text(RichTextPrimitive {
                    rect,
                    spans,
                    default_color: tone_for_right_side(line.row_kind()).default_text(theme),
                    font_size: 13.0,
                    font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
                });
            }
        }
    }

    fn paint_unified_body_row(
        &self,
        scene: &mut Scene,
        theme: &Theme,
        row_rect: Rect,
        line: &RenderLine,
        display_row: &DisplayRow,
        doc: &RenderDoc,
    ) {
        let row_height = self.metrics.body_row_height_px as f32;
        if line.row_kind() == RenderRowKind::Modified
            && line.left_text.is_valid()
            && line.right_text.is_valid()
        {
            for segment_index in 0..display_row.wrap_left.max(1) {
                let segment_rect = Rect {
                    x: self.layout.unified_text_rect.x,
                    y: row_rect.y + segment_index as f32 * row_height + 2.0,
                    width: self.layout.unified_text_rect.width,
                    height: row_height,
                };
                scene.text(TextPrimitive {
                    rect: Rect {
                        x: self.layout.unified_gutter_rect.x + GUTTER_PADDING_PX,
                        y: segment_rect.y,
                        width: self.layout.unified_gutter_rect.width - GUTTER_PADDING_PX * 2.0,
                        height: row_height,
                    },
                    text: format!(
                        "{} {}",
                        format_line_number(line.old_line_no, self.summary.gutter_digits),
                        " ".repeat(self.summary.gutter_digits as usize)
                    ),
                    color: theme.colors.gutter_text,
                    font_size: 12.0,
                    font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
                });
                if let Some(spans) = build_wrapped_rich_text(
                    doc,
                    line.left_text,
                    line.left_runs,
                    segment_index,
                    self.wrap_cols_unified(),
                    RowTone::Removed,
                    theme,
                ) {
                    scene.rich_text(RichTextPrimitive {
                        rect: segment_rect,
                        spans,
                        default_color: theme.colors.line_del_text,
                        font_size: 13.0,
                        font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
                    });
                }
            }

            for segment_index in 0..display_row.wrap_right.max(1) {
                let y = row_rect.y
                    + display_row.wrap_left.max(1) as f32 * row_height
                    + segment_index as f32 * row_height
                    + 2.0;
                let segment_rect = Rect {
                    x: self.layout.unified_text_rect.x,
                    y,
                    width: self.layout.unified_text_rect.width,
                    height: row_height,
                };
                scene.text(TextPrimitive {
                    rect: Rect {
                        x: self.layout.unified_gutter_rect.x + GUTTER_PADDING_PX,
                        y,
                        width: self.layout.unified_gutter_rect.width - GUTTER_PADDING_PX * 2.0,
                        height: row_height,
                    },
                    text: format!(
                        "{} {}",
                        " ".repeat(self.summary.gutter_digits as usize),
                        format_line_number(line.new_line_no, self.summary.gutter_digits)
                    ),
                    color: theme.colors.gutter_text,
                    font_size: 12.0,
                    font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
                });
                if let Some(spans) = build_wrapped_rich_text(
                    doc,
                    line.right_text,
                    line.right_runs,
                    segment_index,
                    self.wrap_cols_unified(),
                    RowTone::Added,
                    theme,
                ) {
                    scene.rich_text(RichTextPrimitive {
                        rect: segment_rect,
                        spans,
                        default_color: theme.colors.line_add_text,
                        font_size: 13.0,
                        font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
                    });
                }
            }
            return;
        }

        scene.text(TextPrimitive {
            rect: Rect {
                x: self.layout.unified_gutter_rect.x + GUTTER_PADDING_PX,
                y: row_rect.y + 3.0,
                width: self.layout.unified_gutter_rect.width - GUTTER_PADDING_PX * 2.0,
                height: row_height,
            },
            text: format!(
                "{} {}",
                format_line_number(line.old_line_no, self.summary.gutter_digits),
                format_line_number(line.new_line_no, self.summary.gutter_digits)
            ),
            color: theme.colors.gutter_text,
            font_size: 12.0,
            font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
        });

        if let Some((text_range, runs, tone)) = unified_body_side(line) {
            if let Some(spans) = build_wrapped_rich_text(
                doc,
                text_range,
                runs,
                0,
                self.wrap_cols_unified(),
                tone,
                theme,
            ) {
                scene.rich_text(RichTextPrimitive {
                    rect: Rect {
                        x: self.layout.unified_text_rect.x,
                        y: row_rect.y + 2.0,
                        width: self.layout.unified_text_rect.width,
                        height: row_height,
                    },
                    spans,
                    default_color: tone.default_text(theme),
                    font_size: 13.0,
                    font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
                });
            }
        }
    }

    fn paint_scrollbar(&self, scene: &mut Scene, theme: &Theme, state: &DiffViewportState) {
        if state.content_height_px <= state.viewport_height_px || state.viewport_height_px == 0 {
            return;
        }
        let track = self.layout.scrollbar_rect;
        let ratio = state.viewport_height_px as f32 / state.content_height_px as f32;
        let thumb_height = (track.height * ratio).max(32.0).min(track.height);
        let scroll_range = state.max_scroll_top_px().max(1) as f32;
        let top_ratio = state.scroll_top_px as f32 / scroll_range;
        let thumb_y = track.y + (track.height - thumb_height) * top_ratio;

        // Track background
        scene.rounded_rect(RoundedRectPrimitive::uniform(
            track,
            4.0,
            Color::rgba(128, 128, 128, 10),
        ));

        // Thumb
        scene.rounded_rect(RoundedRectPrimitive::uniform(
            Rect {
                x: track.x + 1.0,
                y: thumb_y + 1.0,
                width: track.width - 2.0,
                height: thumb_height - 2.0,
            },
            3.0,
            theme.colors.scrollbar_thumb,
        ));
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

fn build_layout(
    bounds: Rect,
    layout: LayoutMode,
    gutter_digits: u32,
    text_metrics: TextMetrics,
) -> ViewportLayout {
    let content_bounds = bounds.inset(VIEWPORT_PADDING_PX);
    let scrollbar_rect = Rect {
        x: content_bounds.right() - SCROLLBAR_WIDTH_PX,
        y: content_bounds.y + SCROLLBAR_MARGIN_PX,
        width: SCROLLBAR_WIDTH_PX,
        height: (content_bounds.height - SCROLLBAR_MARGIN_PX * 2.0).max(0.0),
    };
    let usable_width = (content_bounds.width - SCROLLBAR_WIDTH_PX - SCROLLBAR_MARGIN_PX).max(0.0);
    let gutter_width =
        gutter_digits as f32 * text_metrics.mono_char_width_px + GUTTER_PADDING_PX * 2.0;
    let unified_gutter_width = gutter_digits as f32 * text_metrics.mono_char_width_px * 2.0
        + text_metrics.mono_char_width_px
        + GUTTER_PADDING_PX * 2.0;

    if layout == LayoutMode::Split {
        let column_width = ((usable_width - gutter_width * 2.0 - COLUMN_GAP_PX) / 2.0).max(60.0);
        let left_gutter_rect = Rect {
            x: content_bounds.x,
            y: content_bounds.y,
            width: gutter_width,
            height: content_bounds.height,
        };
        let left_text_rect = Rect {
            x: left_gutter_rect.right(),
            y: content_bounds.y,
            width: column_width,
            height: content_bounds.height,
        };
        let right_gutter_rect = Rect {
            x: left_text_rect.right() + COLUMN_GAP_PX,
            y: content_bounds.y,
            width: gutter_width,
            height: content_bounds.height,
        };
        let right_text_rect = Rect {
            x: right_gutter_rect.right(),
            y: content_bounds.y,
            width: (content_bounds.right()
                - SCROLLBAR_WIDTH_PX
                - SCROLLBAR_MARGIN_PX
                - right_gutter_rect.right())
            .max(60.0),
            height: content_bounds.height,
        };
        ViewportLayout {
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
            scrollbar_rect,
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
        ViewportLayout {
            outer_bounds: bounds,
            content_bounds,
            split_mode: false,
            gutter_digits,
            unified_gutter_rect,
            unified_text_rect,
            left_gutter_rect: Rect::default(),
            left_text_rect: Rect::default(),
            right_gutter_rect: Rect::default(),
            right_text_rect: Rect::default(),
            scrollbar_rect,
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

fn format_line_number(line_no: u32, digits: u32) -> String {
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
) -> Option<Vec<RichTextSpan>> {
    if !text_range.is_valid() {
        return None;
    }
    let full_text = doc.line_text(text_range);
    if full_text.is_empty() {
        return Some(Vec::new());
    }
    let (start, end) = wrapped_byte_slice(full_text, wrap_cols, segment_index)?;
    Some(build_segment_spans(
        full_text,
        start,
        end,
        doc.line_runs(runs),
        tone,
        theme,
    ))
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
                text: full_text[cursor..start].to_owned(),
                color: tone.default_text(theme),
            });
        }

        spans.push(RichTextSpan {
            text: full_text[start..end].to_owned(),
            color: style_run_color(*run, tone, theme),
        });
        cursor = end;
    }

    if cursor < segment_end {
        spans.push(RichTextSpan {
            text: full_text[cursor..segment_end].to_owned(),
            color: tone.default_text(theme),
        });
    }

    if spans.is_empty() {
        spans.push(RichTextSpan {
            text: full_text[segment_start..segment_end].to_owned(),
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
        DiffViewportRuntime, ViewportDocument, build_wrapped_rich_text, wrapped_byte_slice,
    };
    use crate::core::compare::LayoutMode;
    use crate::render::{Rect, TextMetrics};
    use crate::ui::diff_viewport::render_doc::{
        ByteRange, RenderDoc, RenderLine, RenderRowKind, RunRange,
    };
    use crate::ui::diff_viewport::state::DiffViewportState;
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
            style_runs: vec![crate::ui::diff_viewport::render_doc::StyleRun {
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
                .map(|span| span.text.as_str())
                .collect::<String>(),
            "keyword value"
        );
    }

    #[test]
    fn prepare_populates_visible_range_and_hit_testing() {
        let mut state = DiffViewportState {
            layout: LayoutMode::Unified,
            ..DiffViewportState::default()
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

        let mut runtime = DiffViewportRuntime::default();
        runtime.prepare(
            &mut state,
            ViewportDocument::Text {
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
