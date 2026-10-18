//! How a document's change blocks are shown: which removed line sits
//! beside which added line, and which changes a whitespace policy hides.
//!
//! A [`Comparison`] decorates an exact [`DiffDocument`] without changing
//! it, so apply and export always use the exact edits. It is computed once
//! per document revision and options, then used by every projection
//! rebuild.

use std::ops::Range;

use crate::model::{BlockKind, DiffDocument, Side};
use crate::myers::{diff, intern};
use crate::projection::NONE;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Step {
    Removed(u32),
    Added(u32),
    Paired(u32, u32),
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
            for hunk in doc.files().hunks[file as usize].clone() {
                for block in h.blocks[hunk as usize].clone() {
                    let bi = block as usize;
                    let start = out.steps.len() as u32;
                    if b.kind[bi] == BlockKind::Change {
                        let olds = b.old_store[bi]..b.old_store[bi] + b.old_len[bi];
                        let news = b.new_store[bi]..b.new_store[bi] + b.new_len[bi];
                        let (h, l) = out.align_block(old, new, olds, news, &mut keys);
                        hidden += h;
                        limited += l;
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
            Self::Removed(o) => (o, NONE),
            Self::Added(n) => (NONE, n),
            Self::Paired(o, n) | Self::Equivalent(o, n) => (o, n),
        }
    }
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
