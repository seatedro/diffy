//! The prepared frame: everything the renderer reads, built by the state's
//! `prepare` and shared with the cached build closures. The state owns
//! this schema; the renderer only reads it.
//!
//! # Coordinates
//!
//! - A source line is a zero-based line index of one side's file, never a
//!   projection row or a patch-store index. One-based display numbers are
//!   derived from it in exactly one place, [`RowPaint::numbers`].
//! - Byte ranges are half-open UTF-8 byte ranges into the text of the
//!   line's layout ([`LinePaint::layout`]), on char boundaries. That text
//!   is the source line without its line ending, or a prefix of it when
//!   [`LinePaint::detail`] says so, or for a long line the part of it
//!   [`LinePaint::window`] names (add its `start` for line bytes).
//! - Geometry is in logical points: `top` is relative to the viewport's top
//!   edge, x positions to the view's left edge.
//!
//! # Errors
//!
//! Preparing never fails. A line whose layout cannot be shaped is left out
//! of its row (the renderer shows that side as empty); ranges past the end
//! of the shown text are clipped to it.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use diffy_diff::{FileStatus, GapId, InlineDetail, Mode, RowKind, Side};
use diffy_syntax::HighlightKind;

use super::syntax::SyntaxStatus;
use quark_text::TextLayout;
use quark_ui::FocusId;
use quark_ui::element::{ScrollHandle, ScrollbarVisibility};

use super::presentation::{DiffAppearance, DiffMarkers, DiffNumbers, DiffPresentation};

/// Width of a [`DiffMarkers::Bars`] strip, in points.
pub const MARKER_BAR_W: f32 = 4.0;

/// Sizes derived from the style and the font, in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub font_size: f32,
    pub line_h: f32,
    pub char_w: f32,
    /// Width of one line number column.
    pub number_w: f32,
    /// Width of the `+`/`-` column.
    pub sign_w: f32,
    /// Space left of the text inside its column.
    pub text_pad: f32,
}

impl Metrics {
    /// Width of the change marker column under `markers`.
    pub fn marker_w(&self, markers: DiffMarkers) -> f32 {
        match markers {
            DiffMarkers::Signs => self.sign_w,
            DiffMarkers::Bars => MARKER_BAR_W,
            DiffMarkers::None => 0.0,
        }
    }
}

/// Where the gutters and text columns sit, from the left of the view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnBox {
    pub gutter_x: f32,
    pub gutter_w: f32,
    pub text_x: f32,
    pub text_w: f32,
}

/// The columns of one layout: side by side has one per side, unified one
/// shared column stored under [`Side::New`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Columns {
    pub mode: Mode,
    pub sides: [Option<ColumnBox>; 2],
}

impl Columns {
    /// Columns of a view `width` points wide. The gutter holds the number
    /// columns `presentation.numbers` asks for, then the marker column.
    pub fn new(mode: Mode, width: f32, m: &Metrics, presentation: &DiffPresentation) -> Self {
        let marker_w = m.marker_w(presentation.markers);
        match mode {
            Mode::Unified => {
                let numbers = match presentation.numbers {
                    DiffNumbers::Both => 2.0,
                    DiffNumbers::RelevantSide => 1.0,
                    DiffNumbers::None => 0.0,
                };
                let gutter_w = m.number_w * numbers + marker_w;
                let text = ColumnBox {
                    gutter_x: 0.0,
                    gutter_w,
                    text_x: gutter_w,
                    text_w: (width - gutter_w).max(1.0),
                };
                Self {
                    mode,
                    sides: [None, Some(text)],
                }
            }
            Mode::Split => {
                let numbers = match presentation.numbers {
                    DiffNumbers::Both | DiffNumbers::RelevantSide => 1.0,
                    DiffNumbers::None => 0.0,
                };
                let gutter_w = m.number_w * numbers + marker_w;
                let half = (width / 2.0).floor();
                let side = |x: f32, w: f32| ColumnBox {
                    gutter_x: x,
                    gutter_w,
                    text_x: x + gutter_w,
                    text_w: (w - gutter_w).max(1.0),
                };
                Self {
                    mode,
                    sides: [Some(side(0.0, half - 1.0)), Some(side(half, width - half))],
                }
            }
        }
    }

    /// The column showing `side` of a row.
    pub fn of(&self, side: Side) -> ColumnBox {
        match self.mode {
            Mode::Unified => self.sides[1].expect("unified column"),
            Mode::Split => self.sides[side as usize].expect("split column"),
        }
    }

    /// Wrap width of `side`'s text.
    pub fn wrap_width(&self, side: Side, m: &Metrics) -> f32 {
        (self.of(side).text_w - m.text_pad * 2.0).max(m.char_w)
    }
}

/// How much of a source line its layout holds: all of it, or (past
/// [`diffy_diff::DiffLimits::shaped_line_bytes`]) a prefix of whole
/// grapheme clusters. Copying the full line reads the source, never this.
pub use diffy_diff::LineDetail;

/// One side of a materialized line row.
#[derive(Debug)]
pub struct LinePaint {
    pub layout: Arc<TextLayout>,
    /// Highlight kind of each layout span.
    pub tones: Arc<[HighlightKind]>,
    /// Changed words, as byte ranges of the layout's text.
    pub words: Vec<Range<usize>>,
    /// Every word of the line changed: it is tinted as a changed line
    /// instead of painting `words`.
    pub whole: bool,
    pub detail: LineDetail,
    /// Whether `words` is the whole story for this line.
    pub word_detail: WordDetail,
    /// For a line too long to shape whole (without wrap), the part of it
    /// around where its column is scrolled, which is all the layout holds.
    pub window: Option<LineWindow>,
    pub struck: Vec<Range<usize>>,
    pub unspoken: Vec<Range<usize>>,
    pub renamed: Vec<Range<usize>>,
    pub removed_spans: Arc<[bool]>,
    pub merge: Option<Arc<LineMerge>>,
}

impl LinePaint {
    pub(crate) fn label(&self) -> String {
        let text = self.layout.text();
        if self.unspoken.is_empty() {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for range in &self.unspoken {
            out.push_str(&text[at.min(range.start)..range.start]);
            at = at.max(range.end);
        }
        out.push_str(&text[at..]);
        out
    }
}

#[derive(Debug, Default)]
pub struct LineMerge {
    pieces: Vec<(Range<usize>, Side, Range<usize>)>,
}

impl LineMerge {
    pub(crate) fn push(&mut self, layout: Range<usize>, side: Side, source: Range<usize>) {
        self.pieces.push((layout, side, source));
    }

    pub(crate) fn source_len(&self) -> usize {
        self.pieces
            .iter()
            .filter(|(_, side, _)| *side == Side::New)
            .map(|(_, _, source)| source.end)
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn to_layout(&self, side: Side, byte: usize) -> Option<usize> {
        self.pieces
            .iter()
            .filter(|(_, s, _)| *s == side)
            .find(|(_, _, source)| source.start <= byte && byte <= source.end)
            .map(|(layout, _, source)| layout.start + (byte - source.start))
    }

    pub(crate) fn to_source(&self, layout_byte: usize) -> usize {
        let mut last = 0;
        for (layout, side, source) in &self.pieces {
            if *side != Side::New {
                if layout_byte < layout.end {
                    return last;
                }
                continue;
            }
            if layout_byte <= layout.end {
                return source.start + layout_byte.saturating_sub(layout.start);
            }
            last = source.end;
        }
        last
    }
}

/// How complete a line's changed-word highlights are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordDetail {
    /// The line has no changed counterpart to compare with.
    Unpaired,
    /// A long pair's words are being computed off the UI thread; the row
    /// repaints when they land.
    Pending,
    /// The comparison's result: [`InlineDetail::Limited`] when a line
    /// exceeds `DiffLimits::inline_line_bytes`, which an app lowered.
    Done(InlineDetail),
}

/// The part of a long line a [`LinePaint`]'s layout holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineWindow {
    /// Bytes of the shown line the layout's text covers, half open, on
    /// grapheme boundaries.
    pub start: usize,
    pub end: usize,
    /// Where the layout's text starts, from the start of the line's text,
    /// in points: paint and hit-test the layout this far right. In `f64`,
    /// as a 64 MiB line runs to hundreds of millions of points.
    pub x: f64,
    /// Width of the whole shown line in points, estimated from column
    /// counts beyond the window.
    pub width: f64,
}

/// An app annotation's identity, unique within a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AnnotationId(pub u64);

/// A full-width annotation row below the code row of its anchor's last
/// line.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AnnotationSlot {
    pub id: AnnotationId,
    /// The side the anchor's lines are on.
    pub side: Side,
    /// The anchored source lines, half open.
    pub lines: Range<u32>,
    /// The anchor's lines changed since it was attached; the app should
    /// offer reattachment rather than trust the position.
    pub outdated: bool,
    /// The app's content revision; a new one invalidates the measured
    /// height.
    pub revision: u64,
}

/// A fact about a file that its line rows do not show.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FileFact {
    /// Binary content; the text view shows no lines for it.
    Binary,
    /// The file mode changed, e.g. `100644` to `100755`.
    ModeChange { old: Arc<str>, new: Arc<str> },
    /// Renamed with no text changes.
    RenameOnly,
    /// Copied with no text changes.
    CopyOnly,
    /// The side's last line has no line ending.
    NoNewlineAtEof(Side),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MoveNote {
    pub index: u32,
    pub here: bool,
    pub up: Option<bool>,
}

/// What a prepared row shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PreparedKind {
    /// A projection row: a header, a gap, or lines.
    Diff(RowKind),
    /// A metadata row under its file's header.
    Fact(FileFact),
    /// A slot the app's decorator fills.
    Annotation(AnnotationSlot),
    /// The last row of a bounded preview, standing for `hidden_rows`
    /// logical rows left out; it carries the open-full-diff control.
    More { hidden_rows: u32 },
    Pending {
        state: diffy_diff::SlotState,
        estimated_lines: u32,
    },
}

impl PreparedKind {
    /// The projection row kind, for rows of the projection.
    pub fn diff(&self) -> Option<RowKind> {
        match self {
            Self::Diff(kind) => Some(*kind),
            _ => None,
        }
    }

    /// Whether the row shows lines of the file.
    pub fn is_line(&self) -> bool {
        self.diff().is_some_and(RowKind::is_line)
    }
}

/// A materialized row: its lines or its header text. Kept while the row
/// stays in the window and its inputs (`stamp`) stay the same.
#[derive(Debug)]
pub struct RowPaint {
    pub stamp: u64,
    pub kind: PreparedKind,
    /// The document's file index.
    pub file: u32,
    pub sides: [Option<LinePaint>; 2],
    /// Zero-based source line on each side, when the row has one there.
    pub source_lines: [Option<u32>; 2],
    /// File, hunk, or gap header text.
    pub title: Arc<str>,
    pub gap: Option<GapId>,
    /// Gap rows: how many lines the gap hides.
    pub hidden: u32,
    pub status: FileStatus,
    pub stats: (u32, u32),
    pub binary: bool,
    /// File headers: the file's syntax status, when highlighting is on.
    /// Paint its label while it is not [`SyntaxStatus::Ready`].
    pub syntax: Option<SyntaxStatus>,
    /// File headers: the file's rows are folded under the header.
    pub collapsed: bool,
    /// File headers: annotations anchored in the file, shown or not.
    pub annotations: u32,
    pub quiet: bool,
    pub fold: Option<(u32, bool)>,
    pub hint: Option<Arc<str>>,
    pub note: Option<MoveNote>,
    pub summary: Option<Arc<str>>,
}

impl RowPaint {
    /// One-based display number on each side, or zero where the row has
    /// no line on that side.
    pub fn numbers(&self) -> [u32; 2] {
        self.source_lines
            .map(|line| line.map_or(0, |l| l.saturating_add(1)))
    }

    /// Height of the row: its kind's, or its tallest wrapped line's.
    pub fn height(&self, m: &Metrics, presentation: &DiffPresentation) -> f32 {
        let lines = self.sides.iter().flatten();
        lines.map(|l| l.layout.size().1.ceil()).fold(
            self.kind
                .diff()
                .map_or(m.line_h, |k| row_height(k, m, presentation)),
            f32::max,
        )
    }
}

/// Height of a row of `kind` before its text is measured.
pub fn row_height(kind: RowKind, m: &Metrics, presentation: &DiffPresentation) -> f32 {
    super::presentation::band_height(kind, m, presentation)
}

/// A search match on one side of a row.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SearchMark {
    /// Byte range of the layout's text.
    pub range: Range<usize>,
    /// The match next/previous last moved to.
    pub active: bool,
}

/// A row in the frame the view paints.
#[derive(Debug, Clone)]
pub struct FrameRow {
    pub key: u64,
    /// Position among all rows, for accessibility.
    pub index: u32,
    pub top: f32,
    pub height: f32,
    pub paint: Rc<RowPaint>,
    /// Selected byte range of each side's line.
    pub selected: [Option<(usize, usize)>; 2],
    /// Search matches on each side's line, in order.
    pub search: [Vec<SearchMark>; 2],
    /// The row holds the keyboard focus target (the row a navigation key
    /// or search result last moved to).
    pub focused: bool,
}

/// How many logical rows a compact preview may show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiffPreviewLimit {
    pub max_rows: u32,
}

impl Default for DiffPreviewLimit {
    fn default() -> Self {
        Self { max_rows: 12 }
    }
}

/// Everything the view reads, built by the state's `prepare`.
#[derive(Debug)]
pub struct ViewFrame {
    pub id: &'static str,
    pub label: &'static str,
    pub focus: FocusId,
    pub viewport: (f32, f32),
    pub metrics: Metrics,
    pub columns: Columns,
    pub presentation: DiffPresentation,
    pub appearance: DiffAppearance,
    pub wrap: bool,
    pub rows: Vec<FrameRow>,
    /// With sticky headers on: the header of the file the top row belongs
    /// to, once its own row has scrolled above the viewport.
    pub sticky_header: Option<FrameRow>,
    /// Width of each side's widest line seen, padding included.
    pub content_w: [f64; 2],
    /// Where each side's sideways scrolling content is laid out from: near
    /// its scroll, so long lines paint at exact `f32` positions; see
    /// `Div::scroll_origin`.
    pub content_origin: [f64; 2],
    /// The vertical scroll offset and the height of every row, in `f64`
    /// so offsets deep into millions of rows stay exact. Row tops are
    /// already relative to `scroll`.
    pub scroll: f64,
    pub total: f64,
    pub row_count: u32,
    pub hscroll: [ScrollHandle; 2],
    pub scrollbar_auto_hide: bool,
    pub scrollbar: ScrollbarVisibility,
}
