use std::collections::HashSet;
use std::ops::Range;
use std::time::Instant;

use diffy_diff::myers::{self, FastHash};

use crate::Fallback;
use crate::syntax::{Class, NONE, Tree};

type HashMap<K, V> = std::collections::HashMap<K, V, FastHash>;

const WORD: i64 = 4;
const PUNCT: i64 = 1;
const DELIMITER: i64 = 1;
const GAMMA: i64 = 1;
const DETACHED: i64 = 2;
const LINE_SPLIT: i64 = 1;
const RENAME_SHARE: i64 = 4;
const MOVE: i64 = 64;
const MOVE_ITEM: i64 = 8;
const MOVE_LOSS: (i64, i64) = (3, 2);
const CANDIDATES: usize = 4;
const SIGNATURE_LEAVES: usize = 64;
const MAX_PAIR_CELLS: usize = 1 << 16;
const MAX_HASH_HITS: usize = 64;
const CLOCK_STRIDE: u64 = 1 << 16;

/// Which old node each new node corresponds to and back. Every link sits
/// under linked ancestors that correspond to each other, or under the
/// root of a move; links never cross each other outside a move.
pub(crate) struct Matching {
    pub(crate) partner: [Vec<u32>; 2],
    /// Roots of moved subtrees, old and new.
    pub(crate) moves: Vec<(u32, u32)>,
}

pub(crate) fn run(
    old: &Tree<'_>,
    new: &Tree<'_>,
    max_work: u64,
    deadline: Instant,
) -> Result<Matching, Fallback> {
    let mut m = Matcher {
        trees: [old, new],
        partner: [vec![NONE; old.nodes.len()], vec![NONE; new.nodes.len()]],
        parents: [parents(old), parents(new)],
        weights: [weights(old), weights(new)],
        lines: [lines(old), lines(new)],
        names: [names(old), names(new)],
        items: [items(old), items(new)],
        hashes: [Default::default(), Default::default()],
        pins: Vec::new(),
        pins_new: Vec::new(),
        memo: HashMap::default(),
        wrapped: HashMap::default(),
        signatures: Default::default(),
        moves: Vec::new(),
        work: 0,
        max_work,
        clock_at: CLOCK_STRIDE,
        deadline,
    };
    if old.nodes.is_empty() || new.nodes.is_empty() {
        return Ok(m.finish());
    }
    m.index()?;
    m.apply(0, 0, GAMMA)?;
    m.relocate()?;
    m.scope()?;
    m.monotone()?;
    Ok(m.finish())
}

struct Matcher<'t, 'a> {
    trees: [&'t Tree<'a>; 2],
    partner: [Vec<u32>; 2],
    parents: [Vec<u32>; 2],
    weights: [Weights; 2],
    lines: [Lines; 2],
    names: [HashSet<&'a str>; 2],
    items: [HashSet<(&'static str, &'a str)>; 2],
    hashes: [HashMap<u64, Vec<u32>>; 2],
    pins: Vec<(u32, u32)>,
    pins_new: Vec<(u32, u32)>,
    memo: HashMap<(u32, u32, i64), i64>,
    wrapped: HashMap<(u32, u32, u32, i64), i64>,
    signatures: [HashMap<u32, Vec<u64>>; 2],
    moves: Vec<(u32, u32)>,
    work: u64,
    max_work: u64,
    clock_at: u64,
    deadline: Instant,
}

struct Lines {
    starts: Vec<u32>,
    hashes: Vec<u64>,
}

struct Weights {
    own: Vec<i64>,
    sum: Vec<i64>,
}

type Charge = (i64, i64);

#[derive(Clone, Copy)]
enum Act {
    Equal(u32, u32),
    Pair(u32, u32, i64),
    Wrap(usize, usize, u32),
    Unwrap(usize, usize, u32),
}

#[derive(Clone, Copy)]
struct Cell {
    old: (usize, usize),
    new: (usize, usize),
    value: i64,
    act: Act,
}

impl Matcher<'_, '_> {
    fn finish(self) -> Matching {
        let mut moves = self.moves;
        moves.sort_unstable();
        Matching {
            partner: self.partner,
            moves,
        }
    }

    fn spend(&mut self, units: usize) -> Result<(), Fallback> {
        self.work += units as u64;
        if self.work > self.max_work {
            return Err(Fallback::Budget);
        }
        if self.work >= self.clock_at {
            self.clock_at = self.work + CLOCK_STRIDE;
            if Instant::now() > self.deadline {
                return Err(Fallback::Timeout);
            }
        }
        Ok(())
    }

    fn next(&self, side: usize, node: u32) -> u32 {
        self.trees[side].nodes[node as usize].next
    }

    fn hash(&self, side: usize, node: u32) -> u64 {
        self.trees[side].nodes[node as usize].hash
    }

    fn is_list(&self, side: usize, node: u32) -> bool {
        self.trees[side].is_list(node)
    }

    fn inside(&self, side: usize, node: u32, root: u32) -> bool {
        root < node && node < self.next(side, root)
    }

    fn total(&self, side: usize, node: u32) -> i64 {
        let sum = &self.weights[side].sum;
        sum[self.next(side, node) as usize] - sum[node as usize]
    }

    fn own(&self, side: usize, node: u32) -> i64 {
        self.weights[side].own[node as usize]
    }

    fn link(&mut self, a: u32, b: u32) {
        let old_b = self.partner[0][a as usize];
        if old_b != NONE {
            self.partner[1][old_b as usize] = NONE;
        }
        let old_a = self.partner[1][b as usize];
        if old_a != NONE {
            self.partner[0][old_a as usize] = NONE;
        }
        self.partner[0][a as usize] = b;
        self.partner[1][b as usize] = a;
    }

    fn unlink(&mut self, a: u32) {
        let b = self.partner[0][a as usize];
        if b != NONE {
            self.partner[1][b as usize] = NONE;
            self.partner[0][a as usize] = NONE;
        }
    }

    fn free(&self, side: usize, node: u32) -> bool {
        let end = self.next(side, node);
        (node..end).all(|k| self.partner[side][k as usize] == NONE)
    }

    fn equal(&self, a: u32, b: u32) -> bool {
        self.hash(0, a) == self.hash(1, b) && self.next(0, a) - a == self.next(1, b) - b
    }

    fn same(&mut self, a: u32, b: u32) -> bool {
        let (old, new) = (self.trees[0], self.trees[1]);
        let len = self.next(0, a) - a;
        if !self.equal(a, b) {
            return false;
        }
        let shapes_match = (0..len).all(|k| {
            let (x, y) = (&old.nodes[(a + k) as usize], &new.nodes[(b + k) as usize]);
            x.kind == y.kind && x.class == y.class && x.next - (a + k) == y.next - (b + k)
        });
        if !shapes_match {
            return false;
        }
        for k in 0..len {
            self.link(a + k, b + k);
        }
        true
    }

    fn pairable(&self, a: u32, b: u32) -> bool {
        let (ta, tb) = (self.trees[0], self.trees[1]);
        let (na, nb) = (&ta.nodes[a as usize], &tb.nodes[b as usize]);
        na.class == Class::List
            && nb.class == Class::List
            && na.kind == nb.kind
            && ta.text(na.open.clone()) == tb.text(nb.open.clone())
            && ta.text(na.close.clone()) == tb.text(nb.close.clone())
    }

    fn significant(&self, side: usize, node: u32) -> bool {
        let tree = self.trees[side];
        let n = &tree.nodes[node as usize];
        n.class == Class::List
            || tree
                .text(n.span.clone())
                .bytes()
                .any(|b| b.is_ascii_alphanumeric() || b >= 0x80)
    }

    fn bracketed(&self, side: usize, node: u32) -> bool {
        !self.trees[side].nodes[node as usize].open.is_empty()
    }

    fn index(&mut self) -> Result<(), Fallback> {
        let (old, new) = (self.trees[0], self.trees[1]);
        self.spend(old.nodes.len() + new.nodes.len())?;
        let mut counts: [HashMap<u64, (u32, u32)>; 2] = Default::default();
        for side in [0, 1] {
            let tree = self.trees[side];
            for node in 1..tree.nodes.len() as u32 {
                if !self.significant(side, node) {
                    continue;
                }
                let hash = self.hash(side, node);
                self.hashes[side].entry(hash).or_default().push(node);
                if !self.bracketed(side, node) {
                    let entry = counts[side].entry(hash).or_insert((0, node));
                    entry.0 += 1;
                }
            }
        }
        let [olds, news] = counts;
        let mut unique: Vec<(u32, u32)> = olds
            .iter()
            .filter(|(_, (n, _))| *n == 1)
            .filter_map(|(hash, &(_, o))| match news.get(hash) {
                Some(&(1, n)) => Some((o, n)),
                _ => None,
            })
            .filter(|&(o, n)| self.equal(o, n))
            .collect();
        unique.sort_unstable_by_key(|&(o, _)| (std::cmp::Reverse(self.next(0, o) - o), o));
        let mut covered = [vec![false; old.nodes.len()], vec![false; new.nodes.len()]];
        for (o, n) in unique {
            if covered[0][o as usize] || covered[1][n as usize] {
                continue;
            }
            for (side, node) in [(0, o), (1, n)] {
                let end = self.next(side, node) as usize;
                covered[side][node as usize..end].fill(true);
            }
            self.pins.push((o, n));
        }
        self.pins.sort_unstable();
        self.pins_new = self.pins.iter().map(|&(o, n)| (n, o)).collect();
        self.pins_new.sort_unstable();
        Ok(())
    }

    fn pins_in(&self, side: usize, node: u32) -> &[(u32, u32)] {
        let pins = if side == 0 {
            &self.pins
        } else {
            &self.pins_new
        };
        let end = self.next(side, node);
        let from = pins.partition_point(|&(x, _)| x < node);
        let to = pins.partition_point(|&(x, _)| x < end);
        &pins[from..to]
    }

    fn split(&self, x: u32, y: u32) -> i64 {
        let line = |side: usize, at: u32| {
            let lines = &self.lines[side];
            lines.hashes[lines.starts.partition_point(|&s| s <= at).saturating_sub(1)]
        };
        if line(0, x) == line(1, y) {
            0
        } else {
            LINE_SPLIT
        }
    }

    fn detached(&self, x: u32, y: u32) -> i64 {
        let brackets = if self.bracketed(0, x) || self.bracketed(1, y) {
            self.own(0, x) + self.own(1, y)
        } else {
            0
        };
        DETACHED + brackets
    }

    fn renamable(&self, a: u32, b: u32) -> bool {
        let (ta, tb) = (self.trees[0], self.trees[1]);
        let (na, nb) = (&ta.nodes[a as usize], &tb.nodes[b as usize]);
        let (p, q) = (ta.text(na.span.clone()), tb.text(nb.span.clone()));
        na.name && nb.name && p != q && !self.names[1].contains(p) && !self.names[0].contains(q)
    }

    fn leaf_value(&self, a: u32, b: u32, charge: Charge) -> i64 {
        let (ta, tb) = (self.trees[0], self.trees[1]);
        let (na, nb) = (&ta.nodes[a as usize], &tb.nodes[b as usize]);
        if na.class != nb.class || na.class == Class::List {
            return 0;
        }
        let both = self.own(0, a) + self.own(1, b);
        let full = (1 + charge.0) * self.own(0, a) + (1 + charge.1) * self.own(1, b)
            - self.split(na.span.start, nb.span.start);
        let (p, q) = (ta.text(na.span.clone()), tb.text(nb.span.clone()));
        match na.class {
            Class::Plain if p == q => full,
            Class::Plain if self.renamable(a, b) => full - both / RENAME_SHARE,
            Class::Text | Class::Comment if na.kind == nb.kind => {
                if p == q {
                    return full;
                }
                let common = common_words(p, q);
                ((2 + charge.0 + charge.1) * WORD * common).min(full)
            }
            _ => 0,
        }
    }

    fn compatible(&self, a: u32, b: u32) -> bool {
        let (ta, tb) = (self.trees[0], self.trees[1]);
        match (item_entry(ta, a), item_entry(tb, b)) {
            (Some((kind, p)), Some((_, q))) => {
                p == q || !self.items[1].contains(&(kind, p)) && !self.items[0].contains(&(kind, q))
            }
            _ => true,
        }
    }

    fn direct(&self, side: usize, list: u32) -> i64 {
        let tree = self.trees[side];
        let end = self.next(side, list);
        let mut at = list + 1;
        let mut sum = 0;
        while at < end {
            if !tree.is_list(at) {
                sum += self.own(side, at);
            }
            at = self.next(side, at);
        }
        sum
    }

    fn savings(&mut self, a: u32, b: u32, gamma: i64) -> Result<i64, Fallback> {
        if let Some(&value) = self.memo.get(&(a, b, gamma)) {
            return Ok(value);
        }
        let value = if self.equal(a, b) {
            self.total(0, a) + self.total(1, b)
        } else {
            let (na, nb) = (
                &self.trees[0].nodes[a as usize],
                &self.trees[1].nodes[b as usize],
            );
            let mut base =
                self.own(0, a) + self.own(1, b) - gamma * (self.direct(0, a) + self.direct(1, b));
            if !na.open.is_empty() && !nb.open.is_empty() {
                base -= self.split(na.open.start, nb.open.start)
                    + self.split(na.close.start, nb.close.start);
            }
            let xs = self.trees[0].children(a);
            let ys = self.trees[1].children(b);
            let cells = self.cells(&xs, &ys, a, b, (gamma, gamma))?;
            base + chain(&cells).0
        };
        self.memo.insert((a, b, gamma), value);
        Ok(value)
    }

    fn pair_value(&mut self, a: u32, b: u32, charge: Charge) -> Result<i64, Fallback> {
        match (self.is_list(0, a), self.is_list(1, b)) {
            (true, true) if !self.compatible(a, b) => Ok(0),
            (true, true) if self.equal(a, b) || self.pairable(a, b) => self.savings(a, b, GAMMA),
            (false, false) => Ok(self.leaf_value(a, b, charge)),
            _ => Ok(0),
        }
    }

    fn cells(
        &mut self,
        xs: &[u32],
        ys: &[u32],
        a: u32,
        b: u32,
        charge: Charge,
    ) -> Result<Vec<Cell>, Fallback> {
        let mut out = Vec::new();
        if xs.is_empty() || ys.is_empty() {
            return Ok(out);
        }
        self.spend(xs.len() + ys.len())?;
        let (ia, ib) = self.intern(xs, ys);
        let lead_a = |i: usize| self.leads(0, xs[i]);
        let lead_b = |i: usize| self.leads(1, ys[i]);
        let changes = slide(myers::diff(&ia, &ib), &ia, &ib, [&lead_a, &lead_b]);
        let mut matched = [vec![false; xs.len()], vec![false; ys.len()]];
        let mut gaps = Vec::new();
        let (mut i, mut j) = (0, 0);
        for change in changes
            .iter()
            .map(|c| {
                (
                    c.old.start as usize..c.old.end as usize,
                    c.new.start as usize..c.new.end as usize,
                )
            })
            .chain([(xs.len()..xs.len(), ys.len()..ys.len())])
        {
            while i < change.0.start {
                let value = self.pair_value(xs[i], ys[j], charge)?;
                out.push(Cell {
                    old: (i, i),
                    new: (j, j),
                    value,
                    act: Act::Equal(xs[i], ys[j]),
                });
                matched[0][i] = true;
                matched[1][j] = true;
                (i, j) = (i + 1, j + 1);
            }
            if !change.0.is_empty() || !change.1.is_empty() {
                gaps.push(change.clone());
            }
            (i, j) = (change.0.end, change.1.end);
        }
        let mut found: HashMap<(usize, usize, u32, u32), i64> = HashMap::default();
        let mut slots: Vec<(usize, usize)> = Vec::new();
        for (olds, news) in &gaps {
            if olds.len() * news.len() > MAX_PAIR_CELLS {
                continue;
            }
            self.spend(olds.len() * news.len())?;
            for i in olds.clone() {
                let mut ranked: Vec<(i64, usize)> = Vec::new();
                for j in news.clone() {
                    let (x, y) = (xs[i], ys[j]);
                    match (self.is_list(0, x), self.is_list(1, y)) {
                        (false, false) => {
                            let value = self.leaf_value(x, y, charge);
                            if value > 0 {
                                found.insert((i, j, x, y), value);
                            }
                        }
                        (true, true) if self.pairable(x, y) => {
                            ranked.push((self.estimate(x, y), j));
                        }
                        _ => {}
                    }
                }
                ranked.sort_unstable_by_key(|&(e, j)| (std::cmp::Reverse(e), j));
                let floor = ranked.first().map_or(0, |&(e, _)| e / 2);
                for &(_, j) in ranked.iter().take(CANDIDATES).filter(|&&(e, _)| e > floor) {
                    found.entry((i, j, xs[i], ys[j])).or_insert(0);
                }
            }
        }
        let gap_old: Vec<usize> = (0..xs.len()).filter(|&i| !matched[0][i]).collect();
        let gap_new: Vec<usize> = (0..ys.len()).filter(|&j| !matched[1][j]).collect();
        let at_new: HashMap<u32, usize> = gap_new.iter().map(|&j| (ys[j], j)).collect();
        let at_old: HashMap<u32, usize> = gap_old.iter().map(|&i| (xs[i], i)).collect();
        for &i in &gap_old {
            for (j, y) in self.reach(0, xs[i], b, &at_new)? {
                found.entry((i, j, xs[i], y)).or_insert(0);
            }
        }
        for &j in &gap_new {
            for (i, x) in self.reach(1, ys[j], a, &at_old)? {
                if x != xs[i] {
                    found.entry((i, j, x, ys[j])).or_insert(0);
                }
            }
        }
        for (olds, news) in &gaps {
            let trees = self.trees;
            let kind = |side: usize, n: u32| trees[side].nodes[n as usize].kind;
            for i in olds.clone() {
                let x = xs[i];
                let Some(j) = news.clone().find(|&j| kind(1, ys[j]) == kind(0, x)) else {
                    continue;
                };
                let y = ys[j];
                let unique = olds
                    .clone()
                    .filter(|&k| kind(0, xs[k]) == kind(0, x))
                    .count()
                    == 1
                    && news
                        .clone()
                        .filter(|&k| kind(1, ys[k]) == kind(1, y))
                        .count()
                        == 1;
                let alone = found
                    .keys()
                    .all(|k| (k.0 != i && k.1 != j) || (k.2, k.3) == (x, y));
                if unique && alone && self.pairable(x, y) && self.compatible(x, y) {
                    let value = self.savings(x, y, 0)?;
                    if value > 0 {
                        found.insert((i, j, x, y), value);
                        slots.push((i, j));
                    }
                }
            }
        }
        let mut keys: Vec<(usize, usize, u32, u32)> = found.keys().copied().collect();
        keys.sort_unstable();
        let mut detached: Vec<(usize, usize, u32, u32, i64)> = Vec::new();
        for (i, j, x, y) in keys {
            let deep = x != xs[i] || y != ys[j];
            let value = match found[&(i, j, x, y)] {
                0 => {
                    let charge = (
                        if x == xs[i] { charge.0 } else { 0 },
                        if y == ys[j] { charge.1 } else { 0 },
                    );
                    self.pair_value(x, y, charge)?
                }
                value => value,
            } - if deep { self.detached(x, y) } else { 0 };
            if value <= 0 {
                continue;
            }
            out.push(Cell {
                old: (i, i),
                new: (j, j),
                value,
                act: Act::Pair(x, y, slot_gamma(&slots, i, j)),
            });
            if deep {
                detached.push((i, j, x, y, value));
            }
        }
        let reached: HashSet<(bool, usize)> = found
            .keys()
            .flat_map(|&(i, j, _, _)| [(true, i), (false, j)])
            .collect();
        self.wraps(xs, ys, a, b, charge, &detached, &gaps, &reached, &mut out)?;
        Ok(out)
    }

    fn wraps(
        &mut self,
        xs: &[u32],
        ys: &[u32],
        a: u32,
        b: u32,
        charge: Charge,
        deep: &[(usize, usize, u32, u32, i64)],
        gaps: &[(Range<usize>, Range<usize>)],
        reached: &HashSet<(bool, usize)>,
        out: &mut Vec<Cell>,
    ) -> Result<(), Fallback> {
        let mut runs: Vec<(usize, usize, usize, u32, bool)> = Vec::new();
        for &(i, j, x, y, _) in deep {
            let (wrap, host) = if y != ys[j] {
                (true, self.parents[1][y as usize])
            } else {
                (false, self.parents[0][x as usize])
            };
            let (at, other) = if wrap { (i, j) } else { (j, i) };
            match runs
                .iter_mut()
                .find(|r| r.4 == wrap && r.2 == other && r.3 == host && r.1 + 1 == at)
            {
                Some(run) => run.1 = at,
                None => runs.push((at, at, other, host, wrap)),
            }
        }
        let mut spans = Vec::new();
        for (from, to, other, host, wrap) in runs {
            spans.push((from, to, other, host, wrap));
            let Some(gap) = gaps
                .iter()
                .map(|(o, n)| if wrap { o.clone() } else { n.clone() })
                .find(|g| g.contains(&from))
            else {
                continue;
            };
            let bare = |k: usize| !reached.contains(&(wrap, k));
            let (mut start, mut end) = (from, to);
            while start > gap.start && bare(start - 1) {
                start -= 1;
            }
            while end + 1 < gap.end && bare(end + 1) {
                end += 1;
            }
            if (start, end) != (from, to) {
                spans.push((start, end, other, host, wrap));
            }
        }
        for (from, to, other, host, wrap) in spans {
            if from == to {
                continue;
            }
            let key = if wrap {
                (xs[from], xs[to], host, charge.0)
            } else {
                (ys[from], ys[to], host, charge.1 + 2)
            };
            let value = match self.wrapped.get(&key) {
                Some(&value) => value,
                None => {
                    let value = if wrap {
                        let inner = self.trees[1].children(host);
                        let cells = self.cells(&xs[from..=to], &inner, a, host, (charge.0, 0))?;
                        chain(&cells).0 - DETACHED
                    } else {
                        let inner = self.trees[0].children(host);
                        let cells = self.cells(&inner, &ys[from..=to], host, b, (0, charge.1))?;
                        chain(&cells).0 - DETACHED
                    };
                    self.wrapped.insert(key, value);
                    value
                }
            };
            if value <= 0 {
                continue;
            }
            let (old, new, act) = if wrap {
                ((from, to), (other, other), Act::Wrap(from, to, host))
            } else {
                ((other, other), (from, to), Act::Unwrap(from, to, host))
            };
            out.push(Cell {
                old,
                new,
                value,
                act,
            });
        }
        Ok(())
    }

    /// Nodes on the other side, inside `scope`, that `node` may pair
    /// with: through anchors inside it, its name, or an equal subtree
    /// nested deeper. Each comes with the index of the scope child that
    /// holds it.
    fn reach(
        &mut self,
        side: usize,
        node: u32,
        scope: u32,
        children: &HashMap<u32, usize>,
    ) -> Result<Vec<(usize, u32)>, Fallback> {
        let other = 1 - side;
        let mut votes: HashMap<u32, i64> = HashMap::default();
        let pins = self.pins_in(side, node).to_vec();
        self.spend(pins.len())?;
        let fits = |m: &Self, target: u32| {
            let (x, y) = if side == 0 {
                (node, target)
            } else {
                (target, node)
            };
            m.pairable(x, y) || m.equal(x, y)
        };
        for &(p, q) in &pins {
            if !self.inside(other, q, scope) {
                continue;
            }
            let mass = self.total(side, p);
            let mut at = if p == node {
                q
            } else {
                self.parents[other][q as usize]
            };
            while at != NONE && at != scope {
                if fits(self, at) {
                    *votes.entry(at).or_default() += mass;
                }
                at = self.parents[other][at as usize];
            }
        }
        if votes.is_empty() && self.significant(side, node) {
            let end = self.next(other, scope);
            let hits = self.hashes[other]
                .get(&self.hash(side, node))
                .map(|hits| {
                    let from = hits.partition_point(|&h| h <= scope);
                    hits[from..]
                        .iter()
                        .copied()
                        .take_while(|&h| h < end)
                        .filter(|h| !children.contains_key(h))
                        .take(MAX_HASH_HITS)
                        .collect::<Vec<u32>>()
                })
                .unwrap_or_default();
            let lone = self.trees[side].is_list(node) || {
                let parent = self.parents[side][node as usize];
                self.trees[side].children(parent).len() == 1
            };
            for hit in hits.into_iter().filter(|_| lone) {
                if fits(self, hit) || !self.is_list(side, node) {
                    votes.insert(hit, self.total(side, node));
                }
            }
        }
        if let Some(name) = item_at(self.trees[side], node) {
            let name = self.trees[side].text(name);
            for (&target, _) in children.iter() {
                if item_at(self.trees[other], target)
                    .is_some_and(|n| self.trees[other].text(n) == name)
                    && fits(self, target)
                {
                    votes.insert(target, i64::MAX);
                }
            }
        }
        let mut ranked: Vec<(i64, u32)> = votes.into_iter().map(|(t, v)| (v, t)).collect();
        ranked.sort_unstable_by_key(|&(v, t)| (std::cmp::Reverse(v), t));
        let floor = ranked
            .iter()
            .map(|&(v, _)| v)
            .find(|&v| v != i64::MAX)
            .map_or(0, |v| v / 2);
        let mut out = Vec::new();
        for (_, target) in ranked
            .into_iter()
            .filter(|&(v, _)| v > floor)
            .take(CANDIDATES)
        {
            let mut top = target;
            while top != NONE && !children.contains_key(&top) {
                top = self.parents[other][top as usize];
            }
            if let Some(&index) = children.get(&top) {
                out.push((index, target));
            }
        }
        Ok(out)
    }

    fn estimate(&mut self, a: u32, b: u32) -> i64 {
        for (side, node) in [(0, a), (1, b)] {
            if !self.signatures[side].contains_key(&node) {
                let signature = self.signature(side, node);
                self.signatures[side].insert(node, signature);
            }
        }
        let (sa, sb) = (&self.signatures[0][&a], &self.signatures[1][&b]);
        if sa.is_empty() && sb.is_empty() {
            return 1;
        }
        let (mut i, mut j, mut common) = (0, 0, 0);
        while i < sa.len() && j < sb.len() {
            match sa[i].cmp(&sb[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    common += 1;
                    i += 1;
                    j += 1;
                }
            }
        }
        common
    }

    fn signature(&self, side: usize, node: u32) -> Vec<u64> {
        let tree = self.trees[side];
        let end = self.next(side, node);
        let mut out = Vec::new();
        let mut at = node;
        while at < end && out.len() < SIGNATURE_LEAVES {
            let n = &tree.nodes[at as usize];
            if n.class != Class::List && self.significant(side, at) {
                out.push(n.hash);
            }
            at += 1;
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn intern(&self, a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
        let mut ids: HashMap<(u64, u32), u32> =
            HashMap::with_capacity_and_hasher(a.len() + b.len(), FastHash::default());
        let id = |side: usize, node: u32, ids: &mut HashMap<(u64, u32), u32>| {
            let key = (self.hash(side, node), self.next(side, node) - node);
            let next = ids.len() as u32;
            *ids.entry(key).or_insert(next)
        };
        let ia = a.iter().map(|&n| id(0, n, &mut ids)).collect();
        let ib = b.iter().map(|&n| id(1, n, &mut ids)).collect();
        (ia, ib)
    }

    fn leads(&self, side: usize, node: u32) -> bool {
        let tree = self.trees[side];
        let start = tree.nodes[node as usize].span.start as usize;
        tree.src.as_bytes()[..start]
            .iter()
            .rev()
            .take_while(|&&c| c != b'\n')
            .all(u8::is_ascii_whitespace)
    }

    fn apply(&mut self, a: u32, b: u32, gamma: i64) -> Result<(), Fallback> {
        self.link(a, b);
        if self.same(a, b) || !self.is_list(0, a) {
            return Ok(());
        }
        let xs = self.trees[0].children(a);
        let ys = self.trees[1].children(b);
        self.apply_chain(&xs, &ys, a, b, (gamma, gamma))
    }

    fn apply_chain(
        &mut self,
        xs: &[u32],
        ys: &[u32],
        a: u32,
        b: u32,
        charge: Charge,
    ) -> Result<(), Fallback> {
        let cells = self.cells(xs, ys, a, b, charge)?;
        let (_, chosen) = chain(&cells);
        for k in chosen {
            match cells[k].act {
                Act::Equal(x, y) | Act::Pair(x, y, _) => {
                    let gamma = match cells[k].act {
                        Act::Pair(_, _, 0) => 0,
                        _ => GAMMA,
                    };
                    if !self.same(x, y) {
                        self.apply(x, y, gamma)?;
                    }
                }
                Act::Wrap(from, to, host) => {
                    let inner = self.trees[1].children(host);
                    self.apply_chain(&xs[from..=to], &inner, a, host, (charge.0, 0))?;
                }
                Act::Unwrap(from, to, host) => {
                    let inner = self.trees[0].children(host);
                    self.apply_chain(&inner, &ys[from..=to], host, b, (0, charge.1))?;
                }
            }
        }
        Ok(())
    }

    /// Pairs subtrees left unlinked whose correspondence pays for a move
    /// note: anchors that landed outside their container, equal subtrees
    /// and same-named items, best first.
    fn relocate(&mut self) -> Result<(), Fallback> {
        let mut candidates: Vec<(u32, u32)> = Vec::new();
        let pins = self.pins.clone();
        self.spend(pins.len())?;
        for &(p, q) in &pins {
            if self.partner[0][p as usize] != NONE || self.partner[1][q as usize] != NONE {
                continue;
            }
            let olds = self.open_chain(0, p);
            let news = self.open_chain(1, q);
            for &x in &olds {
                if let Some(&y) = news
                    .iter()
                    .find(|&&y| self.pairable(x, y) || self.equal(x, y))
                {
                    candidates.push((x, y));
                }
            }
        }
        let (old, new) = (self.trees[0], self.trees[1]);
        for item in &old.items {
            if self.partner[0][item.node as usize] != NONE {
                continue;
            }
            let name = old.text(item.name.clone());
            let same: Vec<u32> = new
                .items
                .iter()
                .filter(|i| {
                    self.partner[1][i.node as usize] == NONE && new.text(i.name.clone()) == name
                })
                .map(|i| i.node)
                .collect();
            if let [y] = same[..] {
                candidates.push((item.node, y));
            }
        }
        candidates.extend(self.duplicates()?);
        candidates.sort_unstable();
        candidates.dedup();
        let mut valued = Vec::new();
        for (x, y) in candidates {
            if !self.compatible(x, y)
                || !whole_lines(self.trees[0], x)
                || !whole_lines(self.trees[1], y)
                || !self.free(0, x)
                || !self.free(1, y)
            {
                continue;
            }
            let value = self.move_value(x, y)?;
            if value > 0 {
                valued.push((value, x, y));
            }
        }
        valued.sort_unstable_by_key(|&(v, x, y)| (std::cmp::Reverse(v), x, y));
        for (_, x, y) in valued {
            if !self.free(0, x) || !self.free(1, y) || !self.disjoint(x, y) {
                continue;
            }
            self.apply(x, y, GAMMA)?;
            self.moves.push((x, y));
        }
        Ok(())
    }

    fn move_value(&mut self, x: u32, y: u32) -> Result<i64, Fallback> {
        let kept = self.savings(x, y, GAMMA)?;
        let lost = (self.total(0, x) + self.total(1, y) - kept).max(0);
        let note = if item_at(self.trees[0], x).is_some() {
            MOVE_ITEM
        } else {
            MOVE
        };
        Ok(kept - note - lost * MOVE_LOSS.0 / MOVE_LOSS.1)
    }

    fn disjoint(&self, x: u32, y: u32) -> bool {
        let inside = |n: u32, root: u32, side: usize| root <= n && n < self.next(side, root);
        self.moves.iter().all(|&(mo, mn)| {
            !(inside(mo, x, 0) || inside(x, mo, 0) || inside(mn, y, 1) || inside(y, mn, 1))
        })
    }

    fn open_chain(&self, side: usize, node: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut at = node;
        while at != NONE && at != 0 && self.partner[side][at as usize] == NONE {
            if self.is_list(side, at) {
                out.push(at);
            }
            at = self.parents[side][at as usize];
        }
        out
    }

    fn duplicates(&mut self) -> Result<Vec<(u32, u32)>, Fallback> {
        let mut found: [HashMap<u64, Vec<u32>>; 2] = [HashMap::default(), HashMap::default()];
        for side in [0, 1] {
            let tree = self.trees[side];
            self.spend(tree.nodes.len())?;
            let mut at = 1;
            while at < tree.nodes.len() as u32 {
                let end = tree.nodes[at as usize].next;
                if tree.is_list(at) && whole_lines(tree, at) && self.free(side, at) {
                    found[side].entry(self.hash(side, at)).or_default().push(at);
                    at = end;
                    continue;
                }
                at += 1;
            }
        }
        let [olds, news] = found;
        let mut out = Vec::new();
        for (hash, olds) in &olds {
            if let Some(targets) = news.get(hash)
                && olds.len() == targets.len()
            {
                out.extend(olds.iter().copied().zip(targets.iter().copied()));
            }
        }
        Ok(out)
    }

    fn scope(&mut self) -> Result<(), Fallback> {
        for side in [0, 1] {
            let (own, other) = (self.trees[side], self.trees[1 - side]);
            self.spend(own.nodes.len())?;
            let mut stack: Vec<(u32, u32)> = vec![(own.nodes.len() as u32, 0)];
            for a in 1..own.nodes.len() as u32 {
                while stack.last().is_some_and(|&(end, _)| end <= a) {
                    stack.pop();
                }
                let b = self.partner[side][a as usize];
                if b == NONE {
                    continue;
                }
                let scope = stack.last().map_or(0, |&(_, s)| s);
                let inside = scope < b && b < other.nodes[scope as usize].next;
                let root = self
                    .moves
                    .iter()
                    .any(|&(o, n)| if side == 0 { o == a } else { n == a });
                if inside || root {
                    if own.is_list(a) {
                        stack.push((own.nodes[a as usize].next, b));
                    }
                    continue;
                }
                let o = if side == 0 { a } else { b };
                self.unlink(o);
            }
        }
        Ok(())
    }

    fn monotone(&mut self) -> Result<(), Fallback> {
        let (old, new) = (self.trees[0], self.trees[1]);
        self.spend(old.nodes.len())?;
        let owner = |tree: &Tree<'_>, roots: &[u32]| {
            let mut out = vec![NONE; tree.nodes.len()];
            for (index, &root) in roots.iter().enumerate() {
                let end = tree.nodes[root as usize].next;
                for slot in &mut out[root as usize..end as usize] {
                    *slot = index as u32;
                }
            }
            out
        };
        let old_roots: Vec<u32> = self.moves.iter().map(|&(o, _)| o).collect();
        let new_roots: Vec<u32> = self.moves.iter().map(|&(_, n)| n).collect();
        let (old_owner, new_owner) = (owner(old, &old_roots), owner(new, &new_roots));
        let mut groups: HashMap<u32, Vec<(u32, u32, u32)>> = HashMap::default();
        for a in 0..old.nodes.len() as u32 {
            let b = self.partner[0][a as usize];
            if b == NONE {
                continue;
            }
            let group = old_owner[a as usize];
            if group != new_owner[b as usize] {
                self.unlink(a);
                continue;
            }
            let (x, y) = (&old.nodes[a as usize], &new.nodes[b as usize]);
            let entries = groups.entry(group).or_default();
            if x.class == Class::List {
                if !x.open.is_empty() && !y.open.is_empty() {
                    entries.push((x.open.start, y.open.start, a));
                    entries.push((x.close.start, y.close.start, a));
                }
            } else {
                entries.push((x.span.start, y.span.start, a));
            }
        }
        let mut keys: Vec<u32> = groups.keys().copied().collect();
        keys.sort_unstable();
        for key in keys {
            let mut entries = groups.remove(&key).unwrap_or_default();
            entries.sort_unstable();
            let keyed: Vec<(u32, u32, u64)> = entries
                .iter()
                .enumerate()
                .map(|(k, &(_, y, _))| (k as u32, y, 1))
                .collect();
            let kept = heaviest_increasing(&keyed);
            for (k, &(_, _, node)) in entries.iter().enumerate() {
                if !kept[k] {
                    self.unlink(node);
                }
            }
        }
        Ok(())
    }
}

/// The heaviest set of cells increasing on both sides, with its value.
fn chain(cells: &[Cell]) -> (i64, Vec<usize>) {
    if cells.is_empty() {
        return (0, Vec::new());
    }
    let mut order: Vec<usize> = (0..cells.len()).collect();
    order.sort_unstable_by_key(|&k| (cells[k].old.0, cells[k].new.0, k));
    let mut ends: Vec<usize> = order.clone();
    ends.sort_unstable_by_key(|&k| (cells[k].old.1, k));
    let mut keys: Vec<usize> = cells.iter().map(|c| c.new.1).collect();
    keys.sort_unstable();
    keys.dedup();
    let size = keys.len();
    let mut tree = vec![(0i64, usize::MAX); size + 1];
    let mut best = vec![(0i64, usize::MAX); cells.len()];
    let mut released = 0;
    let mut top = (0i64, usize::MAX);
    for &k in &order {
        let cell = &cells[k];
        while released < ends.len() && cells[ends[released]].old.1 < cell.old.0 {
            let e = ends[released];
            let rank = keys.partition_point(|&key| key < cells[e].new.1) + 1;
            let mut i = rank;
            while i <= size {
                if best[e].0 > tree[i].0 {
                    tree[i] = (best[e].0, e);
                }
                i += i & i.wrapping_neg();
            }
            released += 1;
        }
        let rank = keys.partition_point(|&key| key < cell.new.0);
        let mut found = (0i64, usize::MAX);
        let mut i = rank;
        while i > 0 {
            if tree[i].0 > found.0 {
                found = tree[i];
            }
            i &= i - 1;
        }
        best[k] = (found.0 + cell.value, found.1);
        if best[k].0 > top.0 {
            top = (best[k].0, k);
        }
    }
    let mut chosen = Vec::new();
    let mut at = top.1;
    while at != usize::MAX {
        chosen.push(at);
        at = best[at].1;
    }
    chosen.reverse();
    (top.0, chosen)
}

fn slot_gamma(slots: &[(usize, usize)], i: usize, j: usize) -> i64 {
    if slots.contains(&(i, j)) { 0 } else { GAMMA }
}

fn common_words(a: &str, b: &str) -> i64 {
    fn words(s: &str) -> Vec<&str> {
        let mut out: Vec<&str> = s
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|w| !w.is_empty())
            .collect();
        out.sort_unstable();
        out
    }
    let (wa, wb) = (words(a), words(b));
    let (mut i, mut j, mut common) = (0, 0, 0);
    while i < wa.len() && j < wb.len() {
        match wa[i].cmp(wb[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    common
}

fn heaviest_increasing(items: &[(u32, u32, u64)]) -> Vec<bool> {
    let n = items.len();
    let mut kept = vec![false; n];
    if n == 0 {
        return kept;
    }
    let mut keys: Vec<u32> = items.iter().map(|&(_, key, _)| key).collect();
    keys.sort_unstable();
    keys.dedup();
    let size = keys.len();
    let mut tree = vec![(0u64, usize::MAX); size + 1];
    let mut previous = vec![usize::MAX; n];
    let mut best = (0u64, usize::MAX);
    for (index, &(_, key, weight)) in items.iter().enumerate() {
        let rank = keys.partition_point(|&k| k < key);
        let mut found = (0u64, usize::MAX);
        let mut i = rank;
        while i > 0 {
            if tree[i].0 > found.0 {
                found = tree[i];
            }
            i &= i - 1;
        }
        previous[index] = found.1;
        let total = (found.0 + weight, index);
        if total.0 > best.0 {
            best = total;
        }
        let mut i = rank + 1;
        while i <= size {
            if total.0 > tree[i].0 {
                tree[i] = total;
            }
            i += i & i.wrapping_neg();
        }
    }
    let mut at = best.1;
    while at != usize::MAX {
        kept[at] = true;
        at = previous[at];
    }
    kept
}

fn slide(
    mut changes: Vec<myers::Change>,
    a: &[u32],
    b: &[u32],
    leads: [&dyn Fn(usize) -> bool; 2],
) -> Vec<myers::Change> {
    for k in 0..changes.len() {
        let limit = changes
            .get(k + 1)
            .map_or((a.len() as u32, b.len() as u32), |next| {
                (next.old.start, next.new.start)
            });
        let floor = k
            .checked_sub(1)
            .map_or((0, 0), |p| (changes[p].old.end, changes[p].new.end));
        let c = &mut changes[k];
        let (side, items, run, other, ceiling, bottom) = if c.old.is_empty() {
            (1, b, &mut c.new, &mut c.old, limit.1, floor.1)
        } else if c.new.is_empty() {
            (0, a, &mut c.old, &mut c.new, limit.0, floor.0)
        } else {
            continue;
        };
        let other_floor = if side == 1 { floor.0 } else { floor.1 };
        while run.start > bottom
            && other.start > other_floor
            && items[run.start as usize - 1] == items[run.end as usize - 1]
        {
            *run = run.start - 1..run.end - 1;
            *other = other.start - 1..other.end - 1;
        }
        let score = |run: &Range<u32>| {
            let end = run.end as usize;
            2 * u8::from(leads[side](run.start as usize))
                + u8::from(end == items.len() || leads[side](end))
        };
        let (mut best, mut best_score) = (run.clone(), score(run));
        let mut probe = run.clone();
        while probe.end < ceiling && items[probe.start as usize] == items[probe.end as usize] {
            probe = probe.start + 1..probe.end + 1;
            let s = score(&probe);
            if s >= best_score {
                (best, best_score) = (probe.clone(), s);
            }
        }
        let shift = best.start - run.start;
        *run = best;
        *other = other.start + shift..other.end + shift;
    }
    let mut out: Vec<myers::Change> = Vec::with_capacity(changes.len());
    for c in changes {
        if let Some(p) = out.last_mut()
            && let Some(merged) = join(p, &c, a, b)
        {
            *p = merged;
            continue;
        }
        out.push(c);
    }
    out
}

fn join(p: &myers::Change, c: &myers::Change, a: &[u32], b: &[u32]) -> Option<myers::Change> {
    let run = c.old.start - p.old.end;
    let back = |items: &[u32], r: &Range<u32>| {
        (1..=run).all(|t| items[(r.start - t) as usize] == items[(r.end - t) as usize])
    };
    if c.new.is_empty() && !p.new.is_empty() && back(a, &c.old) {
        return Some(myers::Change {
            old: p.old.start..c.old.end - run,
            new: p.new.clone(),
        });
    }
    if c.old.is_empty() && !p.old.is_empty() && back(b, &c.new) {
        return Some(myers::Change {
            old: p.old.clone(),
            new: p.new.start..c.new.end - run,
        });
    }
    None
}

pub(crate) fn parents(tree: &Tree<'_>) -> Vec<u32> {
    let mut out = vec![NONE; tree.nodes.len()];
    let mut stack: Vec<u32> = Vec::new();
    for (at, node) in tree.nodes.iter().enumerate() {
        let at = at as u32;
        while stack
            .last()
            .is_some_and(|&p| tree.nodes[p as usize].next <= at)
        {
            stack.pop();
        }
        if let Some(&p) = stack.last() {
            out[at as usize] = p;
        }
        if node.class == Class::List {
            stack.push(at);
        }
    }
    out
}

fn weights(tree: &Tree<'_>) -> Weights {
    let mut own = Vec::with_capacity(tree.nodes.len());
    let mut sum = Vec::with_capacity(tree.nodes.len() + 1);
    sum.push(0);
    for node in &tree.nodes {
        let text = tree.text(node.span.clone());
        let w = match node.class {
            Class::List => {
                DELIMITER * (i64::from(!node.open.is_empty()) + i64::from(!node.close.is_empty()))
            }
            Class::Text | Class::Comment => {
                let words = text
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .filter(|w| !w.is_empty())
                    .count() as i64;
                WORD * words.max(1)
            }
            Class::Plain if text.bytes().any(|b| b.is_ascii_alphanumeric() || b >= 0x80) => WORD,
            Class::Plain => PUNCT,
        };
        own.push(w);
        sum.push(sum.last().copied().unwrap_or(0) + w);
    }
    Weights { own, sum }
}

fn lines(tree: &Tree<'_>) -> Lines {
    let mut starts = vec![0u32];
    let mut hashes = Vec::new();
    for line in tree.src.split_inclusive('\n') {
        hashes.push(crate::syntax::hash_bytes(line.trim().as_bytes()));
        starts.push(starts.last().copied().unwrap_or(0) + line.len() as u32);
    }
    hashes.push(0);
    Lines { starts, hashes }
}

fn names<'a>(tree: &Tree<'a>) -> HashSet<&'a str> {
    tree.nodes
        .iter()
        .filter(|n| n.name)
        .map(|n| tree.text(n.span.clone()))
        .collect()
}

pub(crate) fn whole_lines(tree: &Tree<'_>, node: u32) -> bool {
    let span = &tree.nodes[node as usize].span;
    let src = tree.src.as_bytes();
    let before = src[..span.start as usize]
        .iter()
        .rev()
        .take_while(|&&b| b != b'\n')
        .all(u8::is_ascii_whitespace);
    let after = src[span.end as usize..]
        .iter()
        .take_while(|&&b| b != b'\n')
        .all(|&b| b.is_ascii_whitespace() || b == b',' || b == b';');
    before && after
}

fn items<'a>(tree: &Tree<'a>) -> HashSet<(&'static str, &'a str)> {
    tree.items
        .iter()
        .map(|item| (item.kind, tree.text(item.name.clone())))
        .collect()
}

fn item_entry<'a>(tree: &Tree<'a>, node: u32) -> Option<(&'static str, &'a str)> {
    let at = tree
        .items
        .binary_search_by_key(&node, |item| item.node)
        .ok()?;
    let item = &tree.items[at];
    Some((item.kind, tree.text(item.name.clone())))
}

fn item_at(tree: &Tree<'_>, node: u32) -> Option<Range<u32>> {
    let at = tree
        .items
        .binary_search_by_key(&node, |item| item.node)
        .ok()?;
    Some(tree.items[at].name.clone())
}
