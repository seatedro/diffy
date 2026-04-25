use crate::actions::Action;
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
            ExpandDirection::Above => {
                crate::actions::EditorAction::ExpandContextAbove(self.hunk_index, step).into()
            }
            ExpandDirection::Below => {
                crate::actions::EditorAction::ExpandContextBelow(self.hunk_index, step).into()
            }
        })
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
