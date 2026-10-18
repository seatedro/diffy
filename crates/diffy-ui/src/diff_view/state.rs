//! The row model under both views: segments of the diff and the flat row
//! table over them.
//!
//! A segment is one document with its own expansion and projection: the
//! whole document for the static view, one file revision for a session
//! view. Rows of every segment, plus metadata, annotation, and preview
//! rows, sit in one [`VariableList`] in display order. Changing a segment
//! re-projects only that segment; the flat row table is then rebuilt from
//! the segments' projections, which costs a pass over row keys but no
//! diffing, projection, or shaping of the other segments.
//!
//! A file is named inside the view by its unit: the document's file index
//! in the static view, a stable slot per [`super::FileId`] in a session.
//! Row keys, selection keys, and paint caches carry the unit, so they stay
//! valid while other files change around them.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use diffy_diff::{
    BlockKind, Comparison, ComparisonOptions, ContextPolicy, DiffDocument, Expansion,
    InlineOptions, LinePair, Mode, PairedInlineDiff, Projection, RowKind, Side, SlotState,
    paired_inline_diff,
};
use quark_ui::virtual_list::{RowTable, VariableList};

use super::DiffViewState;
use super::prepared::{Metrics, row_height};
use super::prepared_update::file_facts;

/// No row, slot, or segment.
pub(crate) const NONE: u32 = u32::MAX;

const UNIT_SHIFT: u32 = 38;
const UNIT_MASK: u64 = 0x3F_FFFF;
const FACT_TAG: u64 = 8 << 60;
const ANNOTATION_TAG: u64 = 9 << 60;
const MORE_TAG: u64 = 10 << 60;

/// Inline results kept per segment before the cache starts over, so
/// scrolling a huge diff does not grow it without bound.
const INLINE_CACHE_PAIRS: usize = 4_096;

/// One document of the view, with its own expansion, comparison, and
/// projection.
#[derive(Debug)]
pub(crate) struct Segment {
    pub doc: Arc<DiffDocument>,
    /// Unit of the document's first file.
    pub slot: u32,
    /// Distinguishes this document from every other the view has shown,
    /// for paint stamps.
    pub generation: u64,
    /// The content revision anchors attach to.
    pub revision: u64,
    pub expansion: Expansion,
    /// Which removed line sits beside which added one, and which changes
    /// the whitespace policy hides; computed once per document and
    /// options.
    pub comparison: Comparison,
    pub projection: Projection,
    pub alternate: Option<(Mode, Projection)>,
    pub pending: Option<(SlotState, u32)>,
    /// Where each projection row sits in the list, if it is shown.
    pub list_rows: ListRows,
    /// Changed ranges of each line pair shown, computed once for both
    /// unified rows of the pair (and the split row) while preparing.
    inline: RefCell<HashMap<LinePair, Rc<PairedInlineDiff>>>,
}

impl Segment {
    pub fn new(
        doc: Arc<DiffDocument>,
        slot: u32,
        generation: u64,
        mode: Mode,
        policy: ContextPolicy,
        options: ComparisonOptions,
    ) -> Self {
        let expansion = Expansion::with_policy(&doc, policy);
        let comparison = Comparison::new(&doc, options);
        let projection = Projection::with_comparison(&doc, mode, &expansion, &comparison);
        Self {
            doc,
            slot,
            generation,
            revision: 0,
            expansion,
            comparison,
            projection,
            alternate: None,
            pending: None,
            list_rows: ListRows::default(),
            inline: RefCell::default(),
        }
    }

    pub fn from_parts(
        doc: Arc<DiffDocument>,
        slot: u32,
        generation: u64,
        expansion: Expansion,
        comparison: Comparison,
        projection: Projection,
        alternate: Option<(Mode, Projection)>,
    ) -> Self {
        Self {
            doc,
            slot,
            generation,
            revision: 0,
            expansion,
            comparison,
            projection,
            alternate,
            pending: None,
            list_rows: ListRows::default(),
            inline: RefCell::default(),
        }
    }

    pub fn switch(&mut self, from: Mode, to: Mode) {
        match self.alternate.take() {
            Some((mode, projection)) if mode == to => {
                let previous = std::mem::replace(&mut self.projection, projection);
                self.alternate = Some((from, previous));
            }
            _ => self.rebuild(to),
        }
    }

    pub fn unit(&self, file: u32) -> u32 {
        self.slot + file
    }

    /// Re-projects after an expansion or mode change.
    pub fn rebuild(&mut self, mode: Mode) {
        self.alternate = None;
        self.projection
            .rebuild_with(&self.doc, mode, &self.expansion, &self.comparison);
    }

    /// Compares the document again under `options` and re-projects.
    pub fn compare(&mut self, mode: Mode, options: ComparisonOptions) {
        if self.comparison.options() != options {
            self.comparison = Comparison::new(&self.doc, options);
            self.inline.get_mut().clear();
            self.rebuild(mode);
        }
    }

    /// Forgets inline results, after the inline options changed.
    pub fn clear_inline(&mut self) {
        self.inline.get_mut().clear();
    }

    /// The changed ranges of `pair`, computed on first use.
    pub fn inline_diff(&self, pair: LinePair, options: &InlineOptions) -> Rc<PairedInlineDiff> {
        if let Some(hit) = self.inline.borrow().get(&pair) {
            return hit.clone();
        }
        let line = |side, i| self.doc.text(pair.file, side).display_line(i).unwrap_or("");
        let diff = Rc::new(paired_inline_diff(
            line(Side::Old, pair.old),
            line(Side::New, pair.new),
            options,
        ));
        let mut cache = self.inline.borrow_mut();
        if cache.len() >= INLINE_CACHE_PAIRS {
            cache.clear();
        }
        cache.insert(pair, diff.clone());
        diff
    }

    /// The row's line on `side` as `(unit, store index)`.
    pub fn line(&self, row: u32, side: Side) -> Option<(u32, u32)> {
        let file = self.projection.file[row as usize];
        Some((self.unit(file), self.projection.line(row, side)?))
    }
}

/// Which segment shows each session file. Empty for the static view.
#[derive(Debug, Default)]
pub(crate) struct FileMap {
    /// Segment position of each slot, or [`NONE`].
    pub slot_seg: Vec<u32>,
    pub slot_ids: Vec<super::FileId>,
    pub id_slots: HashMap<super::FileId, u32>,
    /// Session mode, even while it holds no files.
    pub session: bool,
    pub solo: Option<u32>,
}

impl FileMap {
    pub fn is_static(&self) -> bool {
        !self.session
    }
}

/// What a row of the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowRef {
    Line { seg: u32, row: u32 },
    Fact { seg: u32, file: u32, fact: u32 },
    Annotation { index: u32 },
    More { hidden: u32 },
}

/// What each row of the list shows, in runs: consecutive projection rows
/// of one segment are one entry, so a diff of millions of lines between a
/// few headers costs a few entries rather than one per row.
#[derive(Debug, Default)]
pub(crate) struct RowRefs {
    /// List index each run starts at, ascending from 0.
    starts: Vec<u32>,
    /// A line run names its first row; every other run is one row.
    runs: Vec<RowRef>,
    len: u32,
}

impl RowRefs {
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Rows in run `k`.
    fn run_len(&self, k: usize) -> u32 {
        self.starts.get(k + 1).copied().unwrap_or(self.len) - self.starts[k]
    }

    /// Appends `r`, extending the last run when it continues it, and
    /// returns its list index.
    fn push(&mut self, r: RowRef) -> u32 {
        let index = self.len;
        let continues = match (self.runs.last(), r) {
            (Some(&RowRef::Line { seg, row }), RowRef::Line { seg: s, row: next }) => {
                let k = self.runs.len() - 1;
                seg == s && row.checked_add(self.run_len(k)) == Some(next)
            }
            _ => false,
        };
        if !continues {
            self.starts.push(index);
            self.runs.push(r);
        }
        self.len += 1;
        index
    }

    /// Appends rows `rows` of segment `seg` as one run.
    fn push_lines(&mut self, seg: u32, rows: Range<u32>) {
        if let Some(first) = rows.clone().next() {
            self.push(RowRef::Line { seg, row: first });
            self.len += rows.len() as u32 - 1;
        }
    }

    /// What list row `index` shows. O(log runs).
    pub fn get(&self, index: usize) -> Option<RowRef> {
        let index = u32::try_from(index).ok().filter(|&i| i < self.len)?;
        let k = self.starts.partition_point(|&s| s <= index) - 1;
        Some(match self.runs[k] {
            RowRef::Line { seg, row } => RowRef::Line {
                seg,
                row: row + (index - self.starts[k]),
            },
            r => r,
        })
    }

    /// [`Self::get`] for an index known to be in range.
    pub fn at(&self, index: usize) -> RowRef {
        self.get(index)
            .unwrap_or_else(|| panic!("row {index} of {}", self.len))
    }

    /// Every row in list order, run by run.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = RowRef> + '_ {
        self.runs.iter().enumerate().flat_map(move |(k, &r)| {
            (0..self.run_len(k)).map(move |j| match r {
                RowRef::Line { seg, row } => RowRef::Line { seg, row: row + j },
                r => r,
            })
        })
    }

    /// Rows `range` in list order.
    pub fn range(&self, range: Range<usize>) -> impl DoubleEndedIterator<Item = RowRef> + '_ {
        range.map(|i| self.at(i))
    }
}

/// Builds [`RowRefs`], noting where each annotation's row lands.
#[derive(Default)]
struct RefsBuilder {
    refs: RowRefs,
    /// List index of each annotation entry's row, or [`NONE`].
    annotation_rows: Vec<u32>,
}

impl RefsBuilder {
    fn push(&mut self, r: RowRef) -> u32 {
        let index = self.refs.push(r);
        if let RowRef::Annotation { index: entry } = r {
            let entry = entry as usize;
            if self.annotation_rows.len() <= entry {
                self.annotation_rows.resize(entry + 1, NONE);
            }
            self.annotation_rows[entry] = index;
        }
        index
    }

    /// The rows under `file`'s header: its metadata, then the annotations
    /// that lost their lines.
    fn file_extras(
        &mut self,
        doc: &DiffDocument,
        outdated: Option<&Vec<u32>>,
        seg: u32,
        file: u32,
    ) {
        for fact in 0..file_facts(doc, file).len() as u32 {
            self.push(RowRef::Fact { seg, file, fact });
        }
        for &index in outdated.into_iter().flatten() {
            self.push(RowRef::Annotation { index });
        }
    }
}

/// Where a segment's projection rows sit in the list, in runs of rows
/// shown one after another; rows a collapsed file or a preview leaves out
/// have no place.
#[derive(Debug, Default)]
pub(crate) struct ListRows {
    /// `(first row, its list index)` of each run, ascending.
    runs: Vec<(u32, u32)>,
    /// Rows in each run.
    lens: Vec<u32>,
}

impl ListRows {
    /// Records that `count` rows from `row` on sit from list index `index`.
    fn push(&mut self, row: u32, index: u32, count: u32) {
        if count == 0 {
            return;
        }
        if let (Some(&(r, i)), Some(len)) = (self.runs.last(), self.lens.last_mut())
            && r + *len == row
            && i + *len == index
        {
            *len += count;
            return;
        }
        self.runs.push((row, index));
        self.lens.push(count);
    }

    /// The list index of projection row `row`, unless it is left out.
    /// O(log runs).
    pub fn index(&self, row: u32) -> Option<u32> {
        let k = self
            .runs
            .partition_point(|&(r, _)| r <= row)
            .checked_sub(1)?;
        let (first, index) = self.runs[k];
        (row - first < self.lens[k]).then(|| index + (row - first))
    }

    /// The first of `rows` projection rows left out of the list.
    pub fn first_hidden(&self, rows: u32) -> Option<u32> {
        let mut next = 0;
        for (&(row, _), &len) in self.runs.iter().zip(&self.lens) {
            if row > next {
                return Some(next);
            }
            next = row + len;
        }
        (next < rows).then_some(next)
    }
}

/// The scroll position to keep while rows change: the top row's key and
/// how far into it the view is scrolled, and the source line it shows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Anchor {
    pub key: u64,
    /// Its list index, where a search for the key starts.
    pub index: u32,
    pub delta: f64,
    /// `(unit, side, store index)`.
    pub line: Option<(u32, Side, u32)>,
}

/// Zero-based source line of store line `index` of `file`'s `side`.
pub(crate) fn source_line(doc: &DiffDocument, file: u32, side: Side, index: u32) -> u32 {
    if !doc.files().partial[file as usize] {
        return index;
    }
    let (h, b) = (doc.hunks(), doc.blocks());
    for hunk in doc.files().hunks[file as usize].clone() {
        for block in h.blocks[hunk as usize].clone() {
            let bi = block as usize;
            let (store, len, line) = match side {
                Side::Old => (b.old_store[bi], b.old_len[bi], b.old_line[bi]),
                Side::New => (b.new_store[bi], b.new_len[bi], b.new_line[bi]),
            };
            if (store..store + len).contains(&index) {
                debug_assert!(b.kind[bi] == BlockKind::Context || len > 0);
                return (line + index - store).saturating_sub(1);
            }
        }
    }
    index
}

/// Store index of zero-based source `line` of `file`'s `side`, when the
/// store holds it.
pub(crate) fn store_index(doc: &DiffDocument, file: u32, side: Side, line: u32) -> Option<u32> {
    let store = doc.text(file, side);
    if !doc.files().partial[file as usize] {
        return (line < store.line_count()).then_some(line);
    }
    let (h, b) = (doc.hunks(), doc.blocks());
    for hunk in doc.files().hunks[file as usize].clone() {
        for block in h.blocks[hunk as usize].clone() {
            let bi = block as usize;
            let (first, len, start) = match side {
                Side::Old => (b.old_store[bi], b.old_len[bi], b.old_line[bi]),
                Side::New => (b.new_store[bi], b.new_len[bi], b.new_line[bi]),
            };
            let one_based = line + 1;
            if len > 0 && (start..start + len).contains(&one_based) {
                return Some(first + one_based - start);
            }
        }
    }
    None
}

impl DiffViewState {
    // ---- Units and keys ------------------------------------------------

    /// Segment position and file index within it of `unit`.
    pub(crate) fn locate(&self, unit: u32) -> Option<(usize, u32)> {
        if self.files.is_static() {
            let doc = &self.segments.first()?.doc;
            return (unit < doc.file_count()).then_some((0, unit));
        }
        let seg = *self.files.slot_seg.get(unit as usize)?;
        (seg != NONE).then_some((seg as usize, 0))
    }

    /// Row key of projection row `row` of segment `seg`: the projection's
    /// key with the file replaced by its unit.
    pub(crate) fn line_key(&self, seg: usize, row: u32) -> u64 {
        let segment = &self.segments[seg];
        let key = segment.projection.row_key(row);
        let file = (key >> UNIT_SHIFT) & UNIT_MASK;
        let unit = (u64::from(segment.slot) + file) & UNIT_MASK;
        (key & !(UNIT_MASK << UNIT_SHIFT)) | (unit << UNIT_SHIFT)
    }

    pub(crate) fn ref_key(&self, r: RowRef) -> u64 {
        match r {
            RowRef::Line { seg, row } => self.line_key(seg as usize, row),
            RowRef::Fact { seg, file, fact } => {
                let unit = u64::from(self.segments[seg as usize].unit(file)) & UNIT_MASK;
                FACT_TAG | (unit << UNIT_SHIFT) | u64::from(fact)
            }
            RowRef::Annotation { index } => {
                ANNOTATION_TAG | (self.annotations.entry(index).annotation.id.0 & ((1 << 60) - 1))
            }
            RowRef::More { .. } => MORE_TAG,
        }
    }

    /// The projection row a list row belongs to, for the static view.
    pub(crate) fn projection_row_of(&self, index: usize) -> Option<u32> {
        let segment = self.segments.first()?;
        match self.refs.get(index)? {
            RowRef::Line { row, .. } => Some(row),
            RowRef::Fact { file, .. } => segment.projection.file_rows.get(file as usize).copied(),
            RowRef::Annotation { .. } | RowRef::More { .. } => {
                self.refs.range(0..index).rev().find_map(|r| match r {
                    RowRef::Line { row, .. } => Some(row),
                    _ => None,
                })
            }
        }
    }

    // ---- Rows ----------------------------------------------------------

    /// The first visible row, how far into it the view is scrolled, and
    /// the source line it shows, to restore after the rows change.
    pub(crate) fn anchor(&self) -> Option<Anchor> {
        let rows = self.list.rows();
        let index = rows.row_at(self.list.scroll_offset())?;
        let line = match self.refs.get(index) {
            Some(RowRef::Line { seg, row }) => {
                let segment = &self.segments[seg as usize];
                [Side::New, Side::Old]
                    .into_iter()
                    .find_map(|side| segment.line(row, side).map(|(u, i)| (u, side, i)))
            }
            _ => None,
        };
        Some(Anchor {
            key: self.ref_key(self.refs.at(index)),
            index: index as u32,
            delta: self.list.scroll_offset() - rows.offset_of_index(index),
            line,
        })
    }

    /// List index of the row showing store line `index` of `unit`'s `side`.
    pub(crate) fn list_index_of_line(&self, unit: u32, side: Side, index: u32) -> Option<u32> {
        let (seg, file) = self.locate(unit)?;
        let segment = &self.segments[seg];
        let row = segment.projection.row_of(file, side, index)?;
        segment.list_rows.index(row)
    }

    /// List index of the row keyed `key`, if it is shown. Line, file
    /// header, metadata, annotation, and preview rows are found from the
    /// key's parts; hunk header and gap rows by a pass over the rows
    /// outward from list index `near`, where it was before the rows
    /// changed.
    pub(crate) fn index_of_key(&self, key: u64, near: u32) -> Option<u32> {
        let unit = ((key >> UNIT_SHIFT) & UNIT_MASK) as u32;
        let index = (key & 0x3F_FFFF_FFFF) as u32;
        // The list index of `unit`'s header (no `side`) or of the row
        // showing line `index` of `side`.
        let row_of = |side: Option<Side>| {
            let (seg, file) = self.locate(unit)?;
            let segment = &self.segments[seg];
            let p = &segment.projection;
            let row = match side {
                Some(side) => p.row_of(file, side, index)?,
                None => *p.file_rows.get(file as usize)?,
            };
            segment.list_rows.index(row)
        };
        let matches = |i: u32| {
            self.refs
                .get(i as usize)
                .is_some_and(|r| self.ref_key(r) == key)
        };
        let guess = match key >> 60 {
            t if t == FACT_TAG >> 60 => row_of(None).map(|header| header.saturating_add(1 + index)),
            t if t == ANNOTATION_TAG >> 60 => {
                let rows = self.annotation_rows.iter().copied();
                rows.filter(|&i| i != NONE).find(|&i| matches(i))
            }
            t if t == MORE_TAG >> 60 => self.refs.len().checked_sub(1).map(|i| i as u32),
            t if t == RowKind::FileHeader as u64 => row_of(None),
            t if t == RowKind::Removed as u64 || t == RowKind::Modified as u64 => {
                row_of(Some(Side::Old))
            }
            t if t == RowKind::Context as u64 || t == RowKind::Added as u64 => {
                row_of(Some(Side::New))
            }
            _ => {
                // Outward from `near`: rows move little when a gap opens.
                let len = self.refs.len() as u32;
                for d in 0..len {
                    let below = near.checked_add(d).filter(|&i| i < len);
                    let above = near.checked_sub(d + 1);
                    if below.is_none() && above.is_none() {
                        break;
                    }
                    if let Some(i) = below.into_iter().chain(above).find(|&i| matches(i)) {
                        return Some(i);
                    }
                }
                return None;
            }
        };
        // A row these parts name is the only one the key can be.
        guess.filter(|&i| matches(i))
    }

    /// Rebuilds the row table from the segments' projections, keeping
    /// `anchor` where it was: by the source line it showed, else by its
    /// key, else at the same offset.
    pub(crate) fn rebuild_rows(&mut self, anchor: Option<Anchor>) {
        let m = self.metrics();
        let placed = self.annotation_placement();
        let limit = self.preview.map(|p| p.max_rows);
        let collapsed = &self.collapsed;
        // Files with annotations on their lines, whose rows go one by one.
        let annotated: HashSet<u32> = placed.at_line.keys().map(|&(unit, ..)| unit).collect();
        let mut b = RefsBuilder::default();
        let mut hidden = 0u32;
        let solo = self.files.solo;
        for (si, segment) in self.segments.iter_mut().enumerate() {
            let seg = si as u32;
            if solo.is_some_and(|slot| slot != segment.slot) {
                segment.list_rows = ListRows::default();
                continue;
            }
            let p = &segment.projection;
            let mut shown = ListRows::default();
            for (file, &header) in p.file_rows.iter().enumerate() {
                let file = file as u32;
                let end = p
                    .file_rows
                    .get(file as usize + 1)
                    .copied()
                    .unwrap_or(p.len());
                let unit = segment.slot + file;
                let folded = collapsed.contains(&unit);
                // A folded file shows only its header.
                let rows = header..if folded { header + 1 } else { end };
                let outdated = placed.outdated.get(&unit).filter(|_| !folded);
                if limit.is_none() && !annotated.contains(&unit) {
                    // The common case, a run at a time: the header, its
                    // metadata rows, then every other row of the file.
                    shown.push(header, b.push(RowRef::Line { seg, row: header }), 1);
                    if !folded && segment.pending.is_none() {
                        b.file_extras(&segment.doc, outdated, seg, file);
                    }
                    let lines = header + 1..rows.end;
                    shown.push(lines.start, b.refs.len() as u32, lines.len() as u32);
                    b.refs.push_lines(seg, lines);
                    continue;
                }
                for row in rows {
                    if limit.is_some_and(|max| b.refs.len() as u32 >= max) {
                        hidden += 1;
                        continue;
                    }
                    shown.push(row, b.push(RowRef::Line { seg, row }), 1);
                    let kind = p.kind[row as usize];
                    if kind == RowKind::FileHeader && !folded && segment.pending.is_none() {
                        b.file_extras(&segment.doc, outdated, seg, file);
                    }
                    if kind.is_line() {
                        for side in [Side::Old, Side::New] {
                            let Some(index) = p.line(row, side) else {
                                continue;
                            };
                            for &index in placed
                                .at_line
                                .get(&(unit, side, index))
                                .into_iter()
                                .flatten()
                            {
                                b.push(RowRef::Annotation { index });
                            }
                        }
                    }
                }
            }
            segment.list_rows = shown;
        }
        if hidden > 0 {
            b.push(RowRef::More { hidden });
        }
        self.refs = b.refs;
        self.annotation_rows = b.annotation_rows;
        // Rows are keyed by index, and line rows keep the estimate until
        // wrap measures them: building millions of rows is a pass over
        // their heights, with no key map.
        let rows = RowTable::indexed(m.line_h, self.refs.iter().map(|r| self.fixed_height(r, &m)));
        let mut list = VariableList::with_rows(rows, self.viewport.1);
        // A fresh list is pinned to the bottom; a diff opens at the top.
        list.set_scroll_offset(0.0);
        let offset = anchor
            .and_then(|a| {
                let index = a
                    .line
                    .and_then(|(unit, side, index)| self.list_index_of_line(unit, side, index))
                    .or_else(|| self.index_of_key(a.key, a.index))?;
                Some(list.rows().offset_of_index(index as usize) + a.delta)
            })
            .unwrap_or_else(|| self.list.scroll_offset());
        list.set_scroll_offset(offset);
        self.list = list;
        let near = self.focused_index.unwrap_or(0);
        self.focused_index = self.focused.and_then(|key| self.index_of_key(key, near));
        self.revision += 1;
    }

    /// The height of a row whose height does not come from its text: bands,
    /// annotations, and the preview's last row. `None` for rows wrap
    /// measures (their estimate is a line).
    fn fixed_height(&self, r: RowRef, m: &Metrics) -> Option<f32> {
        match r {
            RowRef::Line { seg, row } => {
                let kind = self.segments[seg as usize].projection.kind[row as usize];
                (!kind.is_line()).then(|| row_height(kind, m, &self.presentation))
            }
            RowRef::Fact { .. } => None,
            RowRef::Annotation { index } => Some(self.annotation_height(index, m)),
            RowRef::More { .. } => Some((m.line_h * 1.4).round()),
        }
    }
}

#[cfg(test)]
mod tests {
    use quark_ui::FocusId;

    use super::*;
    use crate::diff_view::{DiffTarget, FileId, RevealAlign, SourcePoint};

    // Catches the keyboard focus losing its row when rows above it open:
    // the view keeps the focused row's list index beside its key, and
    // revealing hidden lines moves every index below them.
    #[test]
    fn focus_keeps_its_line_when_rows_open_above_it() {
        let old: String = (1..=40).map(|i| format!("line {i}\n")).collect();
        let new = old
            .replace("line 10\n", "changed 10\n")
            .replace("line 30\n", "changed 30\n");
        let doc = diffy_diff::diff_texts(Some("a.txt"), Some("a.txt"), Some(&old), Some(&new), 3);
        let mut state = DiffViewState::new("test.diff", FocusId::new(1), doc);
        state.set_viewport(600.0, 400.0);
        let line_30 = DiffTarget::Source(SourcePoint {
            file: FileId(0),
            side: Side::New,
            line: 29,
            byte: 0,
        });
        state.reveal_target(line_30, RevealAlign::Top);
        assert_eq!(state.focused_target(), Some(line_30));

        assert!(state.expand_all());

        assert_eq!(state.focused_target(), Some(line_30));
    }

    use proptest::prelude::*;

    /// A row to append: a line (`seg`, `row`), a run of `len` lines, or a
    /// metadata row.
    fn appended() -> impl Strategy<Value = (u8, u32, u32)> {
        (0..3u8, 0..6u32, 0..4u32)
    }

    proptest! {
        // Catches runs that merge rows that do not follow on, or split
        // and miscount them: every row reads back as appended, by index
        // and in both directions.
        #[test]
        fn row_refs_read_back_as_appended(ops in prop::collection::vec(appended(), 0..40)) {
            let mut refs = RowRefs::default();
            let mut model = Vec::new();
            // Rows continue from the last line row more often than not.
            let mut next = 0u32;
            for (op, seg, n) in ops {
                let seg = seg % 2;
                match op {
                    0 => {
                        let row = if n == 0 { next + 3 } else { next };
                        refs.push(RowRef::Line { seg, row });
                        model.push(RowRef::Line { seg, row });
                        next = row + 1;
                    }
                    1 => {
                        refs.push_lines(seg, next..next + n);
                        model.extend((next..next + n).map(|row| RowRef::Line { seg, row }));
                        next += n;
                    }
                    _ => {
                        refs.push(RowRef::Fact { seg, file: 0, fact: n });
                        model.push(RowRef::Fact { seg, file: 0, fact: n });
                    }
                }
            }
            prop_assert_eq!(refs.len(), model.len());
            let by_index: Vec<RowRef> = (0..model.len()).map(|i| refs.at(i)).collect();
            prop_assert_eq!(&by_index, &model);
            prop_assert_eq!(&refs.iter().collect::<Vec<_>>(), &model);
            let mut reversed: Vec<RowRef> = refs.iter().rev().collect();
            reversed.reverse();
            prop_assert_eq!(&reversed, &model);
            prop_assert_eq!(refs.get(model.len()), None);
        }

        // Catches a shown run read past its end or merged across a gap:
        // every projection row maps to its list index or to none.
        #[test]
        fn list_rows_map_rows_shown_and_left_out(
            shown in prop::collection::vec(any::<bool>(), 0..60),
        ) {
            let mut list = ListRows::default();
            let mut index = 0u32;
            let mut model = Vec::new();
            for (row, &on) in shown.iter().enumerate() {
                if on {
                    list.push(row as u32, index, 1);
                    model.push(Some(index));
                    // An extra list row between some shown rows.
                    index += if row % 3 == 0 { 2 } else { 1 };
                } else {
                    model.push(None);
                }
            }
            let rows = shown.len() as u32;
            for (row, expected) in model.iter().enumerate() {
                prop_assert_eq!(list.index(row as u32), *expected, "row {}", row);
            }
            prop_assert_eq!(
                list.first_hidden(rows),
                model.iter().position(Option::is_none).map(|r| r as u32)
            );
        }
    }
}
