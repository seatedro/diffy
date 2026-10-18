//! The row model under both views: segments of the diff, the flat row
//! table over them, and the frame `prepare` builds from its window.
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
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use diffy_diff::{
    BlockKind, Comparison, ComparisonOptions, ContextPolicy, DiffDocument, Expansion, FileStatus,
    InlineOptions, LinePair, Mode, PairedInlineDiff, Projection, RowKind, Side, line_detail,
    paired_inline_diff,
};
use quark_render::FontKind;
use quark_text::{LayoutCache, TextParams, TextStyle, TextSystem};
use quark_ui::virtual_list::{RowTable, VariableList};

use super::prepared::{
    Columns, FileFact, FrameRow, LineDetail, LinePaint, LineWindow, Metrics, PreparedKind,
    RowPaint, ViewFrame, WordDetail, row_height,
};
use super::{DiffViewState, PrepareKey};

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
            list_rows: ListRows::default(),
            inline: RefCell::default(),
        }
    }

    pub fn unit(&self, file: u32) -> u32 {
        self.slot + file
    }

    /// Re-projects after an expansion or mode change.
    pub fn rebuild(&mut self, mode: Mode) {
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

/// Facts about `file` its line rows do not show. See [`FileFact`]. A
/// missing final newline is a fact only where it differs between the sides
/// (or the file was added or deleted), since an unchanged one is not part
/// of the change.
pub(crate) fn file_facts(doc: &DiffDocument, file: u32) -> Vec<FileFact> {
    let facts = doc.facts(file);
    let mut out = Vec::new();
    if facts.binary {
        out.push(FileFact::Binary);
    }
    if let Some((old, new)) = facts.mode_change {
        out.push(FileFact::ModeChange { old, new });
    }
    match facts.status {
        FileStatus::Renamed if !facts.has_hunks && !facts.binary => {
            out.push(FileFact::RenameOnly);
        }
        FileStatus::Copied if !facts.has_hunks && !facts.binary => out.push(FileFact::CopyOnly),
        _ => {}
    }
    let eof = [facts.old_missing_newline, facts.new_missing_newline];
    for side in [Side::Old, Side::New] {
        let differs = match facts.status {
            FileStatus::Added => side == Side::New,
            FileStatus::Deleted => side == Side::Old,
            _ => eof[0] != eof[1],
        };
        if eof[side as usize] && differs {
            out.push(FileFact::NoNewlineAtEof(side));
        }
    }
    out
}

/// How a metadata row reads.
pub(crate) fn fact_title(fact: &FileFact) -> String {
    match fact {
        FileFact::Binary => "Binary file not shown".to_owned(),
        FileFact::ModeChange { old, new } => format!("File mode changed from {old} to {new}"),
        FileFact::RenameOnly => "Renamed without changes".to_owned(),
        FileFact::CopyOnly => "Copied without changes".to_owned(),
        FileFact::NoNewlineAtEof(Side::Old) => "No newline at end of the old file".to_owned(),
        FileFact::NoNewlineAtEof(Side::New) => "No newline at end of the new file".to_owned(),
    }
}

/// With wrap on, lines longer than this wrap only a prefix of this many
/// bytes (reported as [`LineDetail::Prefix`]): wrapping lays a line out in
/// full, and a line of megabytes would wrap into a row of millions of
/// points.
pub(crate) const WRAP_LINE_BYTES: usize = 64 << 10;

/// Bytes of `text` a line limited to `limit` bytes shows; see
/// [`diffy_diff::line_detail`].
pub(crate) fn shown_len(text: &str, limit: usize) -> usize {
    match line_detail(text, limit) {
        LineDetail::Complete => text.len(),
        LineDetail::Prefix { shown_bytes, .. } => shown_bytes as usize,
    }
}

/// Largest char boundary at or below `byte`, clamped to the text.
pub(crate) fn floor_boundary(text: &str, byte: usize) -> usize {
    let mut at = byte.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
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

/// `@@ -a,b +c,d @@ section` of a hunk.
fn hunk_title(doc: &DiffDocument, hunk: u32) -> String {
    let (h, i) = (doc.hunks(), hunk as usize);
    let section = &h.section[i];
    format!(
        "@@ -{},{} +{},{} @@{}{section}",
        h.old_start[i],
        h.old_len[i],
        h.new_start[i],
        h.new_len[i],
        if section.is_empty() { "" } else { " " }
    )
}

/// Sizes for a font of `font_size` points whose digits are `char_w` wide.
fn metrics_for(font_size: f32, line_height: f32, char_w: f32, segments: &[Segment]) -> Metrics {
    let lines = segments
        .iter()
        .flat_map(|s| {
            let doc = &s.doc;
            (0..doc.file_count())
                .flat_map(move |f| [doc.text(f, Side::Old), doc.text(f, Side::New)])
        })
        .map(|t| t.line_count())
        .max()
        .unwrap_or(0)
        .max(1);
    // Partial stores hold fewer lines than the files; leave room for a
    // few more digits than they need.
    let digits = (lines.ilog10() + 1).max(3) as f32;
    Metrics {
        font_size,
        line_h: (font_size * line_height).round(),
        char_w,
        number_w: (digits * char_w + char_w * 2.0).ceil(),
        sign_w: (char_w * 2.0).ceil(),
        text_pad: (char_w * 0.5).ceil(),
    }
}

/// The scroll handle a side's text column uses: unified has one column.
pub(crate) fn column_slot(mode: Mode, side: Side) -> usize {
    match mode {
        Mode::Unified => Side::New as usize,
        Mode::Split => side as usize,
    }
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
        for (si, segment) in self.segments.iter_mut().enumerate() {
            let seg = si as u32;
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
                    if !folded {
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
                    if kind == RowKind::FileHeader && !folded {
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

    // ---- Frame ---------------------------------------------------------

    pub(crate) fn overscan(&self) -> f32 {
        self.metrics().line_h * 8.0
    }

    pub(crate) fn metrics(&self) -> Metrics {
        self.metrics
            .map_or_else(|| self.estimated_metrics(), |(m, _)| m)
    }

    /// Metrics before the font has been measured.
    fn estimated_metrics(&self) -> Metrics {
        let font_size = self.style.font_size;
        metrics_for(
            font_size,
            self.style.line_height,
            font_size * 0.6,
            &self.segments,
        )
    }

    /// Follows an automatic layout across the viewport width (see
    /// [`super::presentation::auto_mode`]), keeping the top source line in
    /// place. Explicit layouts stay put.
    fn resolve_layout(&mut self) {
        let m = self.metrics();
        if let Some(mode) = super::presentation::auto_mode(
            self.presentation.layout,
            self.mode,
            self.viewport.0,
            &m,
            &self.presentation,
        ) {
            self.switch_mode(mode);
        }
    }

    pub(crate) fn build_frame(
        &mut self,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
    ) {
        let measured = (scale.to_bits(), text.font_epoch());
        if self.metrics.is_none_or(|(_, key)| key != measured) {
            self.measure_font(text, layouts, scale);
        }
        self.resolve_layout();
        let m = self.metrics();
        let columns = Columns::new(self.mode, self.viewport.0, &m, &self.presentation);
        if self.style.wrap {
            self.measure_window(text, layouts, scale, &columns);
        }
        let window = self.list.window(self.overscan()).range;
        let grid = super::long_lines::GRID_COLUMNS as f64 * f64::from(m.char_w);
        let hgrid = [0, 1].map(|slot| {
            let scroll = self.long_lines.expected_scroll(slot, self.frame_id);
            (scroll.unwrap_or_else(|| self.hscroll[slot].offset_f64().0) / grid) as u32
        });
        let key = PrepareKey {
            window: (window.start, window.end),
            scroll: self.list.scroll_offset().to_bits(),
            revision: self.revision,
            scale: scale.to_bits(),
            hgrid,
        };
        if self.prepared == Some(key) && self.frame.is_some() {
            return;
        }
        self.prepared = Some(key);
        self.frame_id += 1;
        self.prioritize_syntax(window.clone());
        let scroll = self.list.scroll_offset();
        let ordered = self.ordered_selection();
        let mut kept = HashMap::with_capacity(window.len());
        let mut rows = Vec::with_capacity(window.len());
        let mut next_top: Option<f32> = None;
        for index in window {
            let r = self.refs.at(index);
            let key = self.ref_key(r);
            let stamp = self.stamp(r, &columns, scale);
            let paint = match self.painted.remove(&key) {
                Some(p) if p.stamp == stamp => p,
                _ => Rc::new(self.build_row(r, stamp, text, layouts, scale, &columns)),
            };
            for (side, line) in [Side::Old, Side::New].into_iter().zip(&paint.sides) {
                let Some(line) = line else {
                    continue;
                };
                let slot = column_slot(self.mode, side);
                let width = line
                    .window
                    .map_or(f64::from(line.layout.size().0), |w| w.width);
                let pad = f64::from(m.text_pad * 2.0 + m.char_w);
                self.content_w[slot] = self.content_w[slot].max((width + pad).ceil());
            }
            let (selected, search) = match r {
                RowRef::Line { seg, row } => {
                    let selected = [Side::Old, Side::New].map(|side| {
                        let line = paint.sides[side as usize].as_ref()?;
                        // In line bytes, then in the bytes of a window's text.
                        let start = line.window.map_or(0, |w| w.start);
                        let len = line.layout.text().len();
                        let (lo, hi) =
                            self.row_selection(ordered, (seg, row), side, start + len)?;
                        let (lo, hi) = (lo.max(start) - start, hi.saturating_sub(start));
                        (lo < hi).then_some((lo, hi))
                    });
                    (selected, self.search_marks(seg as usize, row, &paint))
                }
                _ => Default::default(),
            };
            let rows_table = self.list.rows();
            let height = rows_table.height_at(index);
            // Rows stack from the first row's top, taken relative to the
            // scroll in f64 before it becomes a viewport position.
            let top = *next_top
                .get_or_insert_with(|| (rows_table.offset_of_index(index) - scroll) as f32);
            next_top = Some(top + height);
            rows.push(FrameRow {
                key,
                index: index as u32,
                top,
                height,
                paint: paint.clone(),
                selected,
                search,
                focused: self.focused == Some(key),
            });
            kept.insert(key, paint);
        }
        let sticky_header = self.sticky_header(scroll, &columns, scale, text, layouts, &mut kept);
        self.painted = kept;
        self.frame = Some(Rc::new(ViewFrame {
            id: self.id,
            label: self.label,
            focus: self.focus,
            viewport: self.viewport,
            metrics: m,
            columns,
            presentation: self.presentation,
            appearance: self.appearance,
            wrap: self.style.wrap,
            rows,
            sticky_header,
            content_w: self.content_w,
            // A grid step before the scroll's: windows reach that far left,
            // so content between the origin and the scroll is all there.
            content_origin: hgrid.map(|step| f64::from(step.saturating_sub(1)) * grid),
            scroll,
            total: self.list.rows().total_extent(),
            row_count: self.refs.len() as u32,
            hscroll: self.hscroll.clone(),
            scrollbar_auto_hide: self.scrollbar_auto_hide,
            scrollbar: self.scrollbar.clone(),
        }));
    }

    /// The header to pin at the top: with sticky headers on, the file
    /// header of the top row's file when that header is above the
    /// viewport. Its paint is kept with the window's.
    #[allow(clippy::too_many_arguments)]
    fn sticky_header(
        &mut self,
        scroll: f64,
        columns: &Columns,
        scale: f32,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        kept: &mut HashMap<u64, Rc<RowPaint>>,
    ) -> Option<FrameRow> {
        if !self.presentation.sticky_headers {
            return None;
        }
        let top = self.list.rows().row_at(scroll)?;
        let (seg, file) = match self.refs.at(top) {
            RowRef::Line { seg, row } => (
                seg,
                self.segments[seg as usize].projection.file[row as usize],
            ),
            RowRef::Fact { seg, file, .. } => (seg, file),
            _ => return None,
        };
        let segment = &self.segments[seg as usize];
        let header = *segment.projection.file_rows.get(file as usize)?;
        let index = segment.list_rows.index(header)?;
        if index as usize >= top {
            return None;
        }
        let r = RowRef::Line { seg, row: header };
        let key = self.ref_key(r);
        let stamp = self.stamp(r, columns, scale);
        let paint = match kept.get(&key).or_else(|| self.painted.get(&key)) {
            Some(p) if p.stamp == stamp => p.clone(),
            _ => Rc::new(self.build_row(r, stamp, text, layouts, scale, columns)),
        };
        kept.insert(key, paint.clone());
        let height = self.list.rows().height_at(index as usize);
        Some(FrameRow {
            key,
            index,
            top: 0.0,
            height,
            paint,
            selected: [None, None],
            search: Default::default(),
            focused: self.focused == Some(key),
        })
    }

    /// Tells the syntax workers which files are on screen, so theirs are
    /// highlighted first. Does nothing for bridges whose files did not
    /// move in or out of view.
    fn prioritize_syntax(&mut self, window: Range<usize>) {
        let refs = || self.refs.range(window.clone());
        // A long side still streaming colors the surroundings of its top
        // line on screen first (headers and the other side's rows have
        // none).
        for side in [Side::Old, Side::New] {
            let top = refs().find_map(|r| match r {
                RowRef::Line { seg, row } => {
                    let p = &self.segments[seg as usize].projection;
                    Some((seg as usize, row, p.line(row, side)?))
                }
                _ => None,
            });
            if let Some((seg, row, index)) = top {
                let segment = &self.segments[seg];
                let file = segment.projection.file[row as usize];
                let start = segment.doc.text(file, side).line_range(index);
                if let (Some((bridge, file)), Some(start)) = (self.syntax_of(seg, file), start) {
                    bridge.set_focus(file, side, start.start);
                }
            }
        }
        let seg_file = |r: RowRef| match r {
            RowRef::Line { seg, row } => Some((
                seg,
                self.segments[seg as usize].projection.file[row as usize],
            )),
            RowRef::Fact { seg, file, .. } => Some((seg, file)),
            _ => None,
        };
        let (Some(first), Some(last)) =
            (refs().find_map(seg_file), refs().rev().find_map(seg_file))
        else {
            return;
        };
        if self.files.is_static() {
            self.syntax.set_visible_files(first.1..last.1 + 1);
            return;
        }
        for (seg, segment) in self.segments.iter().enumerate() {
            if let Some(bridge) = self.slot_syntax.get_mut(segment.slot as usize) {
                let shown = (first.0..=last.0).contains(&(seg as u32));
                bridge.set_visible_files(0..u32::from(shown));
            }
        }
    }

    /// Heights of rows entering the window, from their wrapped layouts.
    fn measure_window(
        &mut self,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
        columns: &Columns,
    ) {
        let m = self.metrics();
        let overscan = self.overscan();
        let mut list = std::mem::replace(&mut self.list, VariableList::new(1.0, 0.0));
        let mut painted = std::mem::take(&mut self.painted);
        let this = &*self;
        // Keys of the indexed row table are list indices.
        list.measure_visible(this.viewport.0, overscan, |index, _| {
            let Some(r) = this.refs.get(index as usize) else {
                return m.line_h;
            };
            let key = this.ref_key(r);
            if let RowRef::Annotation { index } = r {
                return this.annotation_height(index, &m);
            }
            let stamp = this.stamp(r, columns, scale);
            match painted.get(&key) {
                Some(p) if p.stamp == stamp => p.height(&m, &this.presentation),
                _ => {
                    let p = this.build_row(r, stamp, text, layouts, scale, columns);
                    let height = p.height(&m, &this.presentation);
                    painted.insert(key, Rc::new(p));
                    height
                }
            }
        });
        self.list = list;
        self.painted = painted;
    }

    /// Identifies everything a row's paint depends on besides its key.
    fn stamp(&self, r: RowRef, columns: &Columns, scale: f32) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        (
            scale.to_bits(),
            self.style.font_size.to_bits(),
            self.style.line_height.to_bits(),
            self.mode,
            self.limits,
        )
            .hash(&mut h);
        match r {
            RowRef::Line { seg, row } => {
                let segment = &self.segments[seg as usize];
                let p = &segment.projection;
                let file = p.file[row as usize];
                (segment.generation, segment.slot).hash(&mut h);
                if let Some((syntax, file)) = self.syntax_of(seg as usize, file) {
                    syntax.file_generation(file).hash(&mut h);
                }
                if self.style.wrap {
                    let m = self.metrics();
                    for side in [Side::Old, Side::New] {
                        columns.wrap_width(side, &m).to_bits().hash(&mut h);
                    }
                } else if p.kind[row as usize].is_line() {
                    for side in [Side::Old, Side::New] {
                        let Some(index) = p.line(row, side) else {
                            continue;
                        };
                        let store = segment.doc.text(file, side);
                        let range = store.display_range(index).unwrap_or(0..0);
                        if range.len() > super::long_lines::WINDOW_LINE_BYTES {
                            let window = self.line_window(store, range, side, columns);
                            window.map(|w| w.start).hash(&mut h);
                        }
                    }
                }
                if matches!(
                    p.kind[row as usize],
                    RowKind::Modified | RowKind::Removed | RowKind::Added
                ) {
                    self.long_lines.generation().hash(&mut h);
                }
                if p.kind[row as usize] == RowKind::FileHeader {
                    let unit = segment.unit(file);
                    (self.collapsed.contains(&unit), self.file_annotations(unit)).hash(&mut h);
                }
                if let Some(gap) = p.gap(row) {
                    // Its count changes as lines are revealed.
                    gap.hidden.hash(&mut h);
                    self.gap_annotations(segment, gap).hash(&mut h);
                }
            }
            RowRef::Fact { seg, .. } => self.segments[seg as usize].generation.hash(&mut h),
            RowRef::Annotation { index } => self.annotations.entry(index).hash(&mut h),
            RowRef::More { hidden } => hidden.hash(&mut h),
        }
        h.finish()
    }

    fn measure_font(&mut self, text: &mut TextSystem, layouts: &mut LayoutCache, scale: f32) {
        let style = self.text_style();
        let params = TextParams::new("0000000000", style).scale_factor(scale);
        let char_w = layouts
            .layout(text, &params)
            .map_or(self.style.font_size * 0.6, |l| l.size().0 / 10.0);
        let m = metrics_for(
            self.style.font_size,
            self.style.line_height,
            char_w,
            &self.segments,
        );
        let fonts = text.font_epoch();
        // The metrics the rows were built with: measured, or estimated
        // before the first frame.
        let old = self.metrics();
        let new_fonts = self.metrics.is_some_and(|(_, (_, old))| old != fonts);
        self.metrics = Some((m, (scale.to_bits(), fonts)));
        if new_fonts {
            // Rows were shaped and wrapped with the old fonts.
            self.painted.clear();
            self.content_w = [0.0; 2];
        }
        // Row heights follow the line height; wrapped rows also follow the
        // text width, which the digit width moves. Anything else (the
        // first frame measuring the font, mostly) keeps the rows: a
        // rebuild costs a pass over every row.
        if old.line_h != m.line_h {
            let anchor = self.anchor();
            self.rebuild_rows(anchor);
        } else if old != m || new_fonts {
            if self.style.wrap {
                self.list.invalidate_all();
            }
            self.revision += 1;
        }
    }

    fn text_style(&self) -> TextStyle {
        TextStyle::new(self.style.font_size)
            .kind(FontKind::Mono)
            .line_height((self.style.font_size * self.style.line_height).round())
    }

    fn build_row(
        &self,
        r: RowRef,
        stamp: u64,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
        columns: &Columns,
    ) -> RowPaint {
        let blank = |kind: PreparedKind, file: u32| RowPaint {
            stamp,
            kind,
            file,
            sides: [None, None],
            source_lines: [None, None],
            title: Arc::from(""),
            gap: None,
            hidden: 0,
            status: FileStatus::Modified,
            stats: (0, 0),
            binary: false,
            syntax: None,
            collapsed: false,
            annotations: 0,
        };
        match r {
            RowRef::Line { seg, row } => {
                self.build_line_row(seg as usize, row, stamp, text, layouts, scale, columns)
            }
            RowRef::Fact { seg, file, fact } => {
                let segment = &self.segments[seg as usize];
                let facts = file_facts(&segment.doc, file);
                let fact = facts[fact as usize].clone();
                RowPaint {
                    title: fact_title(&fact).into(),
                    ..blank(PreparedKind::Fact(fact), segment.unit(file))
                }
            }
            RowRef::Annotation { index } => {
                let (slot, unit, title) = self.annotation_slot(index);
                RowPaint {
                    title,
                    ..blank(PreparedKind::Annotation(slot), unit)
                }
            }
            RowRef::More { hidden } => {
                let rows = if hidden == 1 { "row" } else { "rows" };
                RowPaint {
                    title: format!("{hidden} more {rows}").into(),
                    ..blank(
                        PreparedKind::More {
                            hidden_rows: hidden,
                        },
                        0,
                    )
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_line_row(
        &self,
        seg: usize,
        row: u32,
        stamp: u64,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
        columns: &Columns,
    ) -> RowPaint {
        let segment = &self.segments[seg];
        let (doc, p) = (&segment.doc, &segment.projection);
        let r = row as usize;
        let (kind, file) = (p.kind[r], p.file[r]);
        let meta = &doc.files().meta[file as usize];
        let mut paint = RowPaint {
            stamp,
            kind: PreparedKind::Diff(kind),
            file: segment.unit(file),
            sides: [None, None],
            source_lines: [None, None],
            title: Arc::from(""),
            gap: None,
            hidden: 0,
            status: meta.status,
            stats: (
                doc.files().additions[file as usize],
                doc.files().deletions[file as usize],
            ),
            binary: meta.binary,
            syntax: None,
            collapsed: false,
            annotations: 0,
        };
        match kind {
            RowKind::FileHeader => {
                let unit = segment.unit(file);
                paint.syntax = self
                    .syntax_of(seg, file)
                    .and_then(|(syntax, file)| syntax.file_status(file));
                paint.collapsed = self.collapsed.contains(&unit);
                paint.annotations = self.file_annotations(unit);
                paint.title = match (&meta.old_path, &meta.new_path) {
                    (Some(old), Some(new)) if old != new => format!("{old} \u{2192} {new}").into(),
                    _ => doc.path(file).into(),
                };
            }
            RowKind::HunkHeader => paint.title = hunk_title(doc, p.hunk[r]).into(),
            RowKind::Gap => {
                let gap = p.gap(row).expect("gap row");
                paint.hidden = gap.hidden;
                paint.gap = Some(diffy_diff::GapId {
                    file: segment.unit(file),
                    ..gap.id
                });
                let lines = if gap.hidden == 1 { "line" } else { "lines" };
                let mut title = format!("{} unchanged {lines}", gap.hidden);
                let notes = self.gap_annotations(segment, gap);
                if notes > 0 {
                    let s = if notes == 1 { "" } else { "s" };
                    title.push_str(&format!(", {notes} annotation{s}"));
                }
                if let Some(h) = gap.id.hunk {
                    title.push_str("    ");
                    title.push_str(&hunk_title(doc, h));
                }
                paint.title = title.into();
            }
            _ => {
                let m = self.metrics();
                let (words, word_detail) = self.word_ranges(segment, row);
                for side in [Side::Old, Side::New] {
                    let Some(index) = p.line(row, side) else {
                        continue;
                    };
                    paint.source_lines[side as usize] = Some(source_line(doc, file, side, index));
                    if p.mode == Mode::Unified && kind == RowKind::Context && side == Side::Old {
                        // Unified context shows the new side's text.
                        continue;
                    }
                    let store = doc.text(file, side);
                    let line = store.display_line(index).unwrap_or("");
                    // Wrapping lays out a line in full, so a huge one wraps
                    // only a prefix (and says so); without wrap a long line
                    // shapes the window in view.
                    let cap = if self.style.wrap {
                        self.limits.shaped_line_bytes.min(WRAP_LINE_BYTES)
                    } else {
                        self.limits.shaped_line_bytes
                    };
                    let detail = line_detail(line, cap);
                    let shown = shown_len(line, cap);
                    let range = store.display_range(index).unwrap_or(0..0);
                    let window =
                        self.line_window(store, range.start..range.start + shown, side, columns);
                    let (from, to) = window.map_or((0, shown), |w| (w.start, w.end));
                    let line = &line[from..to];
                    let range = range.start + from..range.start + to;
                    let (spans, tones) = match self.syntax_of(seg, file) {
                        Some((syntax, file)) => syntax.spans(file, side, range),
                        None => (Vec::new(), Arc::from([])),
                    };
                    let params = TextParams::new(line, self.text_style())
                        .spans(spans)
                        .scale_factor(scale)
                        .wrap_width(self.style.wrap.then(|| columns.wrap_width(side, &m)));
                    let Ok(layout) = layouts.layout(text, &params) else {
                        continue;
                    };
                    let words = match &window {
                        Some(w) => super::long_lines::clip_to_window(&words[side as usize], w),
                        None => clip_ranges(&words[side as usize], shown),
                    };
                    paint.sides[side as usize] = Some(LinePaint {
                        layout,
                        tones,
                        words,
                        detail,
                        word_detail,
                        window,
                    });
                }
            }
        }
        paint
    }

    /// Changed words of `row`'s lines, by side, and how complete they are:
    /// the pair's inline diff, shared by both unified rows that show it. A
    /// long pair's comes from the word thread; none until it lands.
    fn word_ranges(&self, segment: &Segment, row: u32) -> ([Vec<Range<usize>>; 2], WordDetail) {
        let Some(pair) = segment.projection.line_pair(row) else {
            return (Default::default(), WordDetail::Unpaired);
        };
        let options = InlineOptions {
            max_line_bytes: self
                .inline_options
                .max_line_bytes
                .min(self.limits.inline_line_bytes),
            ..self.inline_options
        };
        let line = |side, i| {
            let store = segment.doc.text(pair.file, side);
            (store.shared(), store.display_range(i).unwrap_or(0..0))
        };
        let (a, b) = (line(Side::Old, pair.old), line(Side::New, pair.new));
        let long = a.1.len() + b.1.len() > super::long_lines::SYNC_PAIR_BYTES;
        if long && a.1.len().max(b.1.len()) <= options.max_line_bytes {
            let wake = self.syntax_wake.clone();
            return match self.long_lines.words(a, b, &options, wake) {
                Some(words) => (words.0.clone(), WordDetail::Done(words.1)),
                None => (Default::default(), WordDetail::Pending),
            };
        }
        let d = segment.inline_diff(pair, &options);
        let bytes = |v: &[Range<u32>]| -> Vec<Range<usize>> {
            v.iter().map(|r| r.start as usize..r.end as usize).collect()
        };
        ([bytes(&d.old), bytes(&d.new)], WordDetail::Done(d.detail))
    }

    /// Scrolls the column showing `side` sideways so byte `byte` of a long
    /// line (store index `index` of `file`) is in view, unless it is. Short
    /// lines, and wrapped ones, stay put.
    pub(crate) fn reveal_byte(
        &mut self,
        seg: usize,
        file: u32,
        side: Side,
        index: u32,
        byte: usize,
    ) {
        if self.style.wrap {
            return;
        }
        let store = self.segments[seg].doc.text(file, side);
        let Some(range) = store.display_range(index) else {
            return;
        };
        if range.len() <= super::long_lines::WINDOW_LINE_BYTES {
            return;
        }
        let m = self.metrics();
        let x = self.long_lines.x_of(store.shared(), range, byte, m.char_w);
        let columns = Columns::new(self.mode, self.viewport.0, &m, &self.presentation);
        let view_w = f64::from(columns.of(side).text_w - m.text_pad * 2.0);
        let handle = &self.hscroll[column_slot(self.mode, side)];
        let (at, _) = handle.offset_f64();
        if x < at || x > at + view_w - f64::from(m.char_w * 8.0) {
            let to = (x - view_w / 3.0).max(0.0);
            handle.set_offset(to, 0.0);
            // The handle moves only as the next frame paints; windows built
            // before then must already be the ones it will show.
            self.long_lines
                .expect_scroll(column_slot(self.mode, side), to, self.frame_id);
            self.revision += 1;
        }
    }

    /// The window of a long line (`line`, a range of `store`'s text) to
    /// shape at its column's sideways scroll; `None` with wrap on or for a
    /// line short enough to shape whole.
    fn line_window(
        &self,
        store: &diffy_diff::TextStore,
        line: Range<usize>,
        side: Side,
        columns: &Columns,
    ) -> Option<LineWindow> {
        if self.style.wrap {
            return None;
        }
        let m = self.metrics();
        let slot = column_slot(self.mode, side);
        let scroll = self
            .long_lines
            .expected_scroll(slot, self.frame_id)
            .unwrap_or_else(|| self.hscroll[slot].offset_f64().0);
        let view_w = columns.of(side).text_w;
        self.long_lines
            .window(store.shared(), line, scroll, view_w, m.char_w)
    }
}

/// `ranges` cut to the first `len` bytes.
fn clip_ranges(ranges: &[Range<usize>], len: usize) -> Vec<Range<usize>> {
    ranges
        .iter()
        .filter(|r| r.start < len)
        .map(|r| r.start..r.end.min(len))
        .collect()
}

#[cfg(test)]
mod tests {
    use quark_ui::FocusId;

    use super::*;
    use crate::diff_view::{DiffEvent, DiffStyle, DiffTarget, FileId, RevealAlign, SourcePoint};

    // Catches rows shaped and wrapped before a font change being kept: a
    // wrapped diff must lay out as a fresh view does in the new fonts.
    #[test]
    fn font_change_reshapes_and_rewraps_every_row() {
        let long = "let wrapped = some_function(first_argument, second_argument, third);";
        let patch = format!(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-{long}\n+{long} // x\n"
        );
        let doc = || diffy_diff::parse_unified(&patch).unwrap();
        let open = || {
            let mut state = DiffViewState::new("test.diff", FocusId::new(1), doc());
            state.set_style(DiffStyle {
                wrap: true,
                ..DiffStyle::default()
            });
            state.set_viewport(260.0, 400.0);
            state
        };
        let mut text = TextSystem::vendored_only(&Default::default());
        let mut layouts = LayoutCache::default();
        // Row heights, and each side's text in pixels wide.
        let layout =
            |state: &mut DiffViewState, text: &mut TextSystem, layouts: &mut LayoutCache| {
                state.prepare(text, layouts, 1.0, 0);
                let frame = state.frame.as_ref().unwrap();
                let rows: Vec<String> = frame
                    .rows
                    .iter()
                    .map(|row| {
                        let widths: Vec<f32> = row
                            .paint
                            .sides
                            .iter()
                            .flatten()
                            .map(|side| side.layout.size().0)
                            .collect();
                        format!("{}:{widths:?}", row.height)
                    })
                    .collect();
                rows.join(" ")
            };
        let mut state = open();
        let before = layout(&mut state, &mut text, &mut layouts);

        text.set_font_settings(&quark_text::FontSettings {
            // Proportional, so the lines wrap elsewhere.
            mono_family: "Inter".into(),
            ..Default::default()
        });
        let after = layout(&mut state, &mut text, &mut layouts);

        let expected = layout(&mut open(), &mut text, &mut layouts);
        assert_ne!(before, expected, "the fonts lay the rows out differently");
        assert_eq!(after, expected);
    }

    // Catches the rows being rebuilt whenever the font metrics are
    // measured again, though row heights did not change (here a font the
    // diff does not use was loaded; the first frame of every view measures
    // too). The rebuild dropped the measured heights of wrapped rows off
    // screen, so the content height fell back to estimates, and it costs a
    // pass over every row of a huge diff.
    #[test]
    fn remeasuring_fonts_keeps_wrapped_rows_measured_off_screen() {
        let long = "word ".repeat(40);
        let new: String = (0..40).map(|i| format!("{long}{i}\n")).collect();
        let doc = diffy_diff::diff_texts(Some("a.txt"), Some("a.txt"), Some(""), Some(&new), 3);
        let mut state = DiffViewState::new("test.diff", FocusId::new(1), doc);
        state.set_style(DiffStyle {
            wrap: true,
            ..DiffStyle::default()
        });
        state.set_viewport(300.0, 200.0);
        let mut text = TextSystem::vendored_only(&Default::default());
        let mut layouts = LayoutCache::default();
        state.prepare(&mut text, &mut layouts, 1.0, 0);
        state.handle(DiffEvent::ScrollTo(f32::MAX));
        state.prepare(&mut text, &mut layouts, 1.0, 0);
        let before = state.content_height();

        let font: &'static [u8] = include_bytes!("../../../../assets/fonts/Geist-Bold.otf");
        text.load_font_data(Arc::new(font));
        state.prepare(&mut text, &mut layouts, 1.0, 0);

        assert_eq!(state.content_height(), before);
    }

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

    /// Each prepared line row as `old | new`, changed words in brackets.
    fn split_rows(state: &DiffViewState) -> Vec<String> {
        let frame = state.frame().unwrap();
        let side = |row: &FrameRow, side: Side| {
            let Some(line) = &row.paint.sides[side as usize] else {
                return String::new();
            };
            let text = line.layout.text();
            let mut out = String::new();
            let mut at = 0;
            for r in &line.words {
                out.push_str(&text[at..r.start]);
                out.push_str(&format!("[{}]", &text[r.clone()]));
                at = r.end;
            }
            out.push_str(&text[at..]);
            out
        };
        frame
            .rows
            .iter()
            .filter(|row| row.paint.kind.is_line())
            .map(|row| format!("{} | {}", side(row, Side::Old), side(row, Side::New)))
            .collect()
    }

    // Catches the view pairing changed lines by position: a log line
    // inserted above an edited return must not sit beside the old return,
    // and the return's highlight must be its own edit.
    #[test]
    fn an_edited_line_pairs_with_its_similar_replacement() {
        let old = "fn f() {\n    return a + b;\n}\n";
        let new = "fn f() {\n    log(\"x\");\n    return a + b * 2;\n}\n";
        let doc = diffy_diff::diff_texts(Some("f.rs"), Some("f.rs"), Some(old), Some(new), 3);
        let mut state = DiffViewState::new("test.diff", FocusId::new(1), doc);
        state.set_mode(Mode::Split);
        state.set_viewport(800.0, 400.0);
        let mut text = TextSystem::vendored_only(&Default::default());
        let mut layouts = LayoutCache::default();
        state.prepare(&mut text, &mut layouts, 1.0, 0);

        assert_eq!(
            split_rows(&state),
            [
                "fn f() { | fn f() {",
                " |     log(\"x\");",
                "    return a + b; |     return a + b[ * 2];",
                "} | }",
            ]
        );
    }
}
