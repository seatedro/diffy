use std::collections::HashMap;

use crate::actions::Action;
use crate::core::diff::types::{DiffLine, FileDiff, LineKind};
use crate::core::text::TextBuffer;
use crate::render::Rect;
use crate::render::scene::{FontKind, FontWeight, IconPrimitive, Primitive, TextPrimitive};
use crate::ui::editor::decoration::{
    BlockDecoration, BlockPaintCtx, BlockPlacement, BlockRegistry,
};
use crate::ui::editor::display_layout::DisplayLayoutMetrics;
use crate::ui::editor::render_doc::RenderDoc;
use crate::ui::icons::lucide;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpandDirection {
    Above,
    Below,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HunkExpansion {
    pub above: u32,
    pub below: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileExpansion {
    pub hunks: Vec<HunkExpansion>,
}

impl FileExpansion {
    pub fn ensure_hunk_count(&mut self, count: usize) {
        if self.hunks.len() < count {
            self.hunks.resize(count, HunkExpansion::default());
        }
    }

    pub fn hunk(&self, index: usize) -> HunkExpansion {
        self.hunks.get(index).copied().unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.hunks.iter().all(|h| h.above == 0 && h.below == 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkGapBudget {
    pub above_cap: u32,
    pub below_cap: u32,
}

pub fn gap_budgets(
    file: &FileDiff,
    expansion: &FileExpansion,
    total_lines: Option<u32>,
) -> Vec<HunkGapBudget> {
    let mut budgets = Vec::with_capacity(file.hunks.len());
    for (idx, hunk) in file.hunks.iter().enumerate() {
        let base_new_start = (hunk.new_start - 1).max(0) as u32;
        let above_used = expansion.hunk(idx).above;
        let above_cap = base_new_start.saturating_sub(above_used);

        let below_cap = if let Some(next) = file.hunks.get(idx + 1) {
            let gap_between = (next.new_start - (hunk.new_start + hunk.new_count)).max(0) as u32;
            let this_below = expansion.hunk(idx).below;
            let next_above = expansion.hunk(idx + 1).above;
            gap_between.saturating_sub(this_below + next_above)
        } else if let Some(total) = total_lines {
            let new_end = (hunk.new_start + hunk.new_count - 1).max(0) as u32;
            total
                .saturating_sub(new_end)
                .saturating_sub(expansion.hunk(idx).below)
        } else {
            // Total unknown — be optimistic so the below chip remains clickable.
            // apply_expansion clamps against the actual file length once fetched.
            u32::MAX
        };

        budgets.push(HunkGapBudget {
            above_cap,
            below_cap,
        });
    }
    budgets
}

pub fn apply_expansion(
    base_file: &FileDiff,
    base_text_buffer: &TextBuffer,
    expansion: &FileExpansion,
    lines: &[String],
) -> (FileDiff, TextBuffer) {
    let mut new_buffer = base_text_buffer.clone();
    let mut new_file = base_file.clone();

    for (hunk_index, hunk) in new_file.hunks.iter_mut().enumerate() {
        let exp = expansion.hunk(hunk_index);
        if exp.above == 0 && exp.below == 0 {
            continue;
        }

        let mut above_lines = Vec::new();
        if exp.above > 0 {
            // `lines` is read from the new-side ref, so index by new-side coords.
            // Clamp against the actual headroom available in the new file.
            let base_new_start = (hunk.new_start - 1).max(0) as usize;
            let take = (exp.above as usize).min(base_new_start);
            let start = base_new_start - take;
            for (offset, idx) in (start..base_new_start).enumerate() {
                let text = lines.get(idx).map(String::as_str).unwrap_or("");
                let range = new_buffer.append(text);
                let old_line = (hunk.old_start - take as i32 + offset as i32).max(1);
                let new_line = (idx + 1) as i32;
                above_lines.push(DiffLine {
                    kind: LineKind::Context,
                    old_line_number: Some(old_line),
                    new_line_number: Some(new_line),
                    text_range: range,
                    ..DiffLine::default()
                });
            }
        }

        let mut below_lines = Vec::new();
        if exp.below > 0 {
            let old_end = (hunk.old_start + hunk.old_count - 1).max(0) as usize;
            let new_end = (hunk.new_start + hunk.new_count - 1).max(0) as usize;
            let available = lines.len().saturating_sub(new_end);
            let take = (exp.below as usize).min(available);
            for offset in 0..take {
                let idx = new_end + offset;
                let text = lines.get(idx).map(String::as_str).unwrap_or("");
                let range = new_buffer.append(text);
                below_lines.push(DiffLine {
                    kind: LineKind::Context,
                    old_line_number: Some((old_end + offset + 1) as i32),
                    new_line_number: Some((new_end + offset + 1) as i32),
                    text_range: range,
                    ..DiffLine::default()
                });
            }
        }

        if !above_lines.is_empty() {
            let n = above_lines.len() as i32;
            hunk.old_start = (hunk.old_start - n).max(1);
            hunk.new_start = (hunk.new_start - n).max(1);
            hunk.old_count += n;
            hunk.new_count += n;
            let mut merged = above_lines;
            merged.extend(std::mem::take(&mut hunk.lines));
            hunk.lines = merged;
        }

        if !below_lines.is_empty() {
            let n = below_lines.len() as i32;
            hunk.old_count += n;
            hunk.new_count += n;
            hunk.lines.extend(below_lines);
        }
    }

    (new_file, new_buffer)
}

pub type FileExpansionMap = HashMap<String, FileExpansion>;

#[derive(Debug)]
pub struct ExpandChipBlock {
    pub hunk_index: usize,
    pub direction: ExpandDirection,
    pub remaining_lines: u32,
    pub step: u32,
}

const EXPAND_STEP: u32 = 20;

impl BlockDecoration for ExpandChipBlock {
    fn height(&self, metrics: &DisplayLayoutMetrics) -> u16 {
        metrics.body_row_height_px
    }

    fn paint(&self, ctx: &mut BlockPaintCtx) {
        ctx.scene.rect(crate::render::scene::RectPrimitive {
            rect: ctx.row_rect,
            color: ctx.theme.colors.hunk_header_bg,
        });

        let gutter = if ctx.layout.split_mode {
            ctx.layout.left_gutter_rect
        } else {
            ctx.layout.unified_gutter_rect
        };
        let gutter = Rect {
            x: gutter.x,
            y: ctx.row_rect.y,
            width: gutter.width,
            height: ctx.row_rect.height,
        };
        if ctx.hovered {
            ctx.scene.rect(crate::render::scene::RectPrimitive {
                rect: gutter,
                color: ctx.theme.colors.element_hover,
            });
        }

        let icon_svg = match self.direction {
            ExpandDirection::Above => lucide::CHEVRON_UP,
            ExpandDirection::Below => lucide::CHEVRON_DOWN,
        };
        let text_color = if ctx.hovered {
            ctx.theme.colors.text_strong
        } else {
            ctx.theme.colors.text_muted
        };

        let icon_size = (ctx.row_rect.height.min(gutter.width)).max(8.0) * 0.75;
        let icon_x = gutter.x + (gutter.width - icon_size) * 0.5;
        let icon_y = gutter.y + (gutter.height - icon_size) * 0.5;
        ctx.scene.push(Primitive::Icon(IconPrimitive {
            rect: Rect {
                x: icon_x.round(),
                y: icon_y.round(),
                width: icon_size.round(),
                height: icon_size.round(),
            },
            name: icon_svg.to_owned(),
            color: text_color,
        }));

        let text_origin = if ctx.layout.split_mode {
            ctx.layout.left_text_rect
        } else {
            ctx.layout.unified_text_rect
        };
        let direction_word = match self.direction {
            ExpandDirection::Above => "above",
            ExpandDirection::Below => "below",
        };
        let label = if self.remaining_lines == u32::MAX {
            format!("Show {} more lines {}", self.step, direction_word)
        } else if self.remaining_lines <= self.step {
            format!("Show all {} lines {}", self.remaining_lines, direction_word)
        } else {
            format!("Show {} more lines {}", self.step, direction_word)
        };
        ctx.scene.text(TextPrimitive {
            rect: Rect {
                x: text_origin.x,
                y: ctx.row_rect.y + ctx.text_y_offset,
                width: text_origin.width,
                height: ctx.row_rect.height,
            },
            text: label.into(),
            color: text_color,
            font_size: ctx.font_size,
            font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
        });
    }

    fn on_click(&self) -> Option<Action> {
        let step = self.step.min(self.remaining_lines).max(1);
        Some(match self.direction {
            ExpandDirection::Above => Action::ExpandContextAbove(self.hunk_index, step),
            ExpandDirection::Below => Action::ExpandContextBelow(self.hunk_index, step),
        })
    }
}

pub fn populate_expand_blocks(
    blocks: &mut BlockRegistry,
    base_file: &FileDiff,
    render_doc: &RenderDoc,
    expansion: &FileExpansion,
    total_lines: Option<u32>,
) -> Vec<HunkGapBudget> {
    blocks.clear();
    if base_file.hunks.is_empty() || base_file.is_binary {
        return Vec::new();
    }

    let budgets = gap_budgets(base_file, expansion, total_lines);

    if let Some((last_idx, last_budget)) = budgets.iter().enumerate().last()
        && last_budget.below_cap > 0
    {
        let anchor = render_doc
            .lines
            .iter()
            .rposition(|l| l.hunk_index as usize == last_idx && l.row_kind().is_body());
        if let Some(anchor) = anchor {
            blocks.push(
                BlockPlacement::Below(anchor as u32),
                Box::new(ExpandChipBlock {
                    hunk_index: last_idx,
                    direction: ExpandDirection::Below,
                    remaining_lines: last_budget.below_cap,
                    step: EXPAND_STEP,
                }),
            );
        }
    }

    budgets
}

#[cfg(test)]
mod tests {
    use super::{FileExpansion, HunkExpansion, apply_expansion, gap_budgets};
    use crate::core::diff::types::{DiffLine, FileDiff, Hunk, LineKind};
    use crate::core::text::TextBuffer;

    fn sample_file() -> (FileDiff, TextBuffer) {
        let mut buffer = TextBuffer::default();
        let removed = buffer.append("old");
        let added = buffer.append("new");
        let file = FileDiff {
            path: "src/lib.rs".to_owned(),
            hunks: vec![Hunk {
                old_start: 10,
                old_count: 1,
                new_start: 10,
                new_count: 1,
                header: "@@".to_owned(),
                lines: vec![
                    DiffLine {
                        kind: LineKind::Removed,
                        old_line_number: Some(10),
                        text_range: removed,
                        ..DiffLine::default()
                    },
                    DiffLine {
                        kind: LineKind::Added,
                        new_line_number: Some(10),
                        text_range: added,
                        ..DiffLine::default()
                    },
                ],
            }],
            ..FileDiff::default()
        };
        (file, buffer)
    }

    #[test]
    fn apply_expansion_above_prepends_context_lines() {
        let (base, buffer) = sample_file();
        let mut exp = FileExpansion::default();
        exp.ensure_hunk_count(1);
        exp.hunks[0].above = 2;
        let file_lines: Vec<String> = (1..=20).map(|i| format!("line{i}")).collect();

        let (expanded, new_buffer) = apply_expansion(&base, &buffer, &exp, &file_lines);
        let hunk = &expanded.hunks[0];
        assert_eq!(hunk.old_start, 8);
        assert_eq!(hunk.old_count, 3);
        assert_eq!(hunk.new_start, 8);
        assert_eq!(hunk.new_count, 3);
        assert_eq!(hunk.lines.len(), 4);
        assert_eq!(hunk.lines[0].kind, LineKind::Context);
        assert_eq!(
            new_buffer.view(hunk.lines[0].text_range),
            "line8".to_owned()
        );
        assert_eq!(
            new_buffer.view(hunk.lines[1].text_range),
            "line9".to_owned()
        );
        assert_eq!(hunk.lines[2].kind, LineKind::Removed);
    }

    #[test]
    fn apply_expansion_below_appends_context_lines() {
        let (base, buffer) = sample_file();
        let mut exp = FileExpansion::default();
        exp.ensure_hunk_count(1);
        exp.hunks[0].below = 2;
        let file_lines: Vec<String> = (1..=20).map(|i| format!("line{i}")).collect();

        let (expanded, new_buffer) = apply_expansion(&base, &buffer, &exp, &file_lines);
        let hunk = &expanded.hunks[0];
        assert_eq!(hunk.old_start, 10);
        assert_eq!(hunk.old_count, 3);
        assert_eq!(hunk.new_start, 10);
        assert_eq!(hunk.new_count, 3);
        assert_eq!(hunk.lines.len(), 4);
        assert_eq!(hunk.lines[2].kind, LineKind::Context);
        assert_eq!(
            new_buffer.view(hunk.lines[2].text_range),
            "line11".to_owned()
        );
        assert_eq!(
            new_buffer.view(hunk.lines[3].text_range),
            "line12".to_owned()
        );
    }

    #[test]
    fn apply_expansion_above_clamps_at_start_of_file() {
        let (mut base, buffer) = sample_file();
        base.hunks[0].old_start = 2;
        base.hunks[0].new_start = 2;
        let mut exp = FileExpansion::default();
        exp.ensure_hunk_count(1);
        exp.hunks[0].above = 5;
        let file_lines: Vec<String> = (1..=20).map(|i| format!("line{i}")).collect();

        let (expanded, _buffer) = apply_expansion(&base, &buffer, &exp, &file_lines);
        let hunk = &expanded.hunks[0];
        // Only 1 line of headroom above line 2.
        let added = hunk.lines.len() - 2;
        assert!(added <= 5);
        assert!(hunk.old_start >= 1);
    }

    #[test]
    fn apply_expansion_indexes_new_side_when_counts_differ() {
        // Earlier hunk removed 3 lines. Later hunk's old_start (10) leads new_start (7)
        // by 3, so indexing `lines` (from the new-side ref) with old offsets would
        // pull the wrong content. Verify we read new-side lines.
        let mut buffer = TextBuffer::default();
        let removed = buffer.append("old-hunk");
        let added = buffer.append("new-hunk");
        let file = FileDiff {
            path: "src/lib.rs".to_owned(),
            hunks: vec![Hunk {
                old_start: 10,
                old_count: 1,
                new_start: 7,
                new_count: 1,
                header: "@@".to_owned(),
                lines: vec![
                    DiffLine {
                        kind: LineKind::Removed,
                        old_line_number: Some(10),
                        text_range: removed,
                        ..DiffLine::default()
                    },
                    DiffLine {
                        kind: LineKind::Added,
                        new_line_number: Some(7),
                        text_range: added,
                        ..DiffLine::default()
                    },
                ],
            }],
            ..FileDiff::default()
        };

        let mut exp = FileExpansion::default();
        exp.ensure_hunk_count(1);
        exp.hunks[0].above = 2;
        exp.hunks[0].below = 2;
        let file_lines: Vec<String> = (1..=20).map(|i| format!("line{i}")).collect();

        let (expanded, new_buffer) = apply_expansion(&file, &buffer, &exp, &file_lines);
        let hunk = &expanded.hunks[0];

        // Above: 2 context lines indexed from the new-side file at new_start-1=6.
        // Reads lines[4] and lines[5] = "line5", "line6".
        assert_eq!(hunk.lines.len(), 6);
        assert_eq!(hunk.lines[0].kind, LineKind::Context);
        assert_eq!(new_buffer.view(hunk.lines[0].text_range), "line5");
        assert_eq!(new_buffer.view(hunk.lines[1].text_range), "line6");
        assert_eq!(hunk.lines[2].kind, LineKind::Removed);
        assert_eq!(hunk.lines[3].kind, LineKind::Added);
        // Below: new_end=7, so lines[7]="line8", lines[8]="line9".
        assert_eq!(hunk.lines[4].kind, LineKind::Context);
        assert_eq!(new_buffer.view(hunk.lines[4].text_range), "line8");
        assert_eq!(new_buffer.view(hunk.lines[5].text_range), "line9");

        // new-side line numbers run contiguously 5..=9 around the hunk.
        assert_eq!(hunk.lines[0].new_line_number, Some(5));
        assert_eq!(hunk.lines[1].new_line_number, Some(6));
        assert_eq!(hunk.lines[4].new_line_number, Some(8));
        assert_eq!(hunk.lines[5].new_line_number, Some(9));
    }

    #[test]
    fn gap_budgets_share_budget_between_adjacent_hunks() {
        let mut file = FileDiff::default();
        file.hunks = vec![
            Hunk {
                old_start: 10,
                old_count: 2,
                new_start: 10,
                new_count: 2,
                ..Hunk::default()
            },
            Hunk {
                old_start: 20,
                old_count: 2,
                new_start: 20,
                new_count: 2,
                ..Hunk::default()
            },
        ];
        let mut expansion = FileExpansion::default();
        expansion.ensure_hunk_count(2);
        expansion.hunks[0].below = 3;
        expansion.hunks[1].above = 2;

        let budgets = gap_budgets(&file, &expansion, None);
        // gap = 20 - (10+2) = 8. used = 3 + 2 = 5. remaining = 3.
        assert_eq!(budgets[0].below_cap, 3);
        assert_eq!(budgets[1].above_cap, 20 - 1 - 2);
    }
}
