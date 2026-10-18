//! Myers' O(ND) difference algorithm in linear space, as git's xdiff runs
//! it: common prefix and suffix trimmed, items without a match on the other
//! side marked changed up front, the middle snake found by searching from
//! both ends, a cost cap that settles for the furthest-reaching split on
//! pathological inputs, and change groups slid down past equal items so
//! the output is canonical.
//!
//! The search also runs under a work budget linear in the input. Typical
//! diffs finish inside it and come out exactly as above. A diff that would
//! exceed it (millions of lines with changes everywhere, or repetitive
//! lines that match everywhere) is split instead: lines occurring once on
//! each side anchor it (patience style, their longest increasing run),
//! and each region between anchors is diffed again the same way, so most
//! of the work stays exact. A region with no unique line falls back to the
//! search with a small cost cap that it always applies, which costs a
//! bounded amount per item. Every outcome is a valid edit script.
//!
//! Items are interned ids; equal ids are equal items.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};
use std::ops::Range;

/// One run of changed items: `old` replaced by `new`. Either may be empty.
/// Runs are sorted, disjoint, and separated by equal items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub old: Range<u32>,
    pub new: Range<u32>,
}

/// FxHash with a per-process seed. Interning hashes every byte of both
/// texts, and SipHash costs several times more per byte; the seed keeps a
/// crafted input from predicting collisions.
#[derive(Clone, Copy)]
pub(crate) struct FastHash(u64);

impl Default for FastHash {
    fn default() -> Self {
        static SEED: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
        Self(*SEED.get_or_init(|| std::hash::RandomState::new().hash_one(0u8)))
    }
}

impl BuildHasher for FastHash {
    type Hasher = FastHasher;
    fn build_hasher(&self) -> FastHasher {
        FastHasher(self.0)
    }
}

pub(crate) struct FastHasher(u64);

impl FastHasher {
    #[inline]
    fn mix(&mut self, word: u64) {
        const K: u64 = 0x517c_c1b7_2722_0a95;
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(K);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.mix(u64::from_le_bytes(*word));
        }
        if !rest.is_empty() {
            let mut last = [0u8; 8];
            last[..rest.len()].copy_from_slice(rest);
            self.mix(u64::from_le_bytes(last) ^ ((rest.len() as u64) << 59));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.mix(u64::from(i));
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.mix(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        // A final avalanche so hashbrown's top bits depend on every word.
        let mut x = self.0;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^ (x >> 33)
    }
}

/// Interns items to dense ids shared by both sides.
pub(crate) fn intern<'a, T: Hash + Eq + ?Sized + 'a>(
    old: impl Iterator<Item = &'a T>,
    new: impl Iterator<Item = &'a T>,
) -> (Vec<u32>, Vec<u32>) {
    let mut ids: HashMap<&T, u32, FastHash> = HashMap::default();
    let mut id = |item: &'a T| {
        let next = ids.len() as u32;
        *ids.entry(item).or_insert(next)
    };
    let old: Vec<u32> = old.map(&mut id).collect();
    let new: Vec<u32> = new.map(&mut id).collect();
    (old, new)
}

/// The changes turning `old` into `new`.
pub fn diff(old: &[u32], new: &[u32]) -> Vec<Change> {
    diff_with(old, new, Budget::LINEAR)
}

/// How much search work a region may spend before it is split at anchors
/// instead: `base` plus `per_item` units per item. A unit is one diagonal
/// step or one matched item followed. Regions of at most `small` items
/// are searched directly whatever they cost.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Budget {
    pub(crate) per_item: u64,
    pub(crate) base: u64,
    pub(crate) small: i64,
}

impl Budget {
    /// Covers every diff of a few thousand lines, and large diffs whose
    /// changes are sparse, so those come out as plain Myers.
    pub(crate) const LINEAR: Self = Self {
        per_item: 16,
        base: 1 << 22,
        // At most a few million units.
        small: 2_048,
    };

    /// Never splits: the search exactly as xdiff runs it, for comparing.
    #[cfg(test)]
    pub(crate) const UNLIMITED: Self = Self {
        per_item: u64::MAX,
        base: u64::MAX,
        small: i64::MAX,
    };

    /// Splits at the first chance; for testing the anchored path on small
    /// inputs.
    #[cfg(test)]
    pub(crate) const NONE: Self = Self {
        per_item: 0,
        base: 0,
        small: 0,
    };

    fn of(self, items: usize) -> i64 {
        let units = self
            .base
            .saturating_add(self.per_item.saturating_mul(items as u64));
        units.min(i64::MAX as u64) as i64
    }
}

/// [`diff`] under `budget`.
pub(crate) fn diff_with(old: &[u32], new: &[u32], budget: Budget) -> Vec<Change> {
    let mut removed = vec![false; old.len()];
    let mut added = vec![false; new.len()];
    mark(old, new, &mut removed, &mut added, budget);
    slide(old, &mut removed);
    slide(new, &mut added);
    changes(&removed, &added)
}

/// Marks changed items. Items absent from the other side cannot be part
/// of any common subsequence, so they are marked directly and the search
/// runs on the rest, which is much shorter when the sides differ a lot.
fn mark(old: &[u32], new: &[u32], removed: &mut [bool], added: &mut [bool], budget: Budget) {
    let ids = old
        .iter()
        .chain(new)
        .copied()
        .max()
        .map_or(0, |m| m as usize + 1);
    let mut in_old = vec![false; ids];
    let mut in_new = vec![false; ids];
    for &id in old {
        in_old[id as usize] = true;
    }
    for &id in new {
        in_new[id as usize] = true;
    }
    let keep_old: Vec<u32> = (0..old.len() as u32)
        .filter(|&i| in_new[old[i as usize] as usize])
        .collect();
    let keep_new: Vec<u32> = (0..new.len() as u32)
        .filter(|&i| in_old[new[i as usize] as usize])
        .collect();
    for (i, &id) in old.iter().enumerate() {
        removed[i] = !in_new[id as usize];
    }
    for (i, &id) in new.iter().enumerate() {
        added[i] = !in_old[id as usize];
    }
    drop((in_old, in_new));
    let a: Vec<u32> = keep_old.iter().map(|&i| old[i as usize]).collect();
    let b: Vec<u32> = keep_new.iter().map(|&i| new[i as usize]).collect();
    let mut sub_removed = vec![false; a.len()];
    let mut sub_added = vec![false; b.len()];
    solve(&a, &b, ids, &mut sub_removed, &mut sub_added, budget);
    for (k, &i) in keep_old.iter().enumerate() {
        removed[i as usize] = sub_removed[k];
    }
    for (k, &i) in keep_new.iter().enumerate() {
        added[i as usize] = sub_added[k];
    }
}

/// Searches `a` against `b` within `budget`, splitting at anchors where the
/// search would cost more. `ids` bounds the item ids.
fn solve(
    a: &[u32],
    b: &[u32],
    ids: usize,
    removed: &mut [bool],
    added: &mut [bool],
    budget: Budget,
) {
    let mut search = Search::new(a, b);
    search.work = budget.of(a.len() + b.len());
    let whole = (0, a.len() as i64, 0, b.len() as i64);
    if search.run(whole, removed, added).is_ok() {
        return;
    }
    removed.fill(false);
    added.fill(false);
    let left = std::mem::take(&mut search.left);
    Anchors::new(ids).run(&mut search, left, removed, added, budget);
}

/// How many times a region may be split at anchors found inside the
/// previous split's regions, so a chain of splits that each peel off
/// little cannot add up to quadratic work.
const MAX_ANCHOR_DEPTH: u32 = 6;

/// The cost cap of the search over a region without unique lines, applied
/// throughout: each split then costs at most about the square of this,
/// and moves at least this far.
const FALLBACK_COST_CAP: i64 = 64;

/// Splits regions at lines that occur once on each side.
struct Anchors {
    /// Per id: occurrences in the region's old and new items, and where in
    /// the new items it last occurred. Zeroed again after each use.
    count_old: Vec<u32>,
    count_new: Vec<u32>,
    at_new: Vec<u32>,
}

impl Anchors {
    fn new(ids: usize) -> Self {
        Self {
            count_old: vec![0; ids],
            count_new: vec![0; ids],
            at_new: vec![0; ids],
        }
    }

    /// Diffs `regions`, which the budgeted search left unfinished.
    fn run(
        &mut self,
        search: &mut Search,
        regions: Vec<Region>,
        removed: &mut [bool],
        added: &mut [bool],
        budget: Budget,
    ) {
        let (a, b) = (search.a, search.b);
        // (x0, x1, y0, y1, depth, whether the budgeted search already ran
        // out on it)
        let mut regions: Vec<_> = regions
            .into_iter()
            .map(|(x0, x1, y0, y1)| (x0, x1, y0, y1, 0u32, true))
            .collect();
        while let Some((mut x0, mut x1, mut y0, mut y1, depth, tried)) = regions.pop() {
            while x0 < x1 && y0 < y1 && a[x0 as usize] == b[y0 as usize] {
                x0 += 1;
                y0 += 1;
            }
            while x0 < x1 && y0 < y1 && a[x1 as usize - 1] == b[y1 as usize - 1] {
                x1 -= 1;
                y1 -= 1;
            }
            if x0 == x1 {
                added[y0 as usize..y1 as usize].fill(true);
                continue;
            }
            if y0 == y1 {
                removed[x0 as usize..x1 as usize].fill(true);
                continue;
            }
            let region = (x0, x1, y0, y1);
            let len = (x1 - x0) + (y1 - y0);
            if len <= budget.small {
                search.work = i64::MAX;
                let _ = search.run(region, removed, added);
                continue;
            }
            if !tried {
                search.work = budget.of(len as usize);
                if search.run(region, removed, added).is_err() {
                    // What it finished stays; the rest is split.
                    let left = std::mem::take(&mut search.left);
                    regions.extend(
                        left.into_iter()
                            .map(|(x0, x1, y0, y1)| (x0, x1, y0, y1, depth, true)),
                    );
                }
                continue;
            }
            let anchors = if depth < MAX_ANCHOR_DEPTH {
                self.unique(a, b, region)
            } else {
                Vec::new()
            };
            if anchors.is_empty() {
                search.capped(region, FALLBACK_COST_CAP, removed, added);
                continue;
            }
            let (mut x, mut y) = (x0, y0);
            for (ax, ay) in anchors {
                regions.push((x, i64::from(ax), y, i64::from(ay), depth + 1, false));
                (x, y) = (i64::from(ax) + 1, i64::from(ay) + 1);
            }
            regions.push((x, x1, y, y1, depth + 1, false));
        }
    }

    /// Items occurring exactly once in each side of the region, as
    /// `(old, new)` positions: the longest run increasing on both sides.
    fn unique(
        &mut self,
        a: &[u32],
        b: &[u32],
        (x0, x1, y0, y1): (i64, i64, i64, i64),
    ) -> Vec<(u32, u32)> {
        let (olds, news) = (&a[x0 as usize..x1 as usize], &b[y0 as usize..y1 as usize]);
        for &id in olds {
            self.count_old[id as usize] += 1;
        }
        for (y, &id) in (y0 as u32..).zip(news) {
            self.count_new[id as usize] += 1;
            self.at_new[id as usize] = y;
        }
        let pairs: Vec<(u32, u32)> = (x0 as u32..)
            .zip(olds)
            .filter(|&(_, &id)| {
                self.count_old[id as usize] == 1 && self.count_new[id as usize] == 1
            })
            .map(|(x, &id)| (x, self.at_new[id as usize]))
            .collect();
        for &id in olds {
            self.count_old[id as usize] = 0;
        }
        for &id in news {
            self.count_new[id as usize] = 0;
        }
        longest_increasing(&pairs)
    }
}

/// The longest subsequence of `pairs` (sorted by their first value) whose
/// second values increase, by patience sorting.
fn longest_increasing(pairs: &[(u32, u32)]) -> Vec<(u32, u32)> {
    // `tails[k]`: the pair ending the best run of length k + 1 so far.
    let mut tails: Vec<usize> = Vec::new();
    let mut before = vec![usize::MAX; pairs.len()];
    for (i, &(_, y)) in pairs.iter().enumerate() {
        let k = tails.partition_point(|&t| pairs[t].1 < y);
        if k > 0 {
            before[i] = tails[k - 1];
        }
        if k == tails.len() {
            tails.push(i);
        } else {
            tails[k] = i;
        }
    }
    let mut out = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied().unwrap_or(usize::MAX);
    while at != usize::MAX {
        out.push(pairs[at]);
        at = before[at];
    }
    out.reverse();
    out
}

/// Smallest cost after which the search settles for an approximate split.
const MIN_COST_CAP: i64 = 256;

/// A box of the search: `[x0, x1)` of `a` against `[y0, y1)` of `b`.
type Region = (i64, i64, i64, i64);

/// The search ran out of work units.
struct OverBudget;

struct Search<'a> {
    a: &'a [u32],
    b: &'a [u32],
    /// Furthest x reached on each diagonal `x - y`, forward and backward,
    /// indexed by diagonal plus `offset`.
    forward: Vec<i64>,
    backward: Vec<i64>,
    offset: i64,
    cost_cap: i64,
    /// Apply the cost cap even to halves of an exact split, which xdiff
    /// searches without one.
    always_cap: bool,
    /// Work units left; the search stops once it is spent.
    work: i64,
    /// Boxes left to search, reused between runs.
    stack: Vec<(i64, i64, i64, i64, bool)>,
    /// The boxes a run that ran out of work left unsearched, each
    /// independent of the rest and unmarked.
    left: Vec<Region>,
}

struct Split {
    x: i64,
    y: i64,
    /// Whether each half must be diffed minimally (it was split exactly).
    min_lo: bool,
    min_hi: bool,
}

impl<'a> Search<'a> {
    fn new(a: &'a [u32], b: &'a [u32]) -> Self {
        let (n, m) = (a.len() as i64, b.len() as i64);
        let diagonals = (n + m + 3) as usize;
        let cost_cap = ((n + m + 3) as f64).sqrt() as i64;
        Self {
            a,
            b,
            forward: vec![0; diagonals],
            backward: vec![0; diagonals],
            offset: m + 1,
            cost_cap: cost_cap.max(MIN_COST_CAP),
            always_cap: false,
            work: i64::MAX,
            stack: Vec::new(),
            left: Vec::new(),
        }
    }

    /// Searches `region` with `cap` applied throughout and no budget.
    fn capped(&mut self, region: Region, cap: i64, removed: &mut [bool], added: &mut [bool]) {
        let saved = (self.cost_cap, self.always_cap);
        (self.cost_cap, self.always_cap, self.work) = (cap, true, i64::MAX);
        let _ = self.run(region, removed, added);
        (self.cost_cap, self.always_cap) = saved;
    }

    /// Marks the changed items of `region`, or stops once the work budget
    /// is spent and leaves the boxes it did not finish in `left`. The
    /// boxes it finished are marked for good, since every box of the
    /// search is independent of the others.
    fn run(
        &mut self,
        (x0, x1, y0, y1): Region,
        removed: &mut [bool],
        added: &mut [bool],
    ) -> Result<(), OverBudget> {
        // An explicit stack: recursion depth can reach the edit distance.
        let mut stack = std::mem::take(&mut self.stack);
        stack.clear();
        stack.push((x0, x1, y0, y1, false));
        let mut result = Ok(());
        while let Some((mut x0, mut x1, mut y0, mut y1, need_min)) = stack.pop() {
            let from = x0 + x1;
            while x0 < x1 && y0 < y1 && self.a[x0 as usize] == self.b[y0 as usize] {
                x0 += 1;
                y0 += 1;
            }
            while x0 < x1 && y0 < y1 && self.a[x1 as usize - 1] == self.b[y1 as usize - 1] {
                x1 -= 1;
                y1 -= 1;
            }
            self.work -= (x0 + x1 - from).abs();
            if x0 == x1 {
                added[y0 as usize..y1 as usize].fill(true);
            } else if y0 == y1 {
                removed[x0 as usize..x1 as usize].fill(true);
            } else {
                match self.split(x0, x1, y0, y1, need_min) {
                    Ok(split) => {
                        stack.push((split.x, x1, split.y, y1, split.min_hi));
                        stack.push((x0, split.x, y0, split.y, split.min_lo));
                    }
                    Err(over) => {
                        self.left.clear();
                        self.left.push((x0, x1, y0, y1));
                        self.left
                            .extend(stack.drain(..).map(|(x0, x1, y0, y1, _)| (x0, x1, y0, y1)));
                        result = Err(over);
                        break;
                    }
                }
            }
        }
        self.stack = stack;
        result
    }

    fn f(&mut self, d: i64) -> &mut i64 {
        &mut self.forward[(d + self.offset) as usize]
    }

    fn bk(&mut self, d: i64) -> &mut i64 {
        &mut self.backward[(d + self.offset) as usize]
    }

    /// The middle snake of the box `[x0, x1) x [y0, y1)`, whose corners do
    /// not match (the caller trimmed them).
    fn split(
        &mut self,
        x0: i64,
        x1: i64,
        y0: i64,
        y1: i64,
        need_min: bool,
    ) -> Result<Split, OverBudget> {
        let (a, b) = (self.a, self.b);
        let (dmin, dmax) = (x0 - y1, x1 - y0);
        let (fmid, bmid) = (x0 - y0, x1 - y1);
        let odd = (fmid - bmid) & 1 != 0;
        let (mut fmin, mut fmax, mut bmin, mut bmax) = (fmid, fmid, bmid, bmid);
        *self.f(fmid) = x0;
        *self.bk(bmid) = x1;
        let mut cost = 1;
        loop {
            // Extend the forward paths by one edit.
            if fmin > dmin {
                fmin -= 1;
                *self.f(fmin - 1) = -1;
            } else {
                fmin += 1;
            }
            if fmax < dmax {
                fmax += 1;
                *self.f(fmax + 1) = -1;
            } else {
                fmax -= 1;
            }
            let mut d = fmax;
            while d >= fmin {
                let (left, right) = (*self.f(d - 1), *self.f(d + 1));
                let mut x = if left >= right { left + 1 } else { right };
                let mut y = x - d;
                let from = x;
                while x < x1 && y < y1 && a[x as usize] == b[y as usize] {
                    x += 1;
                    y += 1;
                }
                self.work -= 1 + x - from;
                *self.f(d) = x;
                if odd && bmin <= d && d <= bmax && *self.bk(d) <= x {
                    return Ok(Split {
                        x,
                        y,
                        min_lo: true,
                        min_hi: true,
                    });
                }
                d -= 2;
            }

            // Extend the backward paths by one edit.
            if bmin > dmin {
                bmin -= 1;
                *self.bk(bmin - 1) = i64::MAX;
            } else {
                bmin += 1;
            }
            if bmax < dmax {
                bmax += 1;
                *self.bk(bmax + 1) = i64::MAX;
            } else {
                bmax -= 1;
            }
            let mut d = bmax;
            while d >= bmin {
                let (left, right) = (*self.bk(d - 1), *self.bk(d + 1));
                let mut x = if left < right { left } else { right - 1 };
                let mut y = x - d;
                let from = x;
                while x > x0 && y > y0 && a[x as usize - 1] == b[y as usize - 1] {
                    x -= 1;
                    y -= 1;
                }
                self.work -= 1 + from - x;
                *self.bk(d) = x;
                if !odd && fmin <= d && d <= fmax && x <= *self.f(d) {
                    return Ok(Split {
                        x,
                        y,
                        min_lo: true,
                        min_hi: true,
                    });
                }
                d -= 2;
            }

            if self.work < 0 {
                return Err(OverBudget);
            }
            if (self.always_cap || !need_min) && cost >= self.cost_cap {
                return Ok(self.best_split(x0, x1, y0, y1, (fmin, fmax), (bmin, bmax)));
            }
            cost += 1;
        }
    }

    /// Too expensive to finish: split where the forward or the backward
    /// search got furthest, whichever covers more of the box.
    fn best_split(
        &mut self,
        x0: i64,
        x1: i64,
        y0: i64,
        y1: i64,
        (fmin, fmax): (i64, i64),
        (bmin, bmax): (i64, i64),
    ) -> Split {
        let (mut fbest, mut fbest_x) = (-1, -1);
        let mut d = fmax;
        while d >= fmin {
            let mut x = (*self.f(d)).min(x1);
            let mut y = x - d;
            if y1 < y {
                x = y1 + d;
                y = y1;
            }
            if fbest < x + y {
                fbest = x + y;
                fbest_x = x;
            }
            d -= 2;
        }
        let (mut bbest, mut bbest_x) = (i64::MAX, i64::MAX);
        let mut d = bmax;
        while d >= bmin {
            let mut x = (*self.bk(d)).max(x0);
            let mut y = x - d;
            if y < y0 {
                x = y0 + d;
                y = y0;
            }
            if x + y < bbest {
                bbest = x + y;
                bbest_x = x;
            }
            d -= 2;
        }
        if (x1 + y1) - bbest < fbest - (x0 + y0) {
            Split {
                x: fbest_x,
                y: fbest - fbest_x,
                min_lo: true,
                min_hi: false,
            }
        } else {
            Split {
                x: bbest_x,
                y: bbest - bbest_x,
                min_lo: false,
                min_hi: true,
            }
        }
    }
}

/// Slides every run of changed items as far down as equal items allow, so
/// equivalent scripts come out the same: in `a b a` with one `a` removed,
/// the second goes.
fn slide(items: &[u32], changed: &mut [bool]) {
    let n = items.len();
    let mut start = 0;
    while start < n {
        if !changed[start] {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < n && changed[end] {
            end += 1;
        }
        // Moving the run down one item keeps the unchanged items in
        // order: the item leaving the run's top equals the one entering
        // below it.
        while end < n && items[start] == items[end] {
            changed[start] = false;
            changed[end] = true;
            start += 1;
            end += 1;
            while end < n && changed[end] {
                end += 1;
            }
        }
        start = end;
    }
}

/// Pairs runs of removed and added items. Both sides keep the same number
/// of unchanged items, so walking them in step lines the runs up.
fn changes(removed: &[bool], added: &[bool]) -> Vec<Change> {
    let (n, m) = (removed.len(), added.len());
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < n || j < m {
        if (i < n && removed[i]) || (j < m && added[j]) {
            let (si, sj) = (i, j);
            while i < n && removed[i] {
                i += 1;
            }
            while j < m && added[j] {
                j += 1;
            }
            out.push(Change {
                old: si as u32..i as u32,
                new: sj as u32..j as u32,
            });
        } else {
            i += 1;
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Budget, Change, diff, diff_with, intern};
    use proptest::prelude::*;

    fn chars(old: &str, new: &str) -> Vec<Change> {
        let old: Vec<String> = old.chars().map(String::from).collect();
        let new: Vec<String> = new.chars().map(String::from).collect();
        let (a, b) = intern(
            old.iter().map(|s| s.as_str()),
            new.iter().map(|s| s.as_str()),
        );
        diff(&a, &b)
    }

    /// The changes as `-old+new` runs, for table tests.
    fn script(old: &str, new: &str) -> String {
        let (o, n): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
        chars(old, new)
            .iter()
            .map(|c| {
                let take = |s: &[char], r: &std::ops::Range<u32>| -> String {
                    s[r.start as usize..r.end as usize].iter().collect()
                };
                format!("{}:-{}+{}", c.old.start, take(&o, &c.old), take(&n, &c.new))
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn scripts_are_minimal_and_slid_down() {
        let cases = [
            ("abc", "abc", ""),
            ("", "ab", "0:-+ab"),
            ("ab", "", "0:-ab+"),
            ("abcabba", "cbabac", "0:-ab+ 3:-a+ 5:-+a 7:-+c"),
            ("aba", "ab", "2:-a+"),
            ("xaay", "xay", "2:-a+"),
            ("abc", "axc", "1:-b+x"),
        ];
        for (old, new, expected) in cases {
            assert_eq!(script(old, new), expected, "{old} -> {new}");
        }
    }

    /// Applies `changes` to `old` taking replacement items from `new`.
    fn apply(old: &[u32], new: &[u32], changes: &[Change]) -> Vec<u32> {
        let mut out = Vec::new();
        let mut at = 0;
        for c in changes {
            out.extend_from_slice(&old[at..c.old.start as usize]);
            out.extend_from_slice(&new[c.new.start as usize..c.new.end as usize]);
            at = c.old.end as usize;
        }
        out.extend_from_slice(&old[at..]);
        out
    }

    /// Length of the longest common subsequence, by dynamic programming.
    fn lcs(a: &[u32], b: &[u32]) -> usize {
        let mut row = vec![0usize; b.len() + 1];
        for &x in a {
            let mut prev = 0;
            for (j, &y) in b.iter().enumerate() {
                let keep = row[j + 1];
                row[j + 1] = if x == y {
                    prev + 1
                } else {
                    row[j + 1].max(row[j])
                };
                prev = keep;
            }
        }
        row[b.len()]
    }

    proptest! {
        #[test]
        fn changes_rebuild_new_with_a_minimal_script(
            a in prop::collection::vec(0u32..4, 0..40),
            b in prop::collection::vec(0u32..4, 0..40),
        ) {
            let changes = diff(&a, &b);
            prop_assert_eq!(apply(&a, &b, &changes), b.clone());
            let edits: u32 = changes.iter().map(|c| c.old.len() as u32 + c.new.len() as u32).sum();
            prop_assert_eq!(edits as usize, a.len() + b.len() - 2 * lcs(&a, &b));
            for pair in changes.windows(2) {
                prop_assert!(pair[0].old.end < pair[1].old.start);
            }
        }

        // Catches the anchored split (taken when a search would cost more
        // than its budget) dropping, duplicating, or misplacing items: with
        // no budget every region splits, yet the script still turns old
        // into new.
        #[test]
        fn anchored_splits_still_rebuild_new(
            a in prop::collection::vec(0u32..6, 0..60),
            b in prop::collection::vec(0u32..6, 0..60),
        ) {
            let changes = diff_with(&a, &b, Budget::NONE);
            prop_assert_eq!(apply(&a, &b, &changes), b.clone());
            for pair in changes.windows(2) {
                prop_assert!(pair[0].old.end < pair[1].old.start);
            }
        }

        // Catches anchors that cross each other or a region diffed past
        // its anchors: when no item repeats, every common item is an
        // anchor, and the split script is as short as plain Myers'.
        #[test]
        fn anchored_splits_of_distinct_items_stay_minimal(
            a in prop::sample::subsequence((0u32..48).collect::<Vec<_>>(), 0..48),
            b in prop::sample::subsequence((0u32..48).collect::<Vec<_>>(), 0..48)
                .prop_shuffle(),
        ) {
            let edits = |changes: &[Change]| -> usize {
                changes.iter().map(|c| c.old.len() + c.new.len()).sum()
            };
            let anchored = diff_with(&a, &b, Budget::NONE);
            prop_assert_eq!(apply(&a, &b, &anchored), b.clone());
            prop_assert_eq!(edits(&anchored), a.len() + b.len() - 2 * lcs(&a, &b));
        }
    }
}
