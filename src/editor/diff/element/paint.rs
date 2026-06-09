use std::ops::Range;
use std::sync::Arc;

use accesskit::Role;

use crate::core::text::SyntaxTokenKind;
use crate::editor::diff::decoration::{
    BlockPaintCtx, FileHeaderDecoration, RowDecoration, RowPaintCtx, decoration_for_kind,
};
use crate::editor::diff::render_doc::{
    ByteRange, DisplayRow, FileHeaderMeta, INVALID_U32, RENDER_FLAG_STRUCTURAL, RenderDoc,
    RenderLine, RenderRowKind, RunRange, STYLE_FLAG_CHANGE, STYLE_FLAG_UNCHANGED_CTX, StyleRun,
};
use crate::editor::diff::state::{EditorState, ViewportTextSide};
use crate::render::{
    FontKind, FontStyle, FontWeight, Rect, RectPrimitive, RichTextPrimitive, RichTextSpan,
    RoundedRectPrimitive, Scene, TextPrimitive,
};
use crate::ui::accessibility::{AccessibilityAction, AccessibilityFrame, AccessibilityNode};
use crate::ui::design::{Alpha, Sz};
use crate::ui::element::ScrollActionBuilder;
use crate::ui::state::FocusTarget;
use crate::ui::theme::{Color, Theme};

use super::hit_test::{
    file_path_for_line, line_selection_contains_line, selection_byte_range_for_side,
};
use super::layout::{editor_scale, scaled};
use super::{CachedTextLayout, EditorDocument, EditorElement, GutterTextCacheKey, GutterTextKind};

const STICKY_HEADER_Z: i32 = 10;
const INLINE_CHANGE_BG_MERGE_GAP_COLS: u32 = 2;
const INLINE_CHANGE_BG_Y_INSET_RATIO: f32 = 0.10;

impl EditorElement {
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
                file_index,
                path,
                doc,
                show_file_headers,
            } => {
                // Row geometry is only valid for the exact document `prepare`
                // built it from. If a stale document (older compare
                // generation or different file) reaches paint, skip the body
                // for this frame instead of painting mismatched geometry;
                // the next prepare/paint pass recovers.
                let layout_matches = self.layout_key.is_some_and(|key| {
                    key.compare_generation == compare_generation && key.file_index == file_index
                });
                if !layout_matches || _state.doc_generation != compare_generation {
                    tracing::warn!(
                        compare_generation,
                        file_index,
                        layout_generation = ?self.layout_key.map(|key| key.compare_generation),
                        state_generation = _state.doc_generation,
                        "editor layout/document generation mismatch; skipping paint"
                    );
                    return;
                }
                self.sync_theme_cache(theme);
                scene.clip(self.layout.content_bounds);

                self.paint_gutter_backgrounds(scene, theme);
                self.paint_row_backgrounds(scene, theme, path, doc);
                self.paint_inline_change_backgrounds(scene, theme, doc);
                self.paint_line_highlights(scene, theme);
                self.paint_line_selection(scene, theme, _state, doc);
                self.paint_viewport_text_selection(scene, theme, _state, doc, compare_generation);
                self.paint_search_highlights(scene, theme, _state, doc);
                self.paint_gutter_diff_indicators(scene, theme, doc);
                self.paint_gutter_decorations(scene, theme);
                self.paint_gutter_text(scene, theme, doc);
                self.paint_body_text(scene, theme, path, doc);
                // The add-comment "+" is resolved as an editor overlay and rendered by
                // the shell, not hand-painted here.
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
            && line_selection_contains_line(&state.line_selection, Some(path), line);
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
            let selected = line_selection_contains_line(
                &state.line_selection,
                file_path_for_line(doc, display_row.line_index as usize),
                line,
            );
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
        use crate::editor::diff::state::MatchSide;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum RowTone {
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

pub(super) fn unified_body_side_with_side(
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

pub(super) fn build_wrapped_rich_text(
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
pub(super) fn wrapped_byte_slice(
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
