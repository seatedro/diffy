//! How a document's change blocks are shown: which removed line sits
//! beside which added line, and which changes a whitespace policy hides.
//!
//! A [`Comparison`] decorates an exact [`DiffDocument`] without changing
//! it, so apply and export always use the exact edits. It is computed once
//! per document revision and options, then used by every projection
//! rebuild.

use std::ops::Range;

use crate::merge::{join_lines, merge_pair, reads_as_one};
use crate::model::{BlockKind, DiffDocument, Side};
use crate::myers::{diff, intern};
use crate::projection::NONE;
use crate::semantic::{AlignedRow, ChangeIntensity, Move};
use crate::text::TextStore;

/// Which spaces a comparison ignores. Only spaces and tabs count; a `\r`,
/// a missing final newline, and other Unicode spaces (such as no-break
/// space) always stay changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WhitespaceMode {
    #[default]
    Exact,
    /// Leading and trailing spaces and tabs.
    IgnoreEdgeSpace,
    /// Trailing spaces and tabs, and how many separate words (`git -b`).
    IgnoreSpaceChange,
    /// Every space and tab (`git -w`).
    IgnoreAllSpace,
}

/// How removed and added lines of a change are paired for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PairingMode {
    /// The k-th removed line beside the k-th added line.
    Positional,
    /// Like positional, but when the sides differ in length the shorter
    /// one may shift to sit beside the lines it resembles, if that is
    /// decisively better. Bounded by [`MAX_PAIRING_COMPARISONS`] plus
    /// [`PAIRING_COMPARISONS_PER_LINE`] per line of the change.
    #[default]
    Similarity,
}

/// Line comparisons any change may spend choosing a similarity pairing,
/// on top of [`PAIRING_COMPARISONS_PER_LINE`].
pub const MAX_PAIRING_COMPARISONS: u32 = 4_096;

/// Line comparisons a change may spend per line it holds, so the budget
/// grows with the input. Past it every shift is still scored, from evenly
/// spread sample lines; only a change with more shifts than the budget can
/// sample at all stays positional and counts as limited.
pub const PAIRING_COMPARISONS_PER_LINE: u32 = 4;

/// Bytes compared from each end of a line when scoring similarity.
pub const SIMILARITY_SCAN_BYTES: usize = 256;

pub const MOVE_GROUP_GAP: u32 = 8;
const MAX_REFLOW_STEPS: usize = 64;

/// Fewest sample lines a shift is scored from.
const MIN_PAIRING_SAMPLES: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ComparisonOptions {
    pub whitespace: WhitespaceMode,
    pub pairing: PairingMode,
}

/// A changed line pair, by file and store index; the key for caching its
/// inline diff, independent of the row that shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LinePair {
    pub file: u32,
    pub old: u32,
    pub new: u32,
}

/// One line of a change block, as shown.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LineGroup {
    pub new: Range<u32>,
    pub old: Range<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Step {
    Removed(u32),
    Added(u32),
    Paired(u32, u32),
    Merged(u32, u32),
    Hidden(u32),
    Reflowed(u32),
    Moved(u32),
    MovedHere(u32),
    MovedAway(u32),
    /// Old and new lines equal under the whitespace policy.
    Equivalent(u32, u32),
}

/// Display alignment of every change block of one document, computed
/// once per document revision and options. The document is not changed:
/// apply and export keep using its exact edits.
#[derive(Debug, Clone, Default)]
pub struct Comparison {
    options: ComparisonOptions,
    /// Per block: its steps, empty for context blocks.
    block_steps: Vec<Range<u32>>,
    steps: Vec<Step>,
    /// Per file: line pairs hidden as whitespace-only changes.
    hidden: Vec<u32>,
    /// Per file: changes left positional because similarity would have
    /// cost more than its budget (see [`PairingMode::Similarity`]).
    limited: Vec<u32>,
}

impl Comparison {
    /// Aligns every change block of `doc`. O(changed lines), plus a line
    /// diff per change block under a whitespace policy.
    pub fn new(doc: &DiffDocument, options: ComparisonOptions) -> Self {
        let mut out = Self {
            options,
            ..Self::default()
        };
        let (b, h) = (doc.blocks(), doc.hunks());
        let mut keys = (Vec::new(), Vec::new());
        for file in 0..doc.file_count() {
            let (old, new) = (doc.text(file, Side::Old), doc.text(file, Side::New));
            let (mut hidden, mut limited) = (0, 0);
            let structural = options.pairing == PairingMode::Similarity
                && options.whitespace == WhitespaceMode::Exact
                && doc.prefers_structural(file);
            for hunk in doc.files().hunks[file as usize].clone() {
                let rows = if structural {
                    doc.aligned_rows(hunk)
                } else {
                    &[]
                };
                for block in h.blocks[hunk as usize].clone() {
                    let bi = block as usize;
                    let start = out.steps.len() as u32;
                    if b.kind[bi] == BlockKind::Change {
                        let olds = b.old_store[bi]..b.old_store[bi] + b.old_len[bi];
                        let news = b.new_store[bi]..b.new_store[bi] + b.new_len[bi];
                        let lines = (b.old_line[bi] - 1, b.new_line[bi] - 1);
                        if !rows.is_empty()
                            && out.align_structural(rows, lines, olds.clone(), news.clone())
                        {
                            out.merge_pairs(doc, file, start as usize);
                            out.fold_moves(doc, file, start as usize);
                            out.fold_groups(doc, file, start as usize);
                            out.hide_dissolved(doc, file, start as usize);
                        } else {
                            let (h, l) = out.align_block(old, new, olds, news, &mut keys);
                            hidden += h;
                            limited += l;
                        }
                    }
                    out.block_steps.push(start..out.steps.len() as u32);
                }
            }
            out.hidden.push(hidden);
            out.limited.push(limited);
        }
        debug_assert_eq!(out.verify_integrity(doc), Ok(()));
        out
    }

    pub fn options(&self) -> ComparisonOptions {
        self.options
    }

    /// Changed line pairs of `file` the whitespace policy shows as
    /// unchanged.
    pub fn hidden_whitespace_changes(&self, file: u32) -> u32 {
        self.hidden.get(file as usize).copied().unwrap_or(0)
    }

    /// Changes of `file` shown positionally because similarity pairing
    /// would have exceeded its budget.
    pub fn limited_pairings(&self, file: u32) -> u32 {
        self.limited.get(file as usize).copied().unwrap_or(0)
    }

    pub fn line_groups(&self, doc: &DiffDocument, file: u32) -> Vec<LineGroup> {
        if !self.fits(doc) || file >= doc.file_count() {
            return Vec::new();
        }
        let moves = doc.semantics(file).map_or(&[][..], |s| s.moves());
        let moved_old = |line: u32| {
            moves.iter().position(|m| {
                (m.old_line.saturating_sub(1)..m.old_line + m.old_lines + 1).contains(&line)
            })
        };
        let mut groups = Vec::new();
        let mut move_old: Vec<Option<Range<u32>>> = vec![None; moves.len()];
        let mut run: Option<LineGroup> = None;
        let h = doc.hunks();
        for hunk in doc.files().hunks[file as usize].clone() {
            for block in h.blocks[hunk as usize].clone() {
                for &step in self.block(block) {
                    match step {
                        Step::Hidden(o) if let Some(m) = moved_old(o) => {
                            let range = move_old[m].get_or_insert(o..o + 1);
                            *range = range.start.min(o)..range.end.max(o + 1);
                            continue;
                        }
                        Step::Hidden(o) => {
                            let group = run.get_or_insert(LineGroup {
                                new: 0..0,
                                old: o..o,
                            });
                            if group.old.is_empty() {
                                group.old = o..o;
                            }
                            group.old.end = o + 1;
                            continue;
                        }
                        Step::Reflowed(n) => {
                            let group = run.get_or_insert(LineGroup {
                                new: n..n,
                                old: 0..0,
                            });
                            if group.new.is_empty() {
                                group.new = n..n;
                            }
                            group.new.end = n + 1;
                            continue;
                        }
                        Step::Merged(o, n) => groups.push(LineGroup {
                            new: n..n + 1,
                            old: o..o + 1,
                        }),
                        _ => {}
                    }
                    groups.extend(run.take().filter(|g| !g.new.is_empty()));
                }
                groups.extend(run.take().filter(|g| !g.new.is_empty()));
            }
        }
        for (m, old) in moves.iter().zip(move_old) {
            let old = old.map_or(m.old_line..m.old_line + m.old_lines, |hidden| {
                hidden.start.min(m.old_line)..hidden.end.max(m.old_line + m.old_lines)
            });
            groups.push(LineGroup {
                new: m.new_line..m.new_line + m.lines,
                old,
            });
        }
        groups.sort_by_key(|g| g.new.start);
        groups
    }

    /// Whether this comparison was computed for `doc`'s shape.
    pub fn fits(&self, doc: &DiffDocument) -> bool {
        self.block_steps.len() == doc.blocks().kind.len()
            && self.hidden.len() == doc.file_count() as usize
    }

    pub(crate) fn block(&self, block: u32) -> &[Step] {
        let r = &self.block_steps[block as usize];
        &self.steps[r.start as usize..r.end as usize]
    }

    /// Appends the steps of one change block; returns the lines hidden as
    /// equivalent and whether pairing hit its budget.
    fn align_block(
        &mut self,
        old: &TextStore,
        new: &TextStore,
        olds: Range<u32>,
        news: Range<u32>,
        keys: &mut (Vec<String>, Vec<String>),
    ) -> (u32, u32) {
        let mode = self.options.whitespace;
        if mode == WhitespaceMode::Exact || olds.is_empty() || news.is_empty() {
            let limited = self.pair_run(old, new, olds, news);
            return (0, u32::from(limited));
        }
        keys.0.clear();
        keys.1.clear();
        keys.0
            .extend(olds.clone().map(|i| normalized(old, i, mode)));
        keys.1
            .extend(news.clone().map(|i| normalized(new, i, mode)));
        let (a, b) = intern(keys.0.iter(), keys.1.iter());
        let (mut o, mut n) = (olds.start, news.start);
        let (mut hidden, mut limited) = (0, 0);
        let mut equal = |out: &mut Self, o: &mut u32, n: &mut u32, until: u32| {
            while *o < until {
                out.steps.push(Step::Equivalent(*o, *n));
                hidden += 1;
                *o += 1;
                *n += 1;
            }
        };
        for change in diff(&a, &b) {
            equal(self, &mut o, &mut n, olds.start + change.old.start);
            let run_old = olds.start + change.old.start..olds.start + change.old.end;
            let run_new = news.start + change.new.start..news.start + change.new.end;
            limited += u32::from(self.pair_run(old, new, run_old.clone(), run_new.clone()));
            (o, n) = (run_old.end, run_new.end);
        }
        equal(self, &mut o, &mut n, olds.end);
        (hidden, limited)
    }

    fn align_structural(
        &mut self,
        rows: &[AlignedRow],
        (old_line, new_line): (u32, u32),
        olds: Range<u32>,
        news: Range<u32>,
    ) -> bool {
        let store = |line: Option<u32>, first: u32, stores: &Range<u32>| {
            line.and_then(|line| line.checked_sub(first))
                .map(|offset| stores.start + offset)
                .filter(|index| stores.contains(index))
        };
        let start = self.steps.len();
        let (mut o, mut n) = (olds.start, news.start);
        for row in rows {
            let ro = store(row.old, old_line, &olds);
            let rn = store(row.new, new_line, &news);
            if ro.is_some_and(|i| i != o) || rn.is_some_and(|i| i != n) {
                self.steps.truncate(start);
                return false;
            }
            match (ro, rn) {
                (Some(a), Some(b)) => self.steps.push(Step::Paired(a, b)),
                (Some(a), None) => self.steps.push(Step::Removed(a)),
                (None, Some(b)) => self.steps.push(Step::Added(b)),
                (None, None) => continue,
            }
            o += u32::from(ro.is_some());
            n += u32::from(rn.is_some());
        }
        if o != olds.end || n != news.end {
            self.steps.truncate(start);
            return false;
        }
        true
    }

    fn merge_pairs(&mut self, doc: &DiffDocument, file: u32, start: usize) {
        let (old, new) = (doc.text(file, Side::Old), doc.text(file, Side::New));
        for step in &mut self.steps[start..] {
            let Step::Paired(o, n) = *step else {
                continue;
            };
            let merged = merge_pair(
                old.display_line(o).unwrap_or(""),
                new.display_line(n).unwrap_or(""),
                doc.semantic_spans(file, Side::Old, o),
                doc.semantic_spans(file, Side::New, n),
            );
            if merged.is_some() {
                *step = Step::Merged(o, n);
            }
        }
    }

    /// Hides removed lines that kept all their text: every token lives on
    /// in the new side, so the line only dissolved into others.
    fn hide_dissolved(&mut self, doc: &DiffDocument, file: u32, start: usize) {
        let old = doc.text(file, Side::Old);
        let dissolved = |o: u32| {
            old.display_line(o).is_some_and(|t| !t.trim().is_empty())
                && doc.semantic_spans(file, Side::Old, o).iter().all(|s| {
                    matches!(
                        s.intensity,
                        ChangeIntensity::UnchangedContext | ChangeIntensity::Renamed
                    )
                })
        };
        let block = self.steps.split_off(start);
        for step in block {
            match step {
                Step::Removed(o) if dissolved(o) => self.steps.push(Step::Hidden(o)),
                Step::Paired(o, n) if dissolved(o) => {
                    self.steps.push(Step::Hidden(o));
                    self.steps.push(Step::Added(n));
                }
                _ => self.steps.push(step),
            }
        }
    }

    fn fold_moves(&mut self, doc: &DiffDocument, file: u32, start: usize) {
        let Some(moves) = doc
            .semantics(file)
            .map(|s| s.moves())
            .filter(|m| !m.is_empty())
        else {
            return;
        };
        let old = doc.text(file, Side::Old);
        let hidden_old = |line: u32| {
            let within = |line: u32| {
                moves
                    .iter()
                    .find(|m| (m.old_line..m.old_line + m.old_lines).contains(&line))
            };
            let blank = |line: u32| old.line(line).is_some_and(|t| t.trim().is_empty());
            if let Some(m) = within(line) {
                let new_line = m.new_line + (line - m.old_line);
                return !removes(doc, file, line) || moved_from(doc, file, new_line) == Some(line);
            }
            blank(line)
                && (within(line + 1).is_some() || line.checked_sub(1).and_then(within).is_some())
                && !removes(doc, file, line)
        };
        let moved_new = |line: u32| {
            moves
                .iter()
                .any(|m| (m.new_line..m.new_line + m.lines).contains(&line))
        };
        let leaders = move_leaders(moves);
        let mut noted = vec![[false; 2]; moves.len()];
        let block = self.steps.split_off(start);
        for step in block {
            let (o, n) = step.lines();
            let hide = o != NONE && hidden_old(o);
            let moved = n != NONE && moved_new(n);
            for (i, m) in moves.iter().enumerate() {
                if o != NONE && o == m.old_line && !noted[i][0] {
                    noted[i][0] = true;
                    self.steps.push(Step::MovedAway(i as u32));
                }
                if moved && n == m.new_line && !noted[i][1] && leaders[i] == i as u32 {
                    noted[i][1] = true;
                    self.steps.push(Step::MovedHere(i as u32));
                }
            }
            match step {
                Step::Removed(o) if hide => self.steps.push(Step::Hidden(o)),
                Step::Added(n) if moved => self.steps.push(Step::Moved(n)),
                Step::Paired(o, n) if hide || moved => {
                    self.steps.push(if hide {
                        Step::Hidden(o)
                    } else {
                        Step::Removed(o)
                    });
                    self.steps.push(if moved {
                        Step::Moved(n)
                    } else {
                        Step::Added(n)
                    });
                }
                _ => self.steps.push(step),
            }
        }
    }

    /// Folds the changed lines of each semantic line group into one merged
    /// line when they do not pair line by line and their joined text merges.
    fn fold_groups(&mut self, doc: &DiffDocument, file: u32, start: usize) {
        let Some(groups) = doc
            .semantics(file)
            .map(|s| s.groups())
            .filter(|g| !g.is_empty())
        else {
            return;
        };
        let group_of = |step: Step| {
            if !matches!(
                step,
                Step::Paired(..) | Step::Merged(..) | Step::Removed(_) | Step::Added(_)
            ) {
                return None;
            }
            let (o, n) = step.lines();
            let old = groups.partition_point(|g| g.old.end <= o);
            if groups.get(old).is_some_and(|g| g.old.contains(&o)) {
                return Some(old);
            }
            let new = groups.partition_point(|g| g.new.end <= n);
            groups
                .get(new)
                .is_some_and(|g| g.new.contains(&n))
                .then_some(new)
        };
        let block = self.steps.split_off(start);
        let mut at = 0;
        while at < block.len() {
            let Some(g) = group_of(block[at]) else {
                self.steps.push(block[at]);
                at += 1;
                continue;
            };
            let end = block[at..]
                .iter()
                .position(|&s| group_of(s) != Some(g))
                .map_or(block.len(), |p| at + p);
            let run = &block[at..end];
            let uneven = run.iter().any(|s| !matches!(s, Step::Merged(..)));
            if uneven && run.len() <= MAX_REFLOW_STEPS && composes(doc, file, run) {
                for &step in run {
                    let (old, new) = step.lines();
                    if old != NONE {
                        self.steps.push(Step::Hidden(old));
                    }
                    if new != NONE {
                        self.steps.push(Step::Reflowed(new));
                    }
                }
            } else {
                self.steps.extend_from_slice(run);
            }
            at = end;
        }
    }

    /// Appends the steps of a run of removed lines replaced by added
    /// lines. Returns whether similarity pairing hit its budget.
    fn pair_run(
        &mut self,
        old: &TextStore,
        new: &TextStore,
        olds: Range<u32>,
        news: Range<u32>,
    ) -> bool {
        let (o, n) = (olds.len() as u32, news.len() as u32);
        let (short, long) = (o.min(n), o.max(n));
        let mut offset = 0;
        let mut limited = false;
        if self.options.pairing == PairingMode::Similarity && short > 0 && short != long {
            let offsets = long - short + 1;
            let budget = u64::from(MAX_PAIRING_COMPARISONS)
                + u64::from(PAIRING_COMPARISONS_PER_LINE) * u64::from(o + n);
            // Lines of the shorter side each shift is scored on: all of
            // them when the budget allows, else an even sample.
            let samples = (budget / u64::from(offsets)).min(u64::from(short));
            if samples < MIN_PAIRING_SAMPLES.min(u64::from(short)) {
                limited = true;
            } else {
                let samples = samples as u32;
                let sample = |i: u32| (u64::from(i) * u64::from(short) / u64::from(samples)) as u32;
                // The shorter side shifted by `d` lines into the longer,
                // scaled to a full count of lines.
                let score = |d: u32| -> f32 {
                    let sum: f32 = (0..samples)
                        .map(|i| {
                            let k = sample(i);
                            let (oi, ni) = if o < n { (k, k + d) } else { (k + d, k) };
                            similarity(
                                old.line(olds.start + oi).unwrap_or(""),
                                new.line(news.start + ni).unwrap_or(""),
                            )
                        })
                        .sum();
                    sum * short as f32 / samples as f32
                };
                offset = choose_offset(offsets, short, score);
            }
        }
        let (lead, pairs) = (offset, short);
        let (shift_old, shift_new) = if o > n { (lead, 0) } else { (0, lead) };
        for k in 0..shift_old {
            self.steps.push(Step::Removed(olds.start + k));
        }
        for k in 0..shift_new {
            self.steps.push(Step::Added(news.start + k));
        }
        for k in 0..pairs {
            self.steps.push(Step::Paired(
                olds.start + shift_old + k,
                news.start + shift_new + k,
            ));
        }
        for i in olds.start + shift_old + pairs..olds.end {
            self.steps.push(Step::Removed(i));
        }
        for i in news.start + shift_new + pairs..news.end {
            self.steps.push(Step::Added(i));
        }
        limited
    }

    /// Checks that each change block's steps cover its old and new lines
    /// once each, in order on both sides, and context blocks have none.
    /// O(changed lines).
    pub fn verify_integrity(&self, doc: &DiffDocument) -> Result<(), ComparisonError> {
        if !self.fits(doc) {
            return Err(ComparisonError::Shape);
        }
        let b = doc.blocks();
        for block in 0..b.kind.len() as u32 {
            let bi = block as usize;
            let steps = self.block(block);
            if b.kind[bi] == BlockKind::Context {
                if !steps.is_empty() {
                    return Err(ComparisonError::Block { block });
                }
                continue;
            }
            let (mut o, mut n) = (b.old_store[bi], b.new_store[bi]);
            for step in steps {
                let (so, sn) = step.lines();
                for (line, next) in [(so, &mut o), (sn, &mut n)] {
                    if line == NONE {
                        continue;
                    }
                    if line != *next {
                        return Err(ComparisonError::Block { block });
                    }
                    *next += 1;
                }
            }
            if o != b.old_store[bi] + b.old_len[bi] || n != b.new_store[bi] + b.new_len[bi] {
                return Err(ComparisonError::Block { block });
            }
        }
        Ok(())
    }
}

impl Step {
    /// Old and new store index, [`NONE`] for a missing side.
    pub(crate) fn lines(self) -> (u32, u32) {
        match self {
            Self::Removed(o) | Self::Hidden(o) => (o, NONE),
            Self::Added(n) | Self::Reflowed(n) | Self::Moved(n) => (NONE, n),
            Self::MovedHere(_) | Self::MovedAway(_) => (NONE, NONE),
            Self::Paired(o, n) | Self::Merged(o, n) | Self::Equivalent(o, n) => (o, n),
        }
    }
}

/// Per move, the first of the moves landing close together from the same
/// item, which notes them all at their new place.
pub fn move_leaders(moves: &[Move]) -> Vec<u32> {
    let mut order: Vec<usize> = (0..moves.len()).collect();
    order.sort_by_key(|&i| moves[i].new_line);
    let mut leaders: Vec<u32> = (0..moves.len() as u32).collect();
    let mut group: Option<(usize, u32)> = None;
    for i in order {
        let m = &moves[i];
        match group {
            Some((leader, end))
                if m.from.is_some()
                    && m.from == moves[leader].from
                    && m.from != m.into
                    && m.new_line.saturating_sub(end) <= MOVE_GROUP_GAP =>
            {
                leaders[i] = leader as u32;
                group = Some((leader, end.max(m.new_line + m.lines)));
            }
            _ => group = Some((i, m.new_line + m.lines)),
        }
    }
    leaders
}

pub fn moved_from(doc: &DiffDocument, file: u32, new_line: u32) -> Option<u32> {
    let semantics = doc.semantics(file)?;
    let m = semantics
        .moves()
        .iter()
        .find(|m| (m.new_line..m.new_line + m.lines).contains(&new_line))?;
    let old_line = m.old_line + (new_line - m.new_line);
    if old_line >= m.old_line + m.old_lines || !removes(doc, file, old_line) {
        return None;
    }
    merge_pair(
        doc.text(file, Side::Old).display_line(old_line)?,
        doc.text(file, Side::New).display_line(new_line)?,
        doc.semantic_spans(file, Side::Old, old_line),
        doc.semantic_spans(file, Side::New, new_line),
    )
    .map(|_| old_line)
}

fn removes(doc: &DiffDocument, file: u32, line: u32) -> bool {
    doc.semantic_spans(file, Side::Old, line).iter().any(|s| {
        matches!(
            s.intensity,
            crate::semantic::ChangeIntensity::Novel | crate::semantic::ChangeIntensity::NovelWord
        )
    })
}

/// Whether the lines of `run`, each side joined into one, merge into one
/// readable line.
fn composes(doc: &DiffDocument, file: u32, run: &[Step]) -> bool {
    let mut lines: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
    for &step in run {
        let (o, n) = step.lines();
        if o != NONE {
            lines[0].push(o);
        }
        if n != NONE {
            lines[1].push(n);
        }
    }
    let [old, new] = [Side::Old, Side::New].map(|side| {
        let store = doc.text(file, side);
        join_lines(lines[side as usize].iter().map(|&line| {
            (
                line,
                store.display_line(line).unwrap_or(""),
                doc.semantic_spans(file, side, line),
            )
        }))
    });
    !old.text.trim().is_empty()
        && !new.text.trim().is_empty()
        && reads_as_one(&old)
        && reads_as_one(&new)
        && merge_pair(&old.text, &new.text, &old.spans, &new.spans).is_some()
}

/// A broken [`Comparison`] invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComparisonError {
    /// Computed for a document with other blocks or files.
    Shape,
    Block {
        block: u32,
    },
}

/// The best shift for the shorter side, or 0 (positional) unless one shift
/// beats positional decisively and no other shift comes close to it.
fn choose_offset(offsets: u32, short: u32, score: impl Fn(u32) -> f32) -> u32 {
    let positional = score(0);
    let (mut best, mut best_score, mut runner_up) = (0, positional, f32::NEG_INFINITY);
    for d in 1..offsets {
        let s = score(d);
        if s > best_score {
            runner_up = best_score;
            (best, best_score) = (d, s);
        } else {
            runner_up = runner_up.max(s);
        }
    }
    // A fifth of a matching line per pair, at least a quarter line: a
    // shift needs real resemblance, more than shared punctuation.
    let decisive = best_score - positional >= (0.2 * short as f32).max(0.25);
    let unambiguous = best_score - runner_up > 0.05;
    if best != 0 && decisive && unambiguous {
        best
    } else {
        0
    }
}

/// How alike two lines are, 0 to 1: the share of the longer line covered
/// by their common prefix and suffix once edge spaces are trimmed, both
/// scanned at most [`SIMILARITY_SCAN_BYTES`] deep. Past twice that length
/// the share is of the bytes scanned, so two long lines alike at both ends
/// score as alike rather than near zero.
pub(crate) fn similarity(a: &str, b: &str) -> f32 {
    let edge = [' ', '\t', '\r'];
    let (a, b) = (
        a.trim_matches(edge).as_bytes(),
        b.trim_matches(edge).as_bytes(),
    );
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    let limit = a.len().min(b.len());
    let prefix = a
        .iter()
        .zip(b)
        .take(limit.min(SIMILARITY_SCAN_BYTES))
        .take_while(|(x, y)| x == y)
        .count();
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take((limit - prefix).min(SIMILARITY_SCAN_BYTES))
        .take_while(|(x, y)| x == y)
        .count();
    (prefix + suffix) as f32 / longest.min(2 * SIMILARITY_SCAN_BYTES) as f32
}

/// Line `i` of `store` with spaces normalized per `mode`, keeping a `\r`
/// and whether a newline follows, so line-ending changes stay visible.
fn normalized(store: &TextStore, i: u32, mode: WhitespaceMode) -> String {
    let line = store.line(i).unwrap_or("");
    let (body, cr) = match line.strip_suffix('\r') {
        Some(body) => (body, "\r"),
        None => (line, ""),
    };
    let is_space = |c: char| c == ' ' || c == '\t';
    let mut out = String::with_capacity(body.len() + 2);
    match mode {
        WhitespaceMode::Exact => out.push_str(body),
        WhitespaceMode::IgnoreEdgeSpace => out.push_str(body.trim_matches(is_space)),
        WhitespaceMode::IgnoreSpaceChange => {
            let mut space = false;
            for c in body.trim_end_matches(is_space).chars() {
                if is_space(c) {
                    space = true;
                } else {
                    if space {
                        out.push(' ');
                        space = false;
                    }
                    out.push(c);
                }
            }
        }
        WhitespaceMode::IgnoreAllSpace => out.extend(body.chars().filter(|&c| !is_space(c))),
    }
    out.push_str(cr);
    let eol = i + 1 < store.line_count() || !store.no_newline_at_eof();
    out.push(if eol { '\n' } else { '$' });
    out
}
