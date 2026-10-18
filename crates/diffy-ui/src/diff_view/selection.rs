//! Selection and copy.
//!
//! Selection endpoints are [`SelectionPoint`]s whose block key holds the
//! side, the file's unit, and the line's store index, so a selection
//! survives scrolling, expanding, and other files changing. Rows order by
//! segment, then projection row.
//!
//! [`CopyContent`] names each copy policy. Selection copies represent the
//! rows the user sees: hidden lines, headers, metadata, annotations, and
//! the preview's count row never contribute, lines join with `\n`, and a
//! line limited to a prefix (`DiffLimits::shaped_line_bytes`, lowered by
//! the app) contributes only the bytes it shows. Exact
//! copies of a whole file, a whole line, or the patch are separate
//! policies that read the source.

use diffy_diff::{Mode, RowKind, Side, write_unified};
use quark::selection::{BlockKey, Selection, SelectionPoint};

use super::frame::{column_slot, floor_boundary, shown_len};
use super::navigation::{FileId, Revision};
use super::state::{RowRef, store_index};
use super::{CopySide, DiffViewState};

/// A row among all rows: segment position, then projection row.
pub(crate) type Pos = (u32, u32);

/// Both ends of a selection, start first: each a row and a byte.
pub(crate) type Ordered = ((Pos, usize), (Pos, usize));

/// What a copy puts on the clipboard. See the [module docs](self).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CopyContent {
    /// The selected rows' lines from one side: [`CopySide::New`] takes
    /// context and added lines, [`CopySide::Old`] context and removed
    /// lines, and [`CopySide::Shown`] each row's shown line (side by side,
    /// the side the selection was made on). An endpoint inside a line on
    /// the other side than the one copied takes that whole line, since its
    /// byte offset belongs to different text.
    Selection(CopySide),
    /// One side of a file byte for byte, line endings and a missing final
    /// newline included. Unavailable when the view holds only the
    /// patch's lines of it.
    WholeFile { file: FileId, side: Side },
    /// One complete source line without its line ending, however long
    /// (zero-based `line`).
    Line { file: FileId, side: Side, line: u32 },
    /// The whole diff as a unified patch.
    Patch,
}

const SIDE_BIT: u64 = 1 << 63;

/// The selection key of a line: side, file unit, and store index.
pub(crate) fn block_key(side: Side, unit: u32, index: u32) -> BlockKey {
    let side = if side == Side::New { SIDE_BIT } else { 0 };
    BlockKey(side | (u64::from(unit) << 32) | u64::from(index))
}

pub(crate) fn decode_key(key: BlockKey) -> (Side, u32, u32) {
    let side = if key.0 & SIDE_BIT != 0 {
        Side::New
    } else {
        Side::Old
    };
    (side, ((key.0 & !SIDE_BIT) >> 32) as u32, key.0 as u32)
}

impl DiffViewState {
    /// Copies `content`; `None` when the view cannot produce it exactly
    /// (an unknown file or line, or a whole file only part of which is
    /// loaded). An empty selection copies an empty string.
    pub fn copy(&self, content: CopyContent) -> Option<String> {
        match content {
            CopyContent::Selection(choice) => Some(self.copy_selection(self.chosen_side(choice))),
            CopyContent::WholeFile { file, side } => {
                let (seg, file) = self.locate(self.unit_of(file)?)?;
                let doc = &self.segments[seg].doc;
                if doc.files().partial[file as usize] {
                    return None;
                }
                Some(doc.text(file, side).as_str().to_owned())
            }
            CopyContent::Line { file, side, line } => {
                let (seg, file) = self.locate(self.unit_of(file)?)?;
                let doc = &self.segments[seg].doc;
                let index = store_index(doc, file, side, line)?;
                doc.text(file, side).display_line(index).map(str::to_owned)
            }
            CopyContent::Patch => Some(
                self.segments
                    .iter()
                    .map(|s| write_unified(&s.doc))
                    .collect(),
            ),
        }
    }

    /// The selected text: lines joined by `\n`, from the side the
    /// selection copies (see [`CopyContent::Selection`] with the style's
    /// [`CopySide`]). Empty without a selection.
    pub fn selected_text(&self) -> String {
        self.copy_selection(self.copy_side())
    }

    /// The selected rows' lines from `side` (`None` for each row's shown
    /// line).
    fn copy_selection(&self, side: Option<Side>) -> String {
        let mut out = String::new();
        let Some(s) = self.selection else {
            return out;
        };
        let Some(((start, start_byte), (end, end_byte))) = self.ordered_selection() else {
            return out;
        };
        // The side each endpoint's byte offset belongs to.
        let key_side = |point: SelectionPoint| decode_key(point.block).0;
        let (start_side, end_side) = if self.point_pos(s.anchor) <= self.point_pos(s.focus) {
            (key_side(s.anchor), key_side(s.focus))
        } else {
            (key_side(s.focus), key_side(s.anchor))
        };
        let mut first = true;
        for (seg, segment) in self.segments.iter().enumerate() {
            let seg = seg as u32;
            if seg < start.0 || seg > end.0 {
                continue;
            }
            let rows = if seg == start.0 { start.1 } else { 0 }..=if seg == end.0 {
                end.1
            } else {
                segment.projection.len().saturating_sub(1)
            };
            for row in rows {
                if segment.list_rows.index(row).is_none() {
                    continue;
                }
                let Some((side, index)) = self.copied_line(seg as usize, row, side) else {
                    continue;
                };
                let file = segment.projection.file[row as usize];
                let full = segment
                    .doc
                    .text(file, side)
                    .display_line(index)
                    .unwrap_or("");
                let text = &full[..shown_len(full, self.limits.shaped_line_bytes)];
                let kind = segment.projection.kind[row as usize];
                // An offset applies only to the text it was taken on.
                let applies = |key_side: Side| key_side == side || kind == RowKind::Context;
                let from = if (seg, row) == start && applies(start_side) {
                    floor_boundary(text, start_byte)
                } else {
                    0
                };
                let to = if (seg, row) == end && applies(end_side) {
                    floor_boundary(text, end_byte)
                } else {
                    text.len()
                };
                if !first {
                    out.push('\n');
                }
                first = false;
                out.push_str(&text[from..to.max(from)]);
            }
        }
        out
    }

    /// Which side the copy key reads: the selection's side by side, else
    /// the style's choice (`None` for lines as shown).
    fn copy_side(&self) -> Option<Side> {
        match self.mode {
            Mode::Split => Some(self.selection_side),
            Mode::Unified => self.chosen_side(self.style.copy_side),
        }
    }

    /// The side an explicit choice reads.
    fn chosen_side(&self, choice: CopySide) -> Option<Side> {
        match (choice, self.mode) {
            (CopySide::New, _) => Some(Side::New),
            (CopySide::Old, _) => Some(Side::Old),
            (CopySide::Shown, Mode::Split) => Some(self.selection_side),
            (CopySide::Shown, Mode::Unified) => None,
        }
    }

    /// The line of `row` that copy takes, or `None` when the row has none
    /// on that side.
    pub(crate) fn copied_line(
        &self,
        seg: usize,
        row: u32,
        side: Option<Side>,
    ) -> Option<(Side, u32)> {
        let p = &self.segments[seg].projection;
        let kind = p.kind[row as usize];
        let side = side.unwrap_or(if kind == RowKind::Removed {
            Side::Old
        } else {
            Side::New
        });
        p.line(row, side).map(|i| (side, i))
    }

    /// The line a point on `row` keys to: `side`'s line when side by
    /// side, else the line the row shows.
    pub(crate) fn shown_line(&self, seg: usize, row: u32, side: Side) -> Option<(Side, u32)> {
        let p = &self.segments[seg].projection;
        match p.mode {
            Mode::Split => p.line(row, side).map(|i| (side, i)),
            Mode::Unified => {
                let side = if p.kind[row as usize] == RowKind::Removed {
                    Side::Old
                } else {
                    Side::New
                };
                p.line(row, side).map(|i| (side, i))
            }
        }
    }

    /// The row of a selection point, when its line is shown.
    fn point_pos(&self, point: SelectionPoint) -> Option<Pos> {
        let (side, unit, index) = decode_key(point.block);
        let (seg, file) = self.locate(unit)?;
        let row = self.segments[seg].projection.row_of(file, side, index)?;
        Some((seg as u32, row))
    }

    /// Row and byte of each endpoint, start first. `None` without a
    /// selection or when an endpoint's line is not shown.
    pub(crate) fn ordered_selection(&self) -> Option<Ordered> {
        let s = self.selection?;
        let at = |point: SelectionPoint| Some((self.point_pos(point)?, point.byte));
        let (a, b) = (at(s.anchor)?, at(s.focus)?);
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    /// Selected byte range of `pos`'s line on `side`, as painted.
    pub(crate) fn row_selection(
        &self,
        ordered: Option<Ordered>,
        pos: Pos,
        side: Side,
        len: usize,
    ) -> Option<(usize, usize)> {
        let ((start, start_byte), (end, end_byte)) = ordered?;
        if pos < start || pos > end {
            return None;
        }
        // Only lines copy takes are highlighted, and side by side only on
        // the selection's side.
        self.copied_line(pos.0 as usize, pos.1, self.copy_side())?;
        if self.mode == Mode::Split && side != self.selection_side {
            return None;
        }
        let lo = if pos == start { start_byte.min(len) } else { 0 };
        let hi = if pos == end { end_byte.min(len) } else { len };
        (lo < hi).then_some((lo, hi))
    }

    /// Selects every line: the new side unless a side-by-side selection
    /// already chose one.
    pub fn select_all(&mut self) {
        let side = match self.mode {
            Mode::Split => self.selection_side,
            Mode::Unified => Side::New,
        };
        let point = |r: RowRef, byte: usize| {
            let RowRef::Line { seg, row } = r else {
                return None;
            };
            let (side, index) = self.shown_line(seg as usize, row, side)?;
            let segment = &self.segments[seg as usize];
            let unit = segment.unit(segment.projection.file[row as usize]);
            Some(SelectionPoint::new(block_key(side, unit, index), byte))
        };
        let first = self.refs.iter().find_map(|r| point(r, 0));
        let last = self.refs.iter().rev().find_map(|r| point(r, usize::MAX));
        if let (Some(first), Some(last)) = (first, last) {
            self.selection = Some(Selection::new(first, last));
            self.selection_side = side;
            self.revision += 1;
        }
    }

    pub(crate) fn local(&self, x: f32, y: f32) -> (f32, f32) {
        let b = self.bounds.get();
        (x - b.x, y - b.y)
    }

    /// The side a press at view-local `x` selects.
    pub(crate) fn side_at(&self, x: f32) -> Side {
        let m = self.metrics();
        let columns =
            super::prepared::Columns::new(self.mode, self.viewport.0, &m, &self.presentation);
        match self.mode {
            Mode::Unified => Side::New,
            Mode::Split if x < columns.of(Side::New).gutter_x => Side::Old,
            Mode::Split => Side::New,
        }
    }

    /// The selection point under view-local `(x, y)`, on the selection's
    /// side when side by side. Points above or below the window clamp to
    /// its first or last line; rows without a line on the side (headers,
    /// gaps, padding, metadata, annotations) give the start of the next
    /// line, or the end of the previous one at the bottom.
    pub fn point_at(&self, x: f32, y: f32) -> Option<SelectionPoint> {
        let frame = self.frame.as_ref()?;
        let side = self.selection_side;
        let rows = &frame.rows;
        let hit = rows
            .iter()
            .position(|r| y >= r.top && y < r.top + r.height)
            .unwrap_or(if y < rows.first()?.top {
                0
            } else {
                rows.len() - 1
            });
        let line_of = |r: &super::prepared::FrameRow| match self.refs.get(r.index as usize) {
            Some(RowRef::Line { seg, row }) => {
                let (side, index) = self.shown_line(seg as usize, row, side)?;
                Some((side, index, seg as usize, row))
            }
            _ => None,
        };
        let key_of = |seg: usize, row: u32, side: Side, index: u32| {
            let segment = &self.segments[seg];
            block_key(
                side,
                segment.unit(segment.projection.file[row as usize]),
                index,
            )
        };
        let row = &rows[hit];
        if let Some((line_side, index, seg, r)) = line_of(row) {
            let paint = row.paint.sides[line_side as usize].as_ref()?;
            let column = frame.columns.of(line_side);
            let scroll = if frame.wrap {
                0.0
            } else {
                self.hscroll[column_slot(frame.columns.mode, line_side)]
                    .offset_f64()
                    .0
            };
            // A long line's layout holds only its window. Its x and the
            // scroll can be hundreds of millions of points: subtract them
            // in f64, before the layout's own f32 coordinates.
            let (start, window_x) = paint.window.map_or((0, 0.0), |w| (w.start, w.x));
            let tx = f64::from(x - column.text_x - frame.metrics.text_pad) + scroll - window_x;
            let byte = if x < column.text_x {
                0
            } else {
                start + paint.layout.hit(tx as f32, y - row.top).get()
            };
            return Some(SelectionPoint::new(key_of(seg, r, line_side, index), byte));
        }
        if let Some((s, i, seg, r)) = rows[hit..].iter().find_map(line_of) {
            return Some(SelectionPoint::new(key_of(seg, r, s, i), 0));
        }
        let (s, i, seg, r) = rows[..hit].iter().rev().find_map(line_of)?;
        Some(SelectionPoint::new(key_of(seg, r, s, i), usize::MAX))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineHit {
    pub file: FileId,
    pub revision: Revision,
    pub side: Side,
    pub line: u32,
    pub kind: RowKind,
}

impl DiffViewState {
    pub fn line_at(&self, x: f32, y: f32) -> Option<LineHit> {
        let (x, y) = self.local(x, y);
        let frame = self.frame.as_ref()?;
        let row = frame
            .rows
            .iter()
            .find(|r| y >= r.top && y < r.top + r.height)?;
        let kind = row.paint.kind.diff().filter(|k| k.is_line())?;
        let side = match frame.columns.mode {
            Mode::Split => self.side_at(x),
            Mode::Unified if kind == RowKind::Removed => Side::Old,
            Mode::Unified => Side::New,
        };
        self.line_hit(
            row.paint.file,
            side,
            row.paint.source_lines[side as usize]?,
            kind,
        )
    }

    pub(crate) fn line_hit(
        &self,
        unit: u32,
        side: Side,
        line: u32,
        kind: RowKind,
    ) -> Option<LineHit> {
        let (seg, _) = self.locate(unit)?;
        Some(LineHit {
            file: self.file_id(unit),
            revision: Revision(self.segments[seg].revision),
            side,
            line,
            kind,
        })
    }
}
