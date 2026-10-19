//! Row chrome painted straight into the scene: change markers, the hatch
//! of a missing split side, the dividers, and a line's decoration layers
//! in their fixed order. Each is a handful of primitives pushed from one
//! canvas, never an element per stripe, so the renderer batches them with
//! the rest of the row's rectangles.
//!
//! Geometry is in logical points. Lengths that must land on whole device
//! pixels (stripe periods, hairlines) are computed in physical pixels at
//! the frame's scale and converted back, so the renderer's edge snapping
//! keeps them even at fractional scales.

use std::ops::Range;
use std::sync::Arc;

use quark::path::{Path, StrokeStyle};
use quark_render::scene::{PathPrimitive, Rect, RichTextPrimitive, ShapedText};
use quark_render::{BorderPrimitive, RectPrimitive, RoundedRectPrimitive, Scene};
use quark_text::TextLayout;
use quark_ui::theme::Color;

use super::prepared::SearchMark;

/// Width of a selected newline, in ems.
const NEWLINE_EM: f32 = 0.5;

/// Distance between hatch lines, in points.
pub(super) const HATCH_SPACING: f32 = 6.0;

/// `points` rounded to whole physical pixels at `scale`, at least one.
pub(super) fn whole_pixels(points: f32, scale: f32) -> f32 {
    (points * scale).round().max(1.0) / scale
}

/// The change a marker shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Cue {
    Added,
    Removed,
}

/// Diagonal hatch lines over `rect`, clipped to it. `phase` is the
/// rect's top in document coordinates, so the hatches of stacked rows
/// continue each other wherever the rows scroll.
pub(super) fn hatch(scene: &mut Scene, rect: Rect, phase: f32, color: Color, scale: f32) {
    let spacing = HATCH_SPACING;
    // Lines run up and to the right: x + y is constant along each, so the
    // first one starts where (x + document y) is a multiple of `spacing`.
    let mut builder = Path::builder();
    let start = -phase.rem_euclid(spacing);
    let mut c = start;
    while c < rect.width + rect.height {
        builder.move_to(c - rect.height, rect.height);
        builder.line_to(c, 0.0);
        c += spacing;
    }
    let path = builder.build();
    if path.is_empty() {
        return;
    }
    scene.clip(rect);
    scene.path(
        PathPrimitive::new(Arc::new(path), [rect.x, rect.y])
            .stroke(color, StrokeStyle::new(1.0 / scale)),
    );
    scene.pop_clip();
}

/// `ranges` with runs separated only by whitespace in `text` joined, so a
/// changed phrase paints as one block instead of one per token.
pub(super) fn join_whitespace_gaps(ranges: Vec<Range<usize>>, text: &str) -> Vec<Range<usize>> {
    let mut joined: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = joined.last_mut()
            && range.start >= last.end
            && text
                .get(last.end..range.start)
                .is_some_and(|gap| gap.chars().all(char::is_whitespace))
        {
            last.end = last.end.max(range.end);
            continue;
        }
        joined.push(range);
    }
    joined
}

/// A vertical hairline one physical pixel wide at `x`.
pub(super) fn vertical_hairline(
    scene: &mut Scene,
    x: f32,
    y: f32,
    height: f32,
    color: Color,
    scale: f32,
) {
    scene.rect(RectPrimitive {
        rect: Rect {
            x,
            y,
            width: 1.0 / scale,
            height,
        },
        color,
    });
}

/// Colors of a line's decoration layers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LineColors {
    pub text: Color,
    pub word: Color,
    pub search: Color,
    pub search_active: Color,
    pub search_outline: Color,
    pub selection: Color,
}

/// A shaped line and its decorations, painted in the fixed order: changed
/// words, search matches, selection, then the glyphs. Search matches also
/// get an outline, so a match over a changed word or inside a selection
/// stays visible where the fills stack. (The row's own fill comes before
/// all of this, and a focus outline after.)
#[allow(clippy::too_many_arguments)]
pub(super) fn line(
    scene: &mut Scene,
    origin: (f32, f32),
    layout: &Arc<TextLayout>,
    span_colors: Arc<[Color]>,
    words: &[Range<usize>],
    search: &[SearchMark],
    selected: Option<(usize, usize)>,
    colors: LineColors,
    row_height: f32,
    font_size: f32,
    scale: f32,
) {
    let rects = |range: Range<usize>| {
        layout.selection_rects(range).map(move |r| Rect {
            x: origin.0 + r.x,
            y: origin.1 + r.y,
            width: r.width.max(1.0),
            height: r.height,
        })
    };
    for range in words {
        for rect in rects(range.clone()) {
            scene.rounded_rect(RoundedRectPrimitive::uniform(rect, 2.0, colors.word));
        }
    }
    for mark in search {
        let fill = if mark.active {
            colors.search_active
        } else {
            colors.search
        };
        let outline = whole_pixels(if mark.active { 2.0 } else { 1.0 }, scale);
        for rect in rects(mark.range.clone()) {
            scene.rounded_rect(RoundedRectPrimitive::uniform(rect, 2.0, fill));
            scene.border(BorderPrimitive::uniform(
                rect,
                outline,
                2.0,
                colors.search_outline,
            ));
        }
    }
    if let Some((lo, hi)) = selected {
        // Square and stretched to the row's edges, so the rects of stacked
        // rows and wrapped lines meet as one block. A selected newline
        // (`hi` past the text) shows as a sliver after the last character.
        let len = layout.text().len();
        let (top, bottom) = (origin.1, origin.1 + row_height.max(layout.size().1));
        let mut spans: Vec<Rect> = if lo < hi.min(len) {
            rects(lo..hi.min(len)).collect()
        } else {
            Vec::new()
        };
        if hi > len {
            let caret = layout.caret(len);
            spans.push(Rect {
                x: origin.0 + caret.x,
                y: origin.1 + caret.y,
                width: font_size * NEWLINE_EM,
                height: caret.height,
            });
        }
        let first = spans.iter().map(|r| r.y).fold(f32::INFINITY, f32::min);
        let last = spans.iter().map(|r| r.y).fold(f32::NEG_INFINITY, f32::max);
        for rect in spans {
            let y = if rect.y == first { top } else { rect.y };
            let end = if rect.y == last {
                bottom
            } else {
                rect.y + rect.height
            };
            scene.rect(RectPrimitive {
                rect: Rect {
                    y,
                    height: end - y,
                    ..rect
                },
                color: colors.selection,
            });
        }
    }
    let (w, h) = layout.size();
    scene.rich_text(RichTextPrimitive {
        rect: Rect {
            x: origin.0,
            y: origin.1,
            // Italic and wide glyphs ink past their advance.
            width: w + font_size,
            height: h.max(1.0),
        },
        layout: ShapedText::new(layout.clone()),
        default_color: colors.text,
        span_colors,
    });
}

/// The keyboard focus outline of a row, inset so neighbors do not cover
/// it.
pub(super) fn focus_outline(scene: &mut Scene, rect: Rect, color: Color, scale: f32) {
    scene.border(BorderPrimitive::uniform(
        rect,
        whole_pixels(2.0, scale),
        0.0,
        color,
    ));
}

#[cfg(test)]
mod tests {
    use quark_render::Primitive;
    use quark_render::scene::Scene;
    use quark_text::{LayoutCache, TextParams, TextStyle, TextSystem};

    use super::*;

    fn color(n: u8) -> Color {
        Color::rgba(n, n, n, 255)
    }

    // Catches decoration layers stacking in the wrong order, which hides
    // a match under a changed word or text under a fill: changed words,
    // then the search match and its outline, then the selection, then
    // the glyphs.
    #[test]
    fn a_line_paints_words_then_search_then_selection_then_text() {
        let mut text = TextSystem::vendored_only(&Default::default());
        let mut layouts = LayoutCache::default();
        let layout = layouts
            .layout(
                &mut text,
                &TextParams::new("let total = 1;", TextStyle::new(13.0)),
            )
            .unwrap();
        let colors = LineColors {
            text: color(1),
            word: color(2),
            search: color(3),
            search_active: color(4),
            search_outline: color(5),
            selection: color(6),
        };
        let mut scene = Scene::default();
        let words = vec![4..9, 11..12];
        line(
            &mut scene,
            (0.0, 0.0),
            &layout,
            Arc::from([]),
            &words,
            &[SearchMark {
                range: 4..9,
                active: true,
            }],
            Some((0, 9)),
            colors,
            22.0,
            13.0,
            1.0,
        );

        let order: Vec<String> = scene
            .primitives
            .iter()
            .map(|p| match p {
                Primitive::RoundedRect(r) => format!("fill {}", r.color.r),
                Primitive::Rect(r) => format!("fill {}", r.color.r),
                Primitive::Border(b) => format!("outline {} {}", b.color.r, b.widths[0]),
                Primitive::RichTextRun(_) => "text".to_owned(),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            order,
            [
                "fill 2",
                "fill 2",
                "fill 4",
                "outline 5 2",
                "fill 6",
                "text"
            ]
        );
    }

    #[test]
    fn changed_words_split_only_by_whitespace_join() {
        let text = "f, ready: bool) x";
        assert_eq!(
            join_whitespace_gaps(vec![0..2, 3..9, 10..14, 16..17], text),
            vec![0..14, 16..17]
        );
    }
}
