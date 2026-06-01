use crate::actions::Action;
use crate::editor::diff::decoration::{
    BlockDecoration, BlockPaintCtx, BlockPlacement, BlockRegistry,
};
use crate::editor::diff::display_layout::DisplayLayoutMetrics;
use crate::editor::diff::render_doc::{RenderDoc, RenderRowKind};
use crate::render::Rect;
use crate::render::scene::{FontKind, FontWeight, IconPrimitive, Primitive, TextPrimitive};
use crate::ui::icons::lucide;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpandDirection {
    Above,
    Below,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkGapBudget {
    pub above_cap: u32,
    pub below_cap: u32,
}

#[derive(Debug)]
pub struct ExpandChipBlock {
    pub hunk_index: usize,
    pub direction: ExpandDirection,
    pub remaining_lines: u32,
    pub step: u32,
}

const EXPAND_STEP: u32 = 20;

impl ExpandChipBlock {
    fn label(&self) -> String {
        let direction_word = match self.direction {
            ExpandDirection::Above => "above",
            ExpandDirection::Below => "below",
        };
        if self.remaining_lines == u32::MAX {
            format!("Show {} more lines {}", self.step, direction_word)
        } else if self.remaining_lines <= self.step {
            format!("Show all {} lines {}", self.remaining_lines, direction_word)
        } else {
            format!("Show {} more lines {}", self.step, direction_word)
        }
    }
}

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
        let label = self.label();
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
            ExpandDirection::Above => {
                crate::actions::EditorAction::ExpandContextAbove(self.hunk_index, step).into()
            }
            ExpandDirection::Below => {
                crate::actions::EditorAction::ExpandContextBelow(self.hunk_index, step).into()
            }
        })
    }

    fn accessibility_label(&self) -> Option<String> {
        Some(self.label())
    }
}

pub fn populate_expand_blocks(
    blocks: &mut BlockRegistry,
    base_file: &carbon::FileDiff,
    render_doc: &RenderDoc,
    expansion: &carbon::ExpansionState,
) -> Vec<HunkGapBudget> {
    blocks.clear();
    if base_file.hunks.is_empty() || base_file.is_binary {
        return Vec::new();
    }

    let budgets = base_file
        .hunks
        .iter()
        .map(|hunk| {
            let caps = carbon::expansion_caps(base_file, hunk.id);
            let used = expansion.hunk(hunk.id);
            HunkGapBudget {
                above_cap: caps.above.saturating_sub(used.above),
                below_cap: caps.below.saturating_sub(used.below),
            }
        })
        .collect::<Vec<_>>();

    for (hunk_index, budget) in budgets.iter().enumerate() {
        if budget.above_cap == 0 {
            continue;
        }
        if let Some(anchor) = above_block_anchor(base_file, render_doc, expansion, hunk_index) {
            blocks.push(
                BlockPlacement::Above(anchor),
                Box::new(ExpandChipBlock {
                    hunk_index,
                    direction: ExpandDirection::Above,
                    remaining_lines: budget.above_cap,
                    step: EXPAND_STEP,
                }),
            );
        }
    }

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

fn above_block_anchor(
    base_file: &carbon::FileDiff,
    render_doc: &RenderDoc,
    expansion: &carbon::ExpansionState,
    hunk_index: usize,
) -> Option<u32> {
    let hunk = base_file.hunks.get(hunk_index)?;
    let used_above = expansion.hunk(hunk.id).above;
    if used_above > 0 {
        let old_line_no = hunk
            .old_start_index()
            .saturating_sub(used_above)
            .saturating_add(1);
        let new_line_no = hunk
            .new_start_index()
            .saturating_sub(used_above)
            .saturating_add(1);
        if let Some(anchor) = render_doc.lines.iter().position(|line| {
            line.row_kind() == RenderRowKind::Context
                && line.old_line_no == old_line_no
                && line.new_line_no == new_line_no
        }) {
            return Some(anchor as u32);
        }
    }

    render_doc
        .lines
        .iter()
        .position(|line| {
            line.row_kind() == RenderRowKind::HunkSeparator
                && usize::try_from(line.hunk_index).ok() == Some(hunk_index)
        })
        .map(|anchor| anchor as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::text::TokenBuffer;
    use crate::editor::diff::render_doc::build_render_doc_from_carbon;

    #[test]
    fn above_context_chip_moves_above_revealed_context() {
        let mut file = carbon::parse_unified_patch(
            "\
diff --git a/src/lib.rs b/src/lib.rs
@@ -4 +4 @@
-old text
+new text
",
        )
        .unwrap()
        .files
        .into_iter()
        .next()
        .unwrap();
        file.old_text = Some(carbon::TextStore::from_text("one\ntwo\nthree\nold text\n"));
        file.new_text = Some(carbon::TextStore::from_text("one\ntwo\nthree\nnew text\n"));
        for block in &mut file.blocks {
            block.old.start = block.old_line_start.saturating_sub(1);
            block.new.start = block.new_line_start.saturating_sub(1);
        }
        file.is_partial = false;

        let mut expansion = carbon::ExpansionState::default();
        carbon::expand_context(
            &file,
            &mut expansion,
            file.hunks[0].id,
            carbon::ExpansionDirection::Above,
            1,
        );
        let doc = build_render_doc_from_carbon(
            &file,
            0,
            &expansion,
            &Default::default(),
            &TokenBuffer::default(),
        );
        let first_revealed = doc
            .lines
            .iter()
            .position(|line| line.old_line_no == 3 && line.new_line_no == 3)
            .expect("revealed context line");
        let mut blocks = BlockRegistry::new();

        let budgets = populate_expand_blocks(&mut blocks, &file, &doc, &expansion);

        assert_eq!(budgets[0].above_cap, 2);
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks.placement(0),
            Some(BlockPlacement::Above(first_revealed as u32))
        );
    }
}
