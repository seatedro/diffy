//! Display rows of a document: file headers, hunk headers or collapsed
//! gaps of unchanged lines, and line rows, unified or side by side.
//!
//! A [`Projection`] is a flat column table rebuilt in O(rows) whenever the
//! mode or the [`Expansion`] changes. Rows refer to lines by store index,
//! so an app reads the text from the document.

use crate::compare::{Comparison, LinePair, Step};
use crate::model::{BlockKind, DiffDocument, Side, lines_before};

/// No line on this side.
pub const NONE: u32 = u32::MAX;

/// Gaps that hide this many lines or fewer are shown instead, by default.
pub const MIN_HIDDEN: u32 = 3;

/// Lines one reveal action shows, by default.
pub const REVEAL_STEP: u32 = 20;

/// How collapsed context behaves: lines a reveal action shows, and the
/// largest hidden remainder that is shown instead of collapsed. Zero
/// `min_hidden` keeps even a one-line gap collapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContextPolicy {
    pub reveal_step: u32,
    pub min_hidden: u32,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            reveal_step: REVEAL_STEP,
            min_hidden: MIN_HIDDEN,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    #[default]
    Unified,
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKind {
    FileHeader,
    /// A hunk's `@@` line, for patches without the surrounding text.
    HunkHeader,
    /// Unchanged lines left out; [`Projection::gap`] says which.
    Gap,
    Context,
    Removed,
    Added,
    /// Side by side only: a removed line beside the added line it became.
    Modified,
}

impl RowKind {
    /// Whether the row shows lines of the file.
    pub const fn is_line(self) -> bool {
        matches!(
            self,
            Self::Context | Self::Removed | Self::Added | Self::Modified
        )
    }

    /// How assistive tech describes a line of this kind.
    pub const fn description(self) -> &'static str {
        match self {
            Self::FileHeader => "file",
            Self::HunkHeader => "hunk",
            Self::Gap => "collapsed lines",
            Self::Context => "unchanged",
            Self::Removed => "removed",
            Self::Added => "added",
            Self::Modified => "changed",
        }
    }
}

/// A run of unchanged lines between hunks: above hunk `hunk`, or after the
/// last hunk of `file` when `hunk` is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GapId {
    pub file: u32,
    pub hunk: Option<u32>,
}

/// Which end of a gap to reveal lines from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reveal {
    /// Lines at the top of the gap, below the previous hunk.
    Down,
    /// Lines at the bottom of the gap, above the next hunk.
    Up,
    All,
}

/// Lines revealed from each end of every gap, under a [`ContextPolicy`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expansion {
    /// Per hunk: lines shown from the top and bottom of the gap above it.
    above: Vec<(u32, u32)>,
    /// Per file: lines shown from the top of the gap after its last hunk.
    after: Vec<u32>,
    policy: ContextPolicy,
}

impl Expansion {
    pub fn new(doc: &DiffDocument) -> Self {
        Self::with_policy(doc, ContextPolicy::default())
    }

    pub fn with_policy(doc: &DiffDocument, policy: ContextPolicy) -> Self {
        Self {
            above: vec![(0, 0); doc.hunk_count() as usize],
            after: vec![0; doc.file_count() as usize],
            policy,
        }
    }

    pub fn policy(&self) -> ContextPolicy {
        self.policy
    }

    /// Changes the policy, keeping the lines already revealed.
    pub fn set_policy(&mut self, policy: ContextPolicy) {
        self.policy = policy;
    }

    /// Hides every revealed line of `gap` again. Returns whether anything
    /// changed.
    pub fn collapse(&mut self, gap: GapId) -> bool {
        let slot = match gap.hunk {
            Some(h) => self.above.get_mut(h as usize).map(|s| {
                let was = *s != (0, 0);
                *s = (0, 0);
                was
            }),
            None => self.after.get_mut(gap.file as usize).map(|s| {
                let was = *s != 0;
                *s = 0;
                was
            }),
        };
        slot.unwrap_or(false)
    }

    /// Reveals every gap of every file. Returns whether anything changed.
    pub fn reveal_all(&mut self, doc: &DiffDocument) -> bool {
        let mut changed = false;
        for file in 0..doc.file_count() {
            for hunk in doc.files().hunks[file as usize].clone() {
                let gap = GapId {
                    file,
                    hunk: Some(hunk),
                };
                changed |= self.reveal(doc, gap, Reveal::All, 0);
            }
            changed |= self.reveal(doc, GapId { file, hunk: None }, Reveal::All, 0);
        }
        changed
    }

    /// Lines `gap` holds in all.
    pub fn gap_len(doc: &DiffDocument, gap: GapId) -> u32 {
        match gap.hunk {
            Some(h) => doc.gap_above(h),
            None => doc.gap_after(gap.file),
        }
    }

    /// Lines of `gap` shown from its top and its bottom.
    pub fn revealed(&self, gap: GapId) -> (u32, u32) {
        match gap.hunk {
            Some(h) => self.above.get(h as usize).copied().unwrap_or((0, 0)),
            None => (self.after.get(gap.file as usize).copied().unwrap_or(0), 0),
        }
    }

    /// Lines of `gap` still hidden. A remainder of the policy's
    /// `min_hidden` lines or fewer is shown, so it counts as none.
    pub fn hidden(&self, doc: &DiffDocument, gap: GapId) -> u32 {
        self.hidden_of(gap, Self::gap_len(doc, gap))
    }

    /// [`Self::hidden`] for a gap of `len` lines.
    fn hidden_of(&self, gap: GapId, len: u32) -> u32 {
        let (top, bottom) = self.revealed(gap);
        let hidden = len.saturating_sub(top.saturating_add(bottom));
        if hidden <= self.policy.min_hidden {
            0
        } else {
            hidden
        }
    }

    /// Reveals up to `amount` more lines of `gap`. A trailing gap only
    /// reveals downward. Returns whether anything changed.
    pub fn reveal(&mut self, doc: &DiffDocument, gap: GapId, reveal: Reveal, amount: u32) -> bool {
        self.reveal_of(gap, Self::gap_len(doc, gap), reveal, amount)
    }

    /// [`Self::reveal`] for a gap of `len` lines.
    fn reveal_of(&mut self, gap: GapId, len: u32, reveal: Reveal, amount: u32) -> bool {
        let hidden = self.hidden_of(gap, len);
        if hidden == 0 {
            return false;
        }
        let take = if reveal == Reveal::All || hidden - amount.min(hidden) <= self.policy.min_hidden
        {
            hidden
        } else {
            amount
        };
        match gap.hunk {
            Some(h) => {
                let slot = &mut self.above[h as usize];
                match reveal {
                    Reveal::Up => slot.1 += take,
                    Reveal::Down | Reveal::All => slot.0 += take,
                }
            }
            None => self.after[gap.file as usize] += take,
        }
        true
    }
}

/// One collapsed gap row: the gap and the store indices of its hidden
/// lines on each side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapRow {
    pub id: GapId,
    pub old_start: u32,
    pub new_start: u32,
    pub hidden: u32,
}

/// Display rows as columns. See the [module docs](self).
#[derive(Debug, Clone, Default)]
pub struct Projection {
    pub mode: Mode,
    pub kind: Vec<RowKind>,
    pub file: Vec<u32>,
    /// The hunk of hunk header and line rows that belong to one; [`NONE`]
    /// for expanded gap lines and file headers. For gap rows, the index
    /// into [`Self::gaps`].
    pub hunk: Vec<u32>,
    /// Store index of the row's old and new line, or [`NONE`].
    pub old: Vec<u32>,
    pub new: Vec<u32>,
    /// Unified change rows: the store index of the line on the other side
    /// at the same offset of the change, for word diffs; else [`NONE`].
    pub pair: Vec<u32>,
    pub gaps: Vec<GapRow>,
    /// First row of each file and of each hunk's lines.
    pub file_rows: Vec<u32>,
    pub hunk_rows: Vec<u32>,
    /// Rows with an old line and rows with a new line, in order: the
    /// inverse of `old` and `new` per file.
    old_rows: Vec<u32>,
    new_rows: Vec<u32>,
}

impl Projection {
    pub fn new(doc: &DiffDocument, mode: Mode, expansion: &Expansion) -> Self {
        let mut p = Self {
            mode,
            ..Self::default()
        };
        p.rebuild(doc, mode, expansion);
        p
    }

    /// Rows of `doc` with change blocks aligned by `comparison`, which
    /// must have been computed for `doc`.
    pub fn with_comparison(
        doc: &DiffDocument,
        mode: Mode,
        expansion: &Expansion,
        comparison: &Comparison,
    ) -> Self {
        let mut p = Self {
            mode,
            ..Self::default()
        };
        p.rebuild_with(doc, mode, expansion, comparison);
        p
    }

    pub fn len(&self) -> u32 {
        self.kind.len() as u32
    }

    /// The changed line pair a row shows or belongs to: a side-by-side
    /// modified row, or a unified removed or added row with a partner.
    pub fn line_pair(&self, row: u32) -> Option<LinePair> {
        let r = row as usize;
        let (old, new) = match *self.kind.get(r)? {
            RowKind::Modified => (self.old[r], self.new[r]),
            RowKind::Removed => (self.old[r], self.pair[r]),
            RowKind::Added => (self.pair[r], self.new[r]),
            _ => return None,
        };
        (old != NONE && new != NONE).then_some(LinePair {
            file: self.file[r],
            old,
            new,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.kind.is_empty()
    }

    /// The gap a [`RowKind::Gap`] row shows.
    pub fn gap(&self, row: u32) -> Option<GapRow> {
        let r = row as usize;
        (self.kind.get(r) == Some(&RowKind::Gap)).then(|| self.gaps[self.hunk[r] as usize])
    }

    /// Store index of the row's line on `side`.
    pub fn line(&self, row: u32, side: Side) -> Option<u32> {
        let column = match side {
            Side::Old => &self.old,
            Side::New => &self.new,
        };
        column.get(row as usize).copied().filter(|&i| i != NONE)
    }

    /// The row showing line `index` of `file`'s `side` store, if any.
    pub fn row_of(&self, file: u32, side: Side, index: u32) -> Option<u32> {
        let (rows, column) = match side {
            Side::Old => (&self.old_rows, &self.old),
            Side::New => (&self.new_rows, &self.new),
        };
        let at =
            rows.partition_point(|&r| (self.file[r as usize], column[r as usize]) < (file, index));
        rows.get(at)
            .copied()
            .filter(|&r| self.file[r as usize] == file && column[r as usize] == index)
    }

    /// A key unique among this projection's rows and stable across
    /// expansion changes, for row tables and caches.
    pub fn row_key(&self, row: u32) -> u64 {
        let r = row as usize;
        let tag = self.kind[r] as u64;
        let file = u64::from(self.file[r]) & 0x3F_FFFF;
        let index = match self.kind[r] {
            RowKind::FileHeader => 0,
            RowKind::HunkHeader => u64::from(self.hunk[r]),
            RowKind::Gap => {
                let gap = self.gaps[self.hunk[r] as usize];
                gap.id.hunk.map_or(u64::from(u32::MAX), u64::from)
            }
            RowKind::Removed | RowKind::Modified => u64::from(self.old[r]),
            RowKind::Context | RowKind::Added => u64::from(self.new[r]),
        };
        (tag << 60) | (file << 38) | (index & 0x3F_FFFF_FFFF)
    }

    /// Rebuilds the rows in place, reusing the columns' memory. Changed
    /// lines pair by position; see [`Self::rebuild_with`].
    pub fn rebuild(&mut self, doc: &DiffDocument, mode: Mode, expansion: &Expansion) {
        self.rebuild_inner(doc, mode, expansion, None);
    }

    /// [`Self::rebuild`] with change blocks aligned by `comparison`, which
    /// must have been computed for `doc`.
    pub fn rebuild_with(
        &mut self,
        doc: &DiffDocument,
        mode: Mode,
        expansion: &Expansion,
        comparison: &Comparison,
    ) {
        assert!(
            comparison.fits(doc),
            "comparison computed for another document"
        );
        self.rebuild_inner(doc, mode, expansion, Some(comparison));
    }

    fn rebuild_inner(
        &mut self,
        doc: &DiffDocument,
        mode: Mode,
        expansion: &Expansion,
        comparison: Option<&Comparison>,
    ) {
        self.mode = mode;
        for column in [
            &mut self.file,
            &mut self.hunk,
            &mut self.old,
            &mut self.new,
            &mut self.pair,
            &mut self.file_rows,
            &mut self.hunk_rows,
            &mut self.old_rows,
            &mut self.new_rows,
        ] {
            column.clear();
        }
        self.kind.clear();
        self.gaps.clear();
        let (files, h, b) = (doc.files(), doc.hunks(), doc.blocks());
        for file in 0..doc.file_count() {
            let f = file as usize;
            self.file_rows.push(self.len());
            self.push(RowKind::FileHeader, file, NONE, NONE, NONE, NONE);
            let partial = files.partial[f];
            // Next unshown line of each store, for the gaps between hunks.
            let (mut old_next, mut new_next) = (0u32, 0u32);
            for hunk in files.hunks[f].clone() {
                let hi = hunk as usize;
                let old_before = lines_before(h.old_start[hi], h.old_len[hi]);
                let new_before = lines_before(h.new_start[hi], h.new_len[hi]);
                if partial {
                    self.push(RowKind::HunkHeader, file, hunk, NONE, NONE, NONE);
                } else {
                    let id = GapId {
                        file,
                        hunk: Some(hunk),
                    };
                    self.push_gap(id, (old_next, new_next), new_before - new_next, expansion);
                }
                self.hunk_rows.push(self.len());
                for block in h.blocks[hi].clone() {
                    let bi = block as usize;
                    let (os, ns) = (b.old_store[bi], b.new_store[bi]);
                    let (ol, nl) = (b.old_len[bi], b.new_len[bi]);
                    if let (BlockKind::Change, Some(c)) = (b.kind[bi], comparison) {
                        self.push_steps(c.block(block), file, hunk, mode);
                        continue;
                    }
                    match (b.kind[bi], mode) {
                        (BlockKind::Context, _) => {
                            for k in 0..ol {
                                self.push(RowKind::Context, file, hunk, os + k, ns + k, NONE);
                            }
                        }
                        (BlockKind::Change, Mode::Unified) => {
                            for k in 0..ol {
                                let pair = if k < nl { ns + k } else { NONE };
                                self.push(RowKind::Removed, file, hunk, os + k, NONE, pair);
                            }
                            for k in 0..nl {
                                let pair = if k < ol { os + k } else { NONE };
                                self.push(RowKind::Added, file, hunk, NONE, ns + k, pair);
                            }
                        }
                        (BlockKind::Change, Mode::Split) => {
                            for k in 0..ol.max(nl) {
                                let (old, new) = (
                                    if k < ol { os + k } else { NONE },
                                    if k < nl { ns + k } else { NONE },
                                );
                                let kind = match (old != NONE, new != NONE) {
                                    (true, true) => RowKind::Modified,
                                    (true, false) => RowKind::Removed,
                                    _ => RowKind::Added,
                                };
                                self.push(kind, file, hunk, old, new, NONE);
                            }
                        }
                    }
                }
                old_next = old_before + h.old_len[hi];
                new_next = new_before + h.new_len[hi];
            }
            if !partial {
                let id = GapId { file, hunk: None };
                let len = doc.gap_after(file);
                self.push_gap(id, (old_next, new_next), len, expansion);
            }
        }
        debug_assert_eq!(self.verify_integrity(doc), Ok(()));
    }

    /// Rows of one aligned change block. Unified rows list each run's
    /// removed lines, then its added lines; lines equal under the
    /// whitespace policy split runs and show as context.
    fn push_steps(&mut self, steps: &[Step], file: u32, hunk: u32, mode: Mode) {
        if mode == Mode::Split {
            for &step in steps {
                let (old, new) = step.lines();
                let kind = match step {
                    Step::Removed(_) => RowKind::Removed,
                    Step::Added(_) => RowKind::Added,
                    Step::Paired(..) => RowKind::Modified,
                    Step::Equivalent(..) => RowKind::Context,
                };
                self.push(kind, file, hunk, old, new, NONE);
            }
            return;
        }
        let mut start = 0;
        while start < steps.len() {
            let end = steps[start..]
                .iter()
                .position(|s| matches!(s, Step::Equivalent(..)))
                .map_or(steps.len(), |at| start + at);
            let run = &steps[start..end];
            for &step in run {
                let (old, pair) = step.lines();
                if old != NONE {
                    self.push(RowKind::Removed, file, hunk, old, NONE, pair);
                }
            }
            for &step in run {
                let (pair, new) = step.lines();
                if new != NONE {
                    self.push(RowKind::Added, file, hunk, NONE, new, pair);
                }
            }
            if let Some(&Step::Equivalent(old, new)) = steps.get(end) {
                self.push(RowKind::Context, file, hunk, old, new, NONE);
            }
            start = end + 1;
        }
    }

    fn push(&mut self, kind: RowKind, file: u32, hunk: u32, old: u32, new: u32, pair: u32) {
        let row = self.len();
        if old != NONE {
            self.old_rows.push(row);
        }
        if new != NONE {
            self.new_rows.push(row);
        }
        self.kind.push(kind);
        self.file.push(file);
        self.hunk.push(hunk);
        self.old.push(old);
        self.new.push(new);
        self.pair.push(pair);
    }

    /// The rows of a gap of `len` unchanged lines starting at the given
    /// store indices: revealed lines at the top, a gap row for the hidden
    /// middle (unless it is short enough to show), revealed lines below.
    fn push_gap(&mut self, id: GapId, (old, new): (u32, u32), len: u32, expansion: &Expansion) {
        let (top, hidden, bottom) =
            gap_split(len, expansion.revealed(id), expansion.policy.min_hidden);
        for k in 0..top {
            self.push(RowKind::Context, id.file, NONE, old + k, new + k, NONE);
        }
        if hidden > 0 {
            self.gaps.push(GapRow {
                id,
                old_start: old + top,
                new_start: new + top,
                hidden,
            });
            self.push(
                RowKind::Gap,
                id.file,
                self.gaps.len() as u32 - 1,
                NONE,
                NONE,
                NONE,
            );
        }
        for k in len - bottom..len {
            self.push(RowKind::Context, id.file, NONE, old + k, new + k, NONE);
        }
    }

    /// Checks the columns against each other and `doc`: equal lengths,
    /// line indices inside the stores and increasing per file and side,
    /// and the inverse row lists. O(rows).
    pub fn verify_integrity(&self, doc: &DiffDocument) -> Result<(), ProjectionError> {
        let n = self.kind.len();
        if [
            self.file.len(),
            self.hunk.len(),
            self.old.len(),
            self.new.len(),
            self.pair.len(),
        ]
        .iter()
        .any(|&len| len != n)
            || self.file_rows.len() != doc.file_count() as usize
        {
            return Err(ProjectionError::ColumnLength);
        }
        let mut last: [(u32, Option<u32>); 2] = [(0, None), (0, None)];
        let (mut old_rows, mut new_rows) = (0, 0);
        for row in 0..n {
            let file = self.file[row];
            for (side, column, rows, count) in [
                (0, &self.old, &self.old_rows, &mut old_rows),
                (1, &self.new, &self.new_rows, &mut new_rows),
            ] {
                let index = column[row];
                if index == NONE {
                    continue;
                }
                let store = doc.text(file, if side == 0 { Side::Old } else { Side::New });
                let increasing = match last[side] {
                    (f, Some(prev)) if f == file => index > prev,
                    _ => true,
                };
                if index >= store.line_count() || !increasing {
                    return Err(ProjectionError::Line { row: row as u32 });
                }
                if rows.get(*count) != Some(&(row as u32)) {
                    return Err(ProjectionError::RowList { row: row as u32 });
                }
                *count += 1;
                last[side] = (file, Some(index));
            }
            let lines = (self.old[row] != NONE, self.new[row] != NONE);
            let shape = match self.kind[row] {
                RowKind::Context | RowKind::Modified => lines == (true, true),
                RowKind::Removed => lines == (true, false),
                RowKind::Added => lines == (false, true),
                RowKind::Gap => self.hunk[row] < self.gaps.len() as u32,
                RowKind::FileHeader | RowKind::HunkHeader => lines == (false, false),
            };
            if !shape {
                return Err(ProjectionError::Line { row: row as u32 });
            }
        }
        if old_rows != self.old_rows.len() || new_rows != self.new_rows.len() {
            return Err(ProjectionError::RowList { row: n as u32 });
        }
        Ok(())
    }
}

/// Lines of a gap of `len` shown above its gap row, hidden under it, and
/// shown below it, for `(top, bottom)` lines revealed from each end. A
/// hidden remainder of `min_hidden` lines or fewer is shown instead.
fn gap_split(len: u32, (top, bottom): (u32, u32), min_hidden: u32) -> (u32, u32, u32) {
    let (mut top, bottom) = (top.min(len), bottom.min(len - top.min(len)));
    let mut hidden = len - top - bottom;
    if hidden <= min_hidden {
        top += hidden;
        hidden = 0;
    }
    (top, hidden, bottom)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    ColumnLength,
    Line { row: u32 },
    RowList { row: u32 },
}

#[cfg(kani)]
mod verification {
    use super::*;

    fn any_reveal() -> Reveal {
        match kani::any::<u8>() % 3 {
            0 => Reveal::Down,
            1 => Reveal::Up,
            _ => Reveal::All,
        }
    }

    /// Any gap, above a hunk or trailing a file, after any two reveals of
    /// any amount from either end: its rows (lines shown on top, one gap
    /// row, lines shown below) cover every line once, the gap row agrees
    /// with `Expansion::hidden`, a gap of `min_hidden` lines or fewer is
    /// shown instead, and lines revealed from an end show at that end.
    /// The row emission is left to the proptest: with rows pushed into
    /// the projection's columns, even six lines got the CI runner shut
    /// down while CBMC built the formula.
    #[kani::proof]
    fn gap_rows_show_every_line_once_after_any_reveals() {
        let len: u32 = kani::any();
        let id = GapId {
            file: 0,
            hunk: kani::any::<bool>().then_some(0),
        };
        let mut expansion = Expansion {
            above: vec![(0, 0)],
            after: vec![0],
            policy: ContextPolicy {
                reveal_step: REVEAL_STEP,
                min_hidden: kani::any::<u8>().into(),
            },
        };
        let min_hidden = expansion.policy.min_hidden;
        expansion.reveal_of(id, len, any_reveal(), kani::any());
        expansion.reveal_of(id, len, any_reveal(), kani::any());
        let revealed = expansion.revealed(id);

        let (top, hidden, bottom) = gap_split(len, revealed, min_hidden);

        assert!(u64::from(top) + u64::from(hidden) + u64::from(bottom) == u64::from(len));
        assert!(hidden == expansion.hidden_of(id, len));
        assert!(hidden == 0 || hidden > min_hidden);
        if hidden > 0 {
            assert!((top, bottom) == revealed);
        }
    }

    /// Lines revealed by any stale expansion (more than the gap holds, from
    /// either end) still split the gap without losing or repeating one.
    #[kani::proof]
    fn any_revealed_lines_split_a_gap_exactly() {
        let len: u32 = kani::any();
        let min_hidden: u32 = kani::any();
        let (top, hidden, bottom) = gap_split(len, kani::any(), min_hidden);
        assert!(u64::from(top) + u64::from(hidden) + u64::from(bottom) == u64::from(len));
        assert!(hidden == 0 || hidden > min_hidden);
    }
}
