use std::collections::BTreeMap;

use crate::core::forge::github::{GitHubReviewSide, PullRequestReviewComment};
use crate::render::{
    BorderPrimitive, FontKind, FontWeight, Rect, RectPrimitive, RoundedRectPrimitive, TextPrimitive,
};
use crate::ui::editor::decoration::{
    BlockDecoration, BlockPaintCtx, BlockPlacement, BlockRegistry,
};
use crate::ui::editor::display_layout::DisplayLayoutMetrics;
use crate::ui::editor::render_doc::{INVALID_U32, RenderDoc};

#[derive(Debug, Clone)]
pub struct ReviewCommentBlock {
    comments: Vec<PullRequestReviewComment>,
}

impl ReviewCommentBlock {
    pub fn new(comments: Vec<PullRequestReviewComment>) -> Self {
        Self { comments }
    }

    fn row_count(&self) -> u16 {
        let rows = self
            .comments
            .iter()
            .map(|comment| {
                let body_lines = comment
                    .body
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .take(3)
                    .count()
                    .max(1);
                1_u32.saturating_add(body_lines as u32)
            })
            .sum::<u32>()
            .saturating_add(1)
            .max(2);
        rows.min(u32::from(u16::MAX)) as u16
    }

    fn accessibility_summary(&self) -> String {
        let count = self.comments.len();
        let mut label = if count == 1 {
            "1 review comment".to_owned()
        } else {
            format!("{count} review comments")
        };
        for comment in self.comments.iter().take(3) {
            let author = comment
                .user
                .as_ref()
                .map(|user| user.login.as_str())
                .filter(|login| !login.is_empty())
                .unwrap_or("unknown");
            let snippet = comment
                .body
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("(empty comment)");
            label.push_str("; @");
            label.push_str(author);
            label.push_str(": ");
            label.push_str(snippet);
        }
        label
    }
}

impl BlockDecoration for ReviewCommentBlock {
    fn height(&self, metrics: &DisplayLayoutMetrics) -> u16 {
        metrics.body_row_height_px.saturating_mul(self.row_count())
    }

    fn paint(&self, ctx: &mut BlockPaintCtx) {
        let text_rect = if ctx.layout.split_mode {
            let left = ctx.layout.left_text_rect.x;
            let right = ctx.layout.right_text_rect.x + ctx.layout.right_text_rect.width;
            Rect {
                x: left,
                y: ctx.row_rect.y,
                width: right - left,
                height: ctx.row_rect.height,
            }
        } else {
            Rect {
                x: ctx.layout.unified_text_rect.x,
                y: ctx.row_rect.y,
                width: ctx.layout.unified_text_rect.width,
                height: ctx.row_rect.height,
            }
        };
        let panel = Rect {
            x: text_rect.x,
            y: ctx.row_rect.y + 4.0,
            width: text_rect.width.min(760.0),
            height: (ctx.row_rect.height - 8.0).max(0.0),
        };
        ctx.scene.rounded_rect(RoundedRectPrimitive::uniform(
            panel,
            6.0,
            ctx.theme.colors.modal_surface,
        ));
        ctx.scene.border(BorderPrimitive::uniform(
            panel,
            1.0,
            6.0,
            ctx.theme.colors.border,
        ));

        let stripe = Rect {
            x: panel.x,
            y: panel.y,
            width: 3.0,
            height: panel.height,
        };
        ctx.scene.rect(RectPrimitive {
            rect: stripe,
            color: ctx.theme.colors.accent,
        });

        let mut y = panel.y + 8.0;
        let x = panel.x + 12.0;
        let w = (panel.width - 20.0).max(0.0);
        let line_h = ctx
            .theme
            .metrics
            .ui_row_height
            .min(ctx.row_rect.height)
            .max(18.0);
        for comment in &self.comments {
            let author = comment
                .user
                .as_ref()
                .map(|user| user.login.as_str())
                .filter(|login| !login.is_empty())
                .unwrap_or("unknown");
            let header = if comment.line.is_none() && comment.original_line.is_some() {
                format!("@{author} commented on an outdated line")
            } else {
                format!("@{author}")
            };
            ctx.scene.text(TextPrimitive {
                rect: Rect {
                    x,
                    y,
                    width: w,
                    height: line_h,
                },
                text: header.into(),
                color: ctx.theme.colors.text_strong,
                font_size: ctx.theme.metrics.ui_small_font_size,
                font_kind: FontKind::Ui,
                font_weight: FontWeight::Medium,
            });
            y += line_h;

            let mut emitted = 0;
            for body_line in comment
                .body
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(3)
            {
                ctx.scene.text(TextPrimitive {
                    rect: Rect {
                        x,
                        y,
                        width: w,
                        height: line_h,
                    },
                    text: body_line.to_owned().into(),
                    color: ctx.theme.colors.text,
                    font_size: ctx.theme.metrics.ui_small_font_size,
                    font_kind: FontKind::Ui,
                    font_weight: FontWeight::Normal,
                });
                y += line_h;
                emitted += 1;
            }
            if emitted == 0 {
                ctx.scene.text(TextPrimitive {
                    rect: Rect {
                        x,
                        y,
                        width: w,
                        height: line_h,
                    },
                    text: "(empty comment)".into(),
                    color: ctx.theme.colors.text_muted,
                    font_size: ctx.theme.metrics.ui_small_font_size,
                    font_kind: FontKind::Ui,
                    font_weight: FontWeight::Normal,
                });
                y += line_h;
            }
        }
    }

    fn accessibility_label(&self) -> Option<String> {
        Some(self.accessibility_summary())
    }
}

pub fn populate_review_comment_blocks(
    blocks: &mut BlockRegistry,
    render_doc: &RenderDoc,
    comments: &[PullRequestReviewComment],
) {
    let mut grouped: BTreeMap<u32, Vec<PullRequestReviewComment>> = BTreeMap::new();
    for comment in comments {
        let Some(anchor) = anchor_line_index(render_doc, comment) else {
            continue;
        };
        grouped.entry(anchor).or_default().push(comment.clone());
    }

    for (anchor, mut comments) in grouped {
        comments.sort_by_key(|comment| (comment.in_reply_to_id.unwrap_or(comment.id), comment.id));
        blocks.push(
            BlockPlacement::Below(anchor),
            Box::new(ReviewCommentBlock::new(comments)),
        );
    }
}

fn anchor_line_index(render_doc: &RenderDoc, comment: &PullRequestReviewComment) -> Option<u32> {
    let side = comment.side?;
    let line = comment.line.or(comment.original_line)?;
    render_doc
        .lines
        .iter()
        .enumerate()
        .find_map(|(idx, render_line)| match side {
            GitHubReviewSide::Right if render_line.new_line_no == line => Some(idx as u32),
            GitHubReviewSide::Left if render_line.old_line_no == line => Some(idx as u32),
            _ => None,
        })
        .or_else(|| {
            render_doc
                .lines
                .iter()
                .enumerate()
                .find_map(|(idx, render_line)| {
                    (render_line.old_line_no != INVALID_U32
                        || render_line.new_line_no != INVALID_U32)
                        .then_some(idx as u32)
                })
        })
}
