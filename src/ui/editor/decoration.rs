use std::sync::Arc;

use crate::actions::{Action, ContextMenuEntry};
use crate::render::scene::{IconPrimitive, Primitive, RichTextPrimitive, RichTextSpan};
use crate::render::{FontKind, FontWeight, Rect, RectPrimitive, Scene, TextPrimitive};
use crate::ui::icons::lucide;
use crate::ui::theme::Theme;

use super::display_layout::DisplayLayoutMetrics;
use super::element::{BASE_MONO_FONT_SIZE, EditorLayout};
use super::render_doc::{FileHeaderMeta, RenderDoc, RenderLine, RenderRowKind};

const BASE_HEADER_PAD_X: f32 = 10.0;
const BASE_HEADER_ICON_GAP: f32 = 8.0;
const BASE_HEADER_STATS_GAP: f32 = 12.0;
const BASE_HEADER_ICON_SIZE: f32 = 14.0;
const BASE_HEADER_COPY_ICON_SIZE: f32 = 12.0;
const BASE_HEADER_COPY_GAP: f32 = 8.0;
const HEADER_MIN_SCALE: f32 = 0.7;

pub struct RowPaintCtx<'a> {
    pub scene: &'a mut Scene,
    pub theme: &'a Theme,
    pub layout: &'a EditorLayout,
    pub row_rect: Rect,
    pub text_y_offset: f32,
    pub font_size: f32,
    pub mono_char_width_px: f32,
    pub line: &'a RenderLine,
    pub doc: &'a RenderDoc,
    pub path: &'a str,
    pub is_header_hovered: bool,
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

pub struct FileHeaderDecoration;

impl RowDecoration for FileHeaderDecoration {
    fn height(&self, metrics: &DisplayLayoutMetrics) -> u16 {
        metrics.file_header_height_px
    }

    fn paint_background(&self, ctx: &mut RowPaintCtx) {
        ctx.scene.rect(RectPrimitive {
            rect: ctx.row_rect,
            color: ctx.theme.colors.file_header_bg,
        });
        ctx.scene.rect(RectPrimitive {
            rect: Rect {
                x: ctx.row_rect.x,
                y: ctx.row_rect.y,
                width: ctx.row_rect.width,
                height: 1.0,
            },
            color: ctx.theme.colors.border_soft,
        });
        ctx.scene.rect(RectPrimitive {
            rect: Rect {
                x: ctx.row_rect.x,
                y: ctx.row_rect.bottom() - 1.0,
                width: ctx.row_rect.width,
                height: 1.0,
            },
            color: ctx.theme.colors.border,
        });
    }

    fn paint_content(&self, ctx: &mut RowPaintCtx) {
        let path_text = ctx.doc.line_text(ctx.line.left_text);
        let meta = ctx.doc.file_meta(ctx.line);
        let row_rect = ctx.row_rect;
        let hovered = ctx.is_header_hovered;
        let scale = (ctx.font_size / BASE_MONO_FONT_SIZE).max(HEADER_MIN_SCALE);
        let pad_x = (BASE_HEADER_PAD_X * scale).round();
        let icon_gap = (BASE_HEADER_ICON_GAP * scale).round();
        let stats_gap = (BASE_HEADER_STATS_GAP * scale).round();

        let content_left = ctx.text_origin_x();

        let icon_size = (BASE_HEADER_ICON_SIZE * scale).round();
        let icon_y = row_rect.y + (row_rect.height - icon_size) * 0.5;
        let icon_x = (row_rect.x + pad_x).round();
        let icon_color = ctx.theme.colors.text_muted;
        let icon_svg = if meta.map(|m| m.is_binary).unwrap_or(false) {
            lucide::FILE
        } else {
            lucide::FILE_DIFF
        };
        ctx.scene.push(Primitive::Icon(IconPrimitive {
            rect: Rect {
                x: icon_x,
                y: icon_y.round(),
                width: icon_size,
                height: icon_size,
            },
            name: icon_svg.to_owned(),
            color: icon_color,
        }));

        let path_x = (icon_x + icon_size + icon_gap).max(content_left).round();
        let baseline_y = row_rect.y + ctx.text_y_offset;

        let stats_string = meta
            .map(|m| stats_label(m.additions, m.deletions))
            .unwrap_or_default();
        let stats_width = if stats_string.is_empty() {
            0.0
        } else {
            (stats_string.chars().count() as f32 * ctx.mono_char_width_px).ceil()
        };
        let stats_right = row_rect.right() - pad_x;
        let stats_left = (stats_right - stats_width).round();
        let copy_icon_size = (BASE_HEADER_COPY_ICON_SIZE * scale).round();
        let copy_gap = (BASE_HEADER_COPY_GAP * scale).round();

        let path_chars = path_display_char_count(path_text, meta);
        let path_text_width = (path_chars as f32 * ctx.mono_char_width_px).ceil();
        let path_max_right = if stats_width > 0.0 {
            stats_left - stats_gap
        } else {
            stats_right
        };
        let path_width = (path_max_right - path_x).max(0.0).min(path_text_width);

        let (filename_color, dirname_color) = if hovered {
            (ctx.theme.colors.text_strong, ctx.theme.colors.text_strong)
        } else {
            (ctx.theme.colors.text_strong, ctx.theme.colors.text_muted)
        };
        let path_spans = build_path_spans(path_text, meta, filename_color, dirname_color);
        ctx.scene.rich_text(RichTextPrimitive {
            rect: Rect {
                x: path_x,
                y: baseline_y,
                width: path_width,
                height: row_rect.height,
            },
            spans: path_spans,
            default_color: filename_color,
            font_size: ctx.font_size,
            font_kind: FontKind::Mono,
            font_weight: FontWeight::Medium,
        });

        if hovered {
            let copy_icon_x = (path_x + path_text_width + copy_gap)
                .min(path_max_right - copy_icon_size)
                .round();
            let copy_icon_y = (row_rect.y + (row_rect.height - copy_icon_size) * 0.5).round();
            ctx.scene.push(Primitive::Icon(IconPrimitive {
                rect: Rect {
                    x: copy_icon_x,
                    y: copy_icon_y,
                    width: copy_icon_size,
                    height: copy_icon_size,
                },
                name: lucide::COPY.to_owned(),
                color: ctx.theme.colors.text,
            }));
        }

        if !stats_string.is_empty() {
            let spans = build_stats_spans(meta.unwrap(), ctx.theme);
            ctx.scene.rich_text(RichTextPrimitive {
                rect: Rect {
                    x: stats_left,
                    y: baseline_y,
                    width: stats_width,
                    height: row_rect.height,
                },
                spans,
                default_color: ctx.theme.colors.text_muted,
                font_size: ctx.font_size,
                font_kind: FontKind::Mono,
                font_weight: FontWeight::Medium,
            });
        }
    }
}

fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(idx) => path.split_at(idx + 1),
        None => ("", path),
    }
}

fn build_path_spans(
    path: &str,
    meta: Option<&FileHeaderMeta>,
    filename_color: crate::ui::theme::Color,
    dirname_color: crate::ui::theme::Color,
) -> Arc<[RichTextSpan]> {
    let mut spans: Vec<RichTextSpan> = Vec::with_capacity(5);
    if let Some(m) = meta
        && let Some(old) = m.old_path.as_deref()
    {
        spans.push(RichTextSpan {
            text: old.into(),
            color: dirname_color,
            ..RichTextSpan::default()
        });
        spans.push(RichTextSpan {
            text: " → ".into(),
            color: dirname_color,
            ..RichTextSpan::default()
        });
    }
    let (dir, base) = split_path(path);
    if !dir.is_empty() {
        spans.push(RichTextSpan {
            text: dir.into(),
            color: dirname_color,
            ..RichTextSpan::default()
        });
    }
    spans.push(RichTextSpan {
        text: base.into(),
        color: filename_color,
        ..RichTextSpan::default()
    });
    spans.into()
}

fn path_display_char_count(path: &str, meta: Option<&FileHeaderMeta>) -> usize {
    let mut count = path.chars().count();
    if let Some(m) = meta
        && let Some(old) = m.old_path.as_deref()
    {
        count += old.chars().count() + 3;
    }
    count
}

fn stats_label(additions: u32, deletions: u32) -> String {
    if additions == 0 && deletions == 0 {
        return String::new();
    }
    let mut out = String::new();
    if additions > 0 {
        out.push('+');
        out.push_str(&additions.to_string());
    }
    if deletions > 0 {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push('-');
        out.push_str(&deletions.to_string());
    }
    out
}

fn build_stats_spans(meta: &FileHeaderMeta, theme: &Theme) -> Arc<[RichTextSpan]> {
    let mut spans: Vec<RichTextSpan> = Vec::with_capacity(3);
    if meta.additions > 0 {
        spans.push(RichTextSpan {
            text: format!("+{}", meta.additions).into(),
            color: theme.colors.line_add_text,
            ..RichTextSpan::default()
        });
    }
    if meta.deletions > 0 {
        if !spans.is_empty() {
            spans.push(RichTextSpan {
                text: " ".into(),
                color: theme.colors.text_muted,
                ..RichTextSpan::default()
            });
        }
        spans.push(RichTextSpan {
            text: format!("-{}", meta.deletions).into(),
            color: theme.colors.line_del_text,
            ..RichTextSpan::default()
        });
    }
    spans.into()
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
        RenderRowKind::FileHeader => Some(&FileHeaderDecoration),
        RenderRowKind::HunkSeparator => Some(&HunkSeparatorDecoration),
        RenderRowKind::Context
        | RenderRowKind::Added
        | RenderRowKind::Removed
        | RenderRowKind::Modified
        | RenderRowKind::Block => None,
    }
}

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

pub struct BlockPaintCtx<'a> {
    pub scene: &'a mut Scene,
    pub theme: &'a Theme,
    pub layout: &'a EditorLayout,
    pub row_rect: Rect,
    pub text_y_offset: f32,
    pub font_size: f32,
    pub hovered: bool,
}

pub struct BlockActionCtx<'a> {
    pub layout: &'a EditorLayout,
    pub row_rect: Rect,
}

pub trait BlockDecoration: std::fmt::Debug {
    fn height(&self, metrics: &DisplayLayoutMetrics) -> u16;

    fn paint(&self, _ctx: &mut BlockPaintCtx) {}

    fn on_click(&self) -> Option<Action> {
        None
    }

    fn on_click_at(&self, _ctx: &BlockActionCtx, _x: f32, _y: f32) -> Option<Action> {
        self.on_click()
    }

    fn context_menu_entries(&self) -> Option<Vec<ContextMenuEntry>> {
        None
    }

    fn accessibility_label(&self) -> Option<String> {
        None
    }

    /// Review-thread blocks return their thread + expanded state so the shell can
    /// render the card as a real `view!` element overlay (positioned at this block's
    /// reserved on-screen rect) instead of hand-painting it. Non-review blocks return
    /// `None`.
    fn review_card(&self) -> Option<(&crate::core::review::ReviewThread, bool)> {
        None
    }
}

#[derive(Debug, Default)]
pub struct BlockRegistry {
    blocks: Vec<(BlockPlacement, Box<dyn BlockDecoration>)>,
}

impl BlockRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.blocks.clear();
    }

    pub fn push(&mut self, placement: BlockPlacement, decoration: Box<dyn BlockDecoration>) {
        self.blocks.push((placement, decoration));
    }

    pub fn layout_signature(&self, metrics: DisplayLayoutMetrics) -> u64 {
        let mut sig = 0xcbf2_9ce4_8422_2325_u64;
        sig = mix_layout_signature(sig, self.blocks.len() as u64);
        for (placement, decoration) in &self.blocks {
            let (placement_tag, anchor) = match *placement {
                BlockPlacement::Above(anchor) => (1_u64, anchor),
                BlockPlacement::Below(anchor) => (2_u64, anchor),
            };
            sig = mix_layout_signature(sig, placement_tag);
            sig = mix_layout_signature(sig, u64::from(anchor));
            sig = mix_layout_signature(sig, u64::from(decoration.height(&metrics)));
        }
        sig
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&dyn BlockDecoration> {
        self.blocks.get(index).map(|(_, deco)| deco.as_ref())
    }

    pub fn placement(&self, index: usize) -> Option<BlockPlacement> {
        self.blocks.get(index).map(|(p, _)| *p)
    }

    pub fn indices_at(&self, placement: BlockPlacement) -> impl Iterator<Item = u16> + '_ {
        self.blocks
            .iter()
            .enumerate()
            .filter(move |(_, (p, _))| *p == placement)
            .map(|(i, _)| i as u16)
    }
}

fn mix_layout_signature(sig: u64, value: u64) -> u64 {
    sig ^ value.wrapping_add(0x9e37_79b9_7f4a_7c15).rotate_left(6) ^ (sig >> 2)
}

#[cfg(test)]
mod tests {
    use super::{BlockPlacement, RowDecoration, decoration_for_kind};
    use crate::ui::editor::display_layout::DisplayLayoutMetrics;
    use crate::ui::editor::render_doc::RenderRowKind;

    #[test]
    fn hunk_separator_and_file_header_have_decoration_body_rows_do_not() {
        assert!(decoration_for_kind(RenderRowKind::HunkSeparator).is_some());
        assert!(decoration_for_kind(RenderRowKind::FileHeader).is_some());
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
