use crate::actions::{Action, ContextMenuEntry};
use crate::editor::diff::anchor::{EditorOverlayKind, ResolvedEditorOverlay};
use crate::editor::diff::decoration::BlockActionCtx;
use crate::editor::diff::render_doc::{
    ByteRange, DisplayRow, RenderDoc, RenderLine, RenderRowKind,
};
use crate::editor::diff::state::{
    EditorState, LineSelection, ViewportTextPoint, ViewportTextSelection, ViewportTextSide,
};
use crate::render::Rect;

use super::layout::{editor_scale, scaled};
use super::paint::unified_body_side_with_side;
use super::{CachedTextLayout, EditorElement, EditorLayout, TextBlock};

impl EditorElement {
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

    pub fn review_add_button_overlay(
        &self,
        state: &EditorState,
        doc: &RenderDoc,
        clip: Rect,
    ) -> Option<ResolvedEditorOverlay> {
        if !state.review_enabled {
            return None;
        }
        let row_index = self.layout.highlighted_row?;
        self.review_add_button_overlay_for_row(state, doc, row_index, clip)
    }

    pub fn review_add_button_overlay_at(
        &self,
        state: &EditorState,
        doc: &RenderDoc,
        x: f32,
        y: f32,
        clip: Rect,
    ) -> Option<ResolvedEditorOverlay> {
        if !state.review_enabled {
            return None;
        }
        let row_index = self.hit_test_row(state, x, y)?;
        let overlay = self.review_add_button_overlay_for_row(state, doc, row_index, clip)?;
        overlay.contains(x, y).then_some(overlay)
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
            line_selection_contains_line(
                &state.line_selection,
                file_path_for_line(doc, row.line_index as usize),
                line,
            )
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
        let s = editor_scale(self.text_metrics);
        let size = (self.layout.line_height * 0.72).clamp(scaled(14.0, s), scaled(20.0, s));
        // Straddle the gutter→code divider like GitHub: centered on the boundary,
        // but never reaching left past the line number's right edge.
        let x = (gutter.right() - size * 0.5).max(gutter.right() - self.layout.gutter_padding);
        Some(Rect {
            x,
            y: row_rect.y + (self.layout.line_height - size).max(0.0) * 0.5,
            width: size,
            height: size,
        })
    }

    fn review_add_button_overlay_for_row(
        &self,
        state: &EditorState,
        doc: &RenderDoc,
        row_index: usize,
        clip: Rect,
    ) -> Option<ResolvedEditorOverlay> {
        let display_row = self.rows.get(row_index).copied()?;
        if display_row.is_block() {
            return None;
        }
        let line = doc.lines.get(display_row.line_index as usize)?;
        let rect = self.review_add_comment_button_rect_for_row(line, &display_row)?;
        let emphasised = state.review_add_hovered
            || (!state.line_selection.is_empty()
                && line_selection_contains_line(
                    &state.line_selection,
                    file_path_for_line(doc, display_row.line_index as usize),
                    line,
                ));
        ResolvedEditorOverlay::new(
            EditorOverlayKind::ReviewAddButton {
                line_index: display_row.line_index as usize,
                emphasised,
            },
            rect,
            clip,
        )
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

    pub(super) fn text_blocks_for_line(
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
}

pub(super) fn line_selection_contains_line(
    selection: &LineSelection,
    file_path: Option<&str>,
    line: &RenderLine,
) -> bool {
    let Ok(hunk_id) = u32::try_from(line.hunk_index) else {
        return false;
    };
    (line.old_line_index >= 0
        && selection.contains_in_file(
            file_path,
            hunk_id,
            carbon::DiffSide::Old,
            line.old_line_index as u32,
        ))
        || (line.new_line_index >= 0
            && selection.contains_in_file(
                file_path,
                hunk_id,
                carbon::DiffSide::New,
                line.new_line_index as u32,
            ))
}

pub(super) fn file_path_for_line(doc: &RenderDoc, line_index: usize) -> Option<&str> {
    doc.lines.get(..=line_index)?.iter().rev().find_map(|line| {
        if line.row_kind() == RenderRowKind::FileHeader {
            doc.file_meta(line).map(|meta| meta.path.as_str())
        } else {
            None
        }
    })
}

fn review_comment_gutter_rect(layout: &EditorLayout, line: &RenderLine) -> Option<Rect> {
    // Any body line is commentable (incl. context) — selected_review_range maps it
    // to the new side, or the old side for removed-only lines.
    if line.hunk_index < 0 || !line.row_kind().is_body() {
        return None;
    }
    match line.row_kind() {
        RenderRowKind::Removed if layout.split_mode => Some(layout.left_gutter_rect),
        _ if layout.split_mode => Some(layout.right_gutter_rect),
        _ => Some(layout.unified_gutter_rect),
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

pub(super) fn selection_byte_range_for_side(
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
