use crate::render::{FontKind, FontWeight, Rect, RectPrimitive, Scene, TextPrimitive};
use crate::ui::theme::Theme;

use super::display_layout::DisplayLayoutMetrics;
use super::element::EditorLayout;
use super::render_doc::{RenderDoc, RenderLine, RenderRowKind};

pub struct RowPaintCtx<'a> {
    pub scene: &'a mut Scene,
    pub theme: &'a Theme,
    pub layout: &'a EditorLayout,
    pub row_rect: Rect,
    pub text_y_offset: f32,
    pub font_size: f32,
    pub line: &'a RenderLine,
    pub doc: &'a RenderDoc,
    pub path: &'a str,
}

impl RowPaintCtx<'_> {
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

pub trait RowDecoration {
    fn height(&self, metrics: &DisplayLayoutMetrics) -> u16;

    fn paint_background(&self, _ctx: &mut RowPaintCtx) {}

    fn paint_content(&self, _ctx: &mut RowPaintCtx) {}
}

pub struct HunkSeparatorDecoration;

impl RowDecoration for HunkSeparatorDecoration {
    fn height(&self, metrics: &DisplayLayoutMetrics) -> u16 {
        metrics.hunk_height_px
    }

    fn paint_background(&self, ctx: &mut RowPaintCtx) {
        ctx.scene.rect(RectPrimitive {
            rect: ctx.row_rect,
            color: ctx.theme.colors.hunk_header_bg,
        });
    }

    fn paint_content(&self, ctx: &mut RowPaintCtx) {
        ctx.scene.text(TextPrimitive {
            rect: Rect {
                x: ctx.text_origin_x(),
                y: ctx.row_rect.y + ctx.text_y_offset,
                width: ctx.text_width(),
                height: ctx.row_rect.height,
            },
            text: ctx.doc.line_text(ctx.line.left_text).into(),
            color: ctx.theme.colors.text_muted,
            font_size: ctx.font_size,
            font_kind: FontKind::Mono,
            font_weight: FontWeight::Normal,
        });
    }
}

pub fn decoration_for_kind(kind: RenderRowKind) -> Option<&'static dyn RowDecoration> {
    match kind {
        RenderRowKind::HunkSeparator => Some(&HunkSeparatorDecoration),
        RenderRowKind::FileHeader
        | RenderRowKind::Context
        | RenderRowKind::Added
        | RenderRowKind::Removed
        | RenderRowKind::Modified => None,
    }
}

/// Reserved for the future block-injection registry (expandable context, etc).
/// Not consumed yet — no display-layout pass merges blocks today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockPlacement {
    Above(u32),
    Below(u32),
}

impl BlockPlacement {
    pub fn anchor_line_index(self) -> u32 {
        match self {
            Self::Above(idx) | Self::Below(idx) => idx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockPlacement, RowDecoration, decoration_for_kind};
    use crate::ui::editor::display_layout::DisplayLayoutMetrics;
    use crate::ui::editor::render_doc::RenderRowKind;

    #[test]
    fn hunk_separator_has_decoration_body_and_file_header_do_not() {
        assert!(decoration_for_kind(RenderRowKind::HunkSeparator).is_some());
        assert!(decoration_for_kind(RenderRowKind::FileHeader).is_none());
        assert!(decoration_for_kind(RenderRowKind::Context).is_none());
        assert!(decoration_for_kind(RenderRowKind::Added).is_none());
        assert!(decoration_for_kind(RenderRowKind::Removed).is_none());
        assert!(decoration_for_kind(RenderRowKind::Modified).is_none());
    }

    #[test]
    fn hunk_separator_height_comes_from_metrics() {
        let metrics = DisplayLayoutMetrics {
            body_row_height_px: 20,
            file_header_height_px: 40,
            hunk_height_px: 24,
        };
        let hunk = decoration_for_kind(RenderRowKind::HunkSeparator).expect("hunk separator");
        assert_eq!(RowDecoration::height(hunk, &metrics), 24);
    }

    #[test]
    fn block_placement_reports_anchor() {
        assert_eq!(BlockPlacement::Above(7).anchor_line_index(), 7);
        assert_eq!(BlockPlacement::Below(42).anchor_line_index(), 42);
    }
}
