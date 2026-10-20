use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;

use diffy_diff::myers::{self, FastHash};
use diffy_diff::{InlineDetail, InlineOptions, Rename, paired_inline_diff};

use crate::Fallback;
use crate::align::Notes;
use crate::cost::{
    self, ENTRIES, FOLLOWS, Features, IDENTITY, INSIDE, ISOLATED, MOVED, MOVES, RENAMED, SPLIT,
    UNMATCHED, WORD, Weights,
};
use crate::syntax::{Class, NONE, Tree};

type HashMap<K, V> = std::collections::HashMap<K, V, FastHash>;
type HashSet<K> = std::collections::HashSet<K, FastHash>;

const CANDIDATES: usize = 4;
const SIGNATURE_LEAVES: usize = 64;
const MAX_PAIR_CELLS: usize = 1 << 16;
const MAX_HASH_HITS: usize = 64;
const CLOCK_STRIDE: u64 = 1 << 16;
const MOVE_GROUP_GAP: u32 = diffy_diff::MOVE_GROUP_GAP;
const TIE: u32 = 20;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Options {
    pub(crate) weights: Weights,
    pub(crate) candidates: usize,
    pub(crate) floor: bool,
    pub(crate) exhaustive: bool,
    pub(crate) margins: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            weights: Weights::DEFAULT,
            candidates: CANDIDATES,
            floor: true,
            exhaustive: false,
            margins: false,
        }
    }
}

/// Which old node each new node corresponds to and back. Every link sits
/// under linked ancestors that correspond to each other, or under the
/// root of a move; links never cross each other outside a move.
#[cfg_attr(not(feature = "eval"), allow(dead_code))]
pub(crate) struct Matching {
    pub(crate) partner: [Vec<u32>; 2],
    /// Roots of moved subtrees, old and new.
    pub(crate) moves: Vec<(u32, u32)>,
    pub(crate) renames: Vec<Rename>,
    pub(crate) predicted: Features,
    pub(crate) repairs: u32,
    pub(crate) margins: Vec<Margin>,
    pub(crate) work: u64,
}

#[cfg_attr(not(feature = "eval"), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct Margin {
    pub(crate) kind: &'static str,
    pub(crate) old: u32,
    pub(crate) new: u32,
    pub(crate) best: i64,
    pub(crate) runner: i64,
}

pub(crate) fn run(
    old: &Tree<'_>,
    new: &Tree<'_>,
    max_work: u64,
    deadline: Instant,
    options: &Options,
    vet: Option<&crate::Vet<'_>>,
) -> Result<Matching, Fallback> {
    let mut m = Matcher {
        trees: [old, new],
        options,
        vet,
        vetoed: None,
        partner: [vec![NONE; old.nodes.len()], vec![NONE; new.nodes.len()]],
        parents: [parents(old), parents(new)],
        mass: [mass(old), mass(new)],
        plain: [plain(old), plain(new)],
        lines: [cost::line_keys(old.src), cost::line_keys(new.src)],
        allowed: None,
        hashes: [Default::default(), Default::default()],
        pins: Vec::new(),
        pins_new: Vec::new(),
        anchors: Vec::new(),
        forced: Vec::new(),
        fixed: Vec::new(),
        owner: [vec![NONE; old.nodes.len()], vec![NONE; new.nodes.len()]],
        memo: HashMap::default(),
        wrapped: HashMap::default(),
        acts: HashMap::default(),
        wrap_acts: HashMap::default(),
        signatures: Default::default(),
        edits: HashMap::default(),
        moves: Vec::new(),
        predicted: Features::default(),
        repairs: 0,
        margins: Vec::new(),
        work: 0,
        max_work,
        clock_at: CLOCK_STRIDE,
        deadline,
    };
    if old.nodes.is_empty() || new.nodes.is_empty() {
        return Ok(m.finish(Vec::new()));
    }
    m.index()?;
    m.identities()?;
    m.solve()?;
    let (mut renames, rejected) = m.finalize();
    if rejected {
        m.allowed = Some(renames.iter().map(|r| (r.0, r.1)).collect());
        m.memo.retain(|_, e| !e.hyp);
        m.wrapped.retain(|_, e| !e.hyp);
        let (memo, wrapped) = (&m.memo, &m.wrapped);
        m.acts.retain(|k, _| memo.contains_key(k));
        m.wrap_acts.retain(|k, _| wrapped.contains_key(k));
        m.reset();
        m.solve()?;
        renames = m.finalize().0;
    }
    m.scope()?;
    m.monotone()?;
    let renames = renames
        .into_iter()
        .map(|(old, new, uses)| Rename {
            old: old.into(),
            new: new.into(),
            uses,
        })
        .collect();
    Ok(m.finish(renames))
}

struct Matcher<'t, 'a> {
    trees: [&'t Tree<'a>; 2],
    options: &'t Options,
    vet: Option<&'t crate::Vet<'t>>,
    vetoed: Option<HashSet<(&'a str, &'a str)>>,
    partner: [Vec<u32>; 2],
    parents: [Vec<u32>; 2],
    mass: [Mass; 2],
    plain: [HashSet<&'a str>; 2],
    lines: [(Vec<u32>, Vec<u64>); 2],
    allowed: Option<HashSet<(&'a str, &'a str)>>,
    hashes: [HashMap<u64, Vec<u32>>; 2],
    pins: Vec<(u32, u32)>,
    pins_new: Vec<(u32, u32)>,
    anchors: Vec<(u32, u32)>,
    forced: Vec<(u32, u32)>,
    fixed: Vec<(u32, u32)>,
    owner: [Vec<u32>; 2],
    memo: HashMap<(u32, u32), Entry>,
    wrapped: HashMap<(u32, u32, u32, bool), Entry>,
    acts: HashMap<(u32, u32), Rc<[Act]>>,
    wrap_acts: HashMap<(u32, u32, u32, bool), Rc<[Act]>>,
    signatures: [HashMap<u32, Vec<u64>>; 2],
    edits: HashMap<(u32, u32), Option<(i64, i64)>>,
    moves: Vec<(u32, u32)>,
    predicted: Features,
    repairs: u32,
    margins: Vec<Margin>,
    work: u64,
    max_work: u64,
    clock_at: u64,
    deadline: Instant,
}

struct Mass {
    own: Vec<i64>,
    sum: Vec<i64>,
}

#[derive(Clone, Copy, Default)]
struct Entry {
    d: Features,
    hyp: bool,
}

type Ctx = (bool, bool);

#[derive(Clone, Copy)]
enum Act {
    Leaf(u32, u32, Ctx),
    List(u32, u32),
    Wrap(u32, u32, u32, (u32, u32)),
    Unwrap(u32, u32, u32, (u32, u32)),
}

struct Anchor {
    old: usize,
    new: usize,
    x: u32,
    y: u32,
    deep: bool,
}

#[derive(Clone, Copy)]
struct Cell {
    old: (usize, usize),
    new: (usize, usize),
    value: i64,
    exact: bool,
    fixed: bool,
    act: Act,
}

impl<'a> Matcher<'_, 'a> {
    fn finish(self, renames: Vec<Rename>) -> Matching {
        let mut moves = self.moves;
        moves.sort_unstable();
        Matching {
            partner: self.partner,
            moves,
            renames,
            predicted: self.predicted,
            repairs: self.repairs,
            margins: self.margins,
            work: self.work,
        }
    }

    fn reset(&mut self) {
        for side in &mut self.partner {
            side.fill(NONE);
        }
        self.moves.clear();
        self.margins.clear();
        self.predicted = Features::default();
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

    fn degraded(&self) -> bool {
        !self.options.exhaustive && self.work * 2 > self.max_work
    }

    fn candidates(&self) -> usize {
        match (self.options.exhaustive, self.degraded()) {
            (true, _) => usize::MAX,
            (false, true) => 1,
            (false, false) => self.options.candidates,
        }
    }

    fn value(&self, d: &Features) -> i64 {
        (self.options.weights.cost(d) << TIE) + d[SPLIT]
    }

    fn scale(x: i64) -> i64 {
        x << TIE
    }

    fn split(&self, x: u32, y: u32) -> bool {
        cost::line_key(&self.lines[0], x) != cost::line_key(&self.lines[1], y)
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
        let sum = &self.mass[side].sum;
        sum[self.next(side, node) as usize] - sum[node as usize]
    }

    fn own(&self, side: usize, node: u32) -> i64 {
        self.mass[side].own[node as usize]
    }

    fn text(&self, side: usize, node: u32) -> &'a str {
        let tree = self.trees[side];
        tree.text(tree.nodes[node as usize].span.clone())
    }

    fn linked(&self, side: usize, node: u32) -> bool {
        node != NONE && self.partner[side][node as usize] != NONE
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
            self.repairs += 1;
        }
    }

    fn free(&mut self, side: usize, node: u32) -> Result<bool, Fallback> {
        let end = self.next(side, node);
        self.spend((end - node) as usize)?;
        Ok((node..end).all(|k| self.partner[side][k as usize] == NONE))
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

    fn feasible(&self, a: u32, b: u32) -> bool {
        let (ta, tb) = (self.trees[0], self.trees[1]);
        let (na, nb) = (&ta.nodes[a as usize], &tb.nodes[b as usize]);
        match (na.class, nb.class) {
            (Class::List, Class::List) => {
                na.kind == nb.kind
                    && ta.text(na.open.clone()) == tb.text(nb.open.clone())
                    && ta.text(na.close.clone()) == tb.text(nb.close.clone())
            }
            (x, y) => x == y && (x == Class::Plain || na.kind == nb.kind),
        }
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

    fn rename_ok(&self, p: &'a str, q: &'a str) -> bool {
        p != q
            && !self.plain[1].contains(p)
            && !self.plain[0].contains(q)
            && self.allowed.as_ref().is_none_or(|r| r.contains(&(p, q)))
    }

    fn identity(&self, a: u32, b: u32) -> Option<bool> {
        let (ta, tb) = (self.trees[0], self.trees[1]);
        let (p, q) = (ta.text(item_at(ta, a)?), tb.text(item_at(tb, b)?));
        (p != q).then(|| self.rename_ok(p, q))
    }

    fn leaf(&mut self, a: u32, b: u32, ctx: Ctx, exact: bool) -> Result<Option<Entry>, Fallback> {
        let (ta, tb) = (self.trees[0], self.trees[1]);
        let (na, nb) = (&ta.nodes[a as usize], &tb.nodes[b as usize]);
        if na.class == Class::List || !self.feasible(a, b) {
            return Ok(None);
        }
        let (wa, wb) = (self.own(0, a), self.own(1, b));
        let (p, q) = (self.text(0, a), self.text(1, b));
        let mut e = Entry::default();
        let (ua, ub) = match na.class {
            Class::Plain if p == q => (0, 0),
            Class::Plain if na.name && nb.name && self.rename_ok(p, q) => {
                e.hyp = true;
                e.d[RENAMED] = -(wa + wb);
                (0, 0)
            }
            Class::Text | Class::Comment if cost::same_text(na.class, p, q) => (0, 0),
            Class::Text | Class::Comment => {
                let found = if exact {
                    self.edit(a, b)?
                } else {
                    self.guess(a, b)?
                };
                match found {
                    Some(u) => u,
                    None => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        e.d[UNMATCHED] = wa + wb - ua - ub;
        e.d[INSIDE] = if ctx.0 { wa - ua } else { 0 } + if ctx.1 { wb - ub } else { 0 };
        if !ctx.0 && !ctx.1 {
            e.d[ISOLATED] = -1;
        }
        if self.split(na.span.start, nb.span.start) {
            e.d[SPLIT] = -1;
        }
        Ok((self.value(&e.d) > 0).then_some(e))
    }

    fn inexact(&self, a: u32, b: u32) -> bool {
        let class = self.trees[0].nodes[a as usize].class;
        matches!(class, Class::Text | Class::Comment)
            && !cost::same_text(class, self.text(0, a), self.text(1, b))
    }

    fn guess(&mut self, a: u32, b: u32) -> Result<Option<(i64, i64)>, Fallback> {
        let (p, q) = (self.text(0, a), self.text(1, b));
        let mut wa: Vec<&str> = cost::words(p).collect();
        let mut wb: Vec<&str> = cost::words(q).collect();
        self.spend(wa.len() + wb.len())?;
        wa.sort_unstable();
        wb.sort_unstable();
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
        if common == 0 {
            return Ok(None);
        }
        let left = |words: usize| WORD * (words - common) as i64;
        Ok(Some((left(wa.len()), left(wb.len()))))
    }

    fn edit(&mut self, a: u32, b: u32) -> Result<Option<(i64, i64)>, Fallback> {
        if let Some(&found) = self.edits.get(&(a, b)) {
            return Ok(found);
        }
        let (p, q) = (self.text(0, a), self.text(1, b));
        self.spend((p.len() + q.len()) / 8 + 1)?;
        let diff = paired_inline_diff(p, q, &InlineOptions::default());
        let found = (diff.detail == InlineDetail::Exact
            && (!diff.old.is_empty() || !diff.new.is_empty()))
        .then(|| (cost::edited(p, &diff.old), cost::edited(q, &diff.new)));
        self.edits.insert((a, b), found);
        Ok(found)
    }

    fn direct(&mut self, side: usize, list: u32) -> Result<i64, Fallback> {
        let tree = self.trees[side];
        let end = self.next(side, list);
        let mut at = list + 1;
        let mut sum = 0;
        let mut steps = 0;
        while at < end {
            if !tree.is_list(at) {
                sum += self.own(side, at);
            }
            at = self.next(side, at);
            steps += 1;
        }
        self.spend(steps)?;
        Ok(sum)
    }

    fn savings(&mut self, a: u32, b: u32) -> Result<Entry, Fallback> {
        if let Some(&e) = self.memo.get(&(a, b)) {
            return Ok(e);
        }
        self.spend(1)?;
        let mut e = Entry::default();
        if self.equal(a, b) {
            e.d[UNMATCHED] = self.total(0, a) + self.total(1, b);
            e.d[SPLIT] = -self.splits(a, b)?;
        } else {
            e.d[UNMATCHED] = self.own(0, a) + self.own(1, b);
            e.d[SPLIT] = -self.delimiter_splits(a, b);
            e.d[INSIDE] = -(self.direct(0, a)? + self.direct(1, b)?);
            match self.identity(a, b) {
                Some(true) => e.hyp = true,
                Some(false) => e.d[IDENTITY] = -1,
                None => {}
            }
            let xs = self.trees[0].children(a);
            let ys = self.trees[1].children(b);
            let mut cells = self.cells(&xs, &ys, a, b, (true, true))?;
            let chosen = self.select(&mut cells)?;
            for &k in &chosen {
                let c = self.features(&cells[k])?;
                e.d += c.d;
                e.hyp |= c.hyp;
            }
            let acts = chosen.iter().map(|&k| cells[k].act).collect();
            self.acts.insert((a, b), acts);
        }
        self.memo.insert((a, b), e);
        Ok(e)
    }

    fn delimiter_splits(&self, a: u32, b: u32) -> i64 {
        let (na, nb) = (
            &self.trees[0].nodes[a as usize],
            &self.trees[1].nodes[b as usize],
        );
        let mut count = 0;
        for (x, y) in [(&na.open, &nb.open), (&na.close, &nb.close)] {
            if !x.is_empty() && !y.is_empty() && self.split(x.start, y.start) {
                count += 1;
            }
        }
        count
    }

    fn splits(&mut self, a: u32, b: u32) -> Result<i64, Fallback> {
        let len = self.next(0, a) - a;
        self.spend(len as usize)?;
        let mut count = 0;
        for k in 0..len {
            let (x, y) = (a + k, b + k);
            if self.is_list(0, x) {
                count += self.delimiter_splits(x, y);
            } else {
                let start = |side: usize, n: u32| self.trees[side].nodes[n as usize].span.start;
                count += i64::from(self.split(start(0, x), start(1, y)));
            }
        }
        Ok(count)
    }

    fn features(&mut self, cell: &Cell) -> Result<Entry, Fallback> {
        Ok(match cell.act {
            Act::Leaf(x, y, ctx) => self.leaf(x, y, ctx, cell.exact)?.unwrap_or_default(),
            Act::List(x, y) => self.savings(x, y)?,
            Act::Wrap(..) | Act::Unwrap(..) => self.wrapped[&self.wrap_key(cell)],
        })
    }

    fn wrap_key(&self, cell: &Cell) -> (u32, u32, u32, bool) {
        match cell.act {
            Act::Wrap(_, _, host, ends) => (ends.0, ends.1, host, true),
            Act::Unwrap(_, _, host, ends) => (ends.0, ends.1, host, false),
            _ => (NONE, NONE, NONE, false),
        }
    }

    fn select(&mut self, cells: &mut [Cell]) -> Result<Vec<usize>, Fallback> {
        loop {
            self.spend(cells.len())?;
            let mut chosen = chain(cells).1;
            let mut taken = vec![false; cells.len()];
            for &k in &chosen {
                taken[k] = true;
            }
            chosen.extend((0..cells.len()).filter(|&k| cells[k].fixed && !taken[k]));
            let mut refined = false;
            for &k in &chosen {
                if cells[k].exact {
                    continue;
                }
                refined = true;
                cells[k].exact = true;
                if let Act::Leaf(x, y, ctx) = cells[k].act {
                    cells[k].value = match self.leaf(x, y, ctx, true)? {
                        Some(e) => self.value(&e.d),
                        None => -1,
                    };
                }
            }
            if !refined {
                return Ok(chosen);
            }
        }
    }

    fn cells(
        &mut self,
        xs: &[u32],
        ys: &[u32],
        a: u32,
        b: u32,
        ctx: Ctx,
    ) -> Result<Vec<Cell>, Fallback> {
        let mut out = Vec::new();
        if xs.is_empty() || ys.is_empty() {
            return Ok(out);
        }
        self.spend(xs.len() + ys.len())?;
        let fixed = self.anchored(xs, ys)?;
        let mut held = [vec![false; xs.len()], vec![false; ys.len()]];
        for f in fixed.iter().filter(|f| !f.deep) {
            held[0][f.old] = true;
            held[1][f.new] = true;
        }
        let (ia, ib) = self.intern(xs, ys);
        let lead_a = |i: usize| self.leads(0, xs[i]);
        let lead_b = |i: usize| self.leads(1, ys[i]);
        let changes = slide(myers::diff(&ia, &ib), &ia, &ib, [&lead_a, &lead_b]);
        let mut matched = [vec![false; xs.len()], vec![false; ys.len()]];
        let mut keys: Vec<(usize, usize, u32, u32)> = Vec::new();
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
                keys.push((i, j, xs[i], ys[j]));
                matched[0][i] = true;
                matched[1][j] = true;
                (i, j) = (i + 1, j + 1);
            }
            if !change.0.is_empty() || !change.1.is_empty() {
                gaps.push(change.clone());
            }
            (i, j) = (change.0.end, change.1.end);
        }
        let exhaustive = self.options.exhaustive;
        let dense: Vec<(Range<usize>, Range<usize>)> = if exhaustive {
            vec![(0..xs.len(), 0..ys.len())]
        } else if self.degraded() {
            Vec::new()
        } else {
            gaps.iter()
                .filter(|(o, n)| o.len() * n.len() <= MAX_PAIR_CELLS)
                .cloned()
                .collect()
        };
        for (olds, news) in &dense {
            self.spend(olds.len() * news.len())?;
            for i in olds.clone().filter(|&i| !held[0][i]) {
                let mut ranked: Vec<(i64, usize)> = Vec::new();
                for j in news.clone().filter(|&j| !held[1][j]) {
                    let (x, y) = (xs[i], ys[j]);
                    if !self.feasible(x, y) {
                        continue;
                    }
                    if !self.is_list(0, x) {
                        if self.candidate_leaf(x, y) {
                            keys.push((i, j, x, y));
                        }
                    } else if exhaustive {
                        keys.push((i, j, x, y));
                    } else {
                        ranked.push((self.estimate(x, y)?, j));
                    }
                }
                ranked.sort_unstable_by_key(|&(e, j)| (std::cmp::Reverse(e), j));
                let best = ranked.first().map_or(0, |&(e, _)| e);
                for &(_, j) in ranked
                    .iter()
                    .take(self.candidates())
                    .filter(|&&(e, _)| !self.options.floor || 2 * e >= best)
                {
                    keys.push((i, j, xs[i], ys[j]));
                }
            }
        }
        let open = [0, 1].map(|side| {
            matched[side]
                .iter()
                .zip(&held[side])
                .map(|(&m, &h)| (exhaustive || !m) && !h)
                .collect::<Vec<bool>>()
        });
        for i in 0..xs.len() {
            if open[0][i] {
                for (j, y) in self.reach(0, xs[i], b, ys, &open[1])? {
                    keys.push((i, j, xs[i], y));
                }
            }
        }
        for j in 0..ys.len() {
            if open[1][j] {
                for (i, x) in self.reach(1, ys[j], a, xs, &open[0])? {
                    keys.push((i, j, x, ys[j]));
                }
            }
        }
        keys.extend(
            fixed
                .iter()
                .filter(|f| f.deep)
                .map(|f| (f.old, f.new, f.x, f.y)),
        );
        keys.sort_unstable();
        keys.dedup();
        self.spend(keys.len())?;
        let mut deep: Vec<(usize, usize, u32, u32)> = Vec::new();
        let mut reached = held;
        for (i, j, x, y) in keys {
            if !fits(&fixed, (i, i), (j, j)) || !self.respects(x, y) {
                continue;
            }
            let c = (x == xs[i] && ctx.0, y == ys[j] && ctx.1);
            let cell = if self.is_list(0, x) {
                if !self.feasible(x, y) {
                    continue;
                }
                let e = self.savings(x, y)?;
                Cell {
                    old: (i, i),
                    new: (j, j),
                    value: self.value(&e.d),
                    exact: true,
                    fixed: false,
                    act: Act::List(x, y),
                }
            } else {
                let Some(e) = self.leaf(x, y, c, false)? else {
                    continue;
                };
                Cell {
                    old: (i, i),
                    new: (j, j),
                    value: self.value(&e.d),
                    exact: !self.inexact(x, y),
                    fixed: false,
                    act: Act::Leaf(x, y, c),
                }
            };
            if cell.value <= 0 {
                continue;
            }
            reached[0][i] = true;
            reached[1][j] = true;
            if x != xs[i] || y != ys[j] {
                deep.push((i, j, x, y));
            }
            out.push(cell);
        }
        self.wraps(xs, ys, a, b, ctx, &deep, &gaps, &reached, &mut out)?;
        out.retain(|c| fits(&fixed, c.old, c.new));
        for f in fixed.into_iter().filter(|f| !f.deep) {
            let e = self.savings(f.x, f.y)?;
            out.push(Cell {
                old: (f.old, f.old),
                new: (f.new, f.new),
                value: self.value(&e.d),
                exact: true,
                fixed: true,
                act: Act::List(f.x, f.y),
            });
        }
        Ok(out)
    }

    fn anchored(&mut self, xs: &[u32], ys: &[u32]) -> Result<Vec<Anchor>, Fallback> {
        let (Some(&first), Some(&last)) = (xs.first(), xs.last()) else {
            return Ok(Vec::new());
        };
        let (Some(&low), Some(&high)) = (ys.first(), ys.last()) else {
            return Ok(Vec::new());
        };
        let (end, top) = (self.next(0, last), self.next(1, high));
        let from = self.anchors.partition_point(|&(x, _)| x < first);
        let to = self.anchors.partition_point(|&(x, _)| x < end);
        self.spend(to - from)?;
        let mut found: Vec<(usize, usize, u32, u32)> = Vec::new();
        for &(x, y) in &self.anchors[from..to] {
            if y < low || y >= top {
                continue;
            }
            let i = xs.partition_point(|&c| c <= x) - 1;
            let j = ys.partition_point(|&c| c <= y) - 1;
            if found.last().is_none_or(|f| (f.0, f.1) != (i, j)) {
                found.push((i, j, x, y));
            }
        }
        if found.is_empty() {
            return Ok(Vec::new());
        }
        let mut rows = vec![0u32; xs.len()];
        let mut cols = vec![0u32; ys.len()];
        for f in &found {
            rows[f.0] += 1;
            cols[f.1] += 1;
        }
        found.retain(|f| rows[f.0] == 1 && cols[f.1] == 1);
        let keyed: Vec<(u32, u32, u64)> = found
            .iter()
            .enumerate()
            .map(|(k, f)| {
                let w = self.total(0, xs[f.0]) + self.total(1, ys[f.1]);
                (k as u32, f.1 as u32, w as u64)
            })
            .collect();
        let kept = heaviest_increasing(&keyed);
        let mut out = Vec::new();
        for (k, (i, j, x, y)) in found.into_iter().enumerate() {
            if !kept[k] {
                continue;
            }
            let (p, q) = (xs[i], ys[j]);
            let ends = if self.feasible(p, q) && self.respects(p, q) {
                Some((p, q))
            } else if let Some(n) =
                self.outermost(1, y, q, |m, n| m.feasible(p, n) && m.respects(p, n))
            {
                Some((p, n))
            } else {
                self.outermost(0, x, p, |m, n| m.feasible(n, q) && m.respects(n, q))
                    .map(|n| (n, q))
            };
            if let Some((x, y)) = ends {
                out.push(Anchor {
                    old: i,
                    new: j,
                    x,
                    y,
                    deep: (x, y) != (p, q),
                });
            }
        }
        Ok(out)
    }

    fn outermost(
        &self,
        side: usize,
        from: u32,
        top: u32,
        fits: impl Fn(&Self, u32) -> bool,
    ) -> Option<u32> {
        let mut found = None;
        let mut at = from;
        while at != top && at != NONE {
            if fits(self, at) {
                found = Some(at);
            }
            at = self.parents[side][at as usize];
        }
        found
    }

    fn respects(&self, x: u32, y: u32) -> bool {
        [(0, x, y), (1, y, x)].into_iter().all(|(side, n, m)| {
            let o = self.owner[side][n as usize];
            if o == NONE {
                return true;
            }
            let pair = self.fixed[o as usize];
            let (root, peer) = if side == 0 { pair } else { (pair.1, pair.0) };
            let (own, other) = (self.unit(side, root), self.unit(1 - side, peer));
            if n == root {
                m == peer
            } else if n == own.1 {
                m == other.1
            } else {
                other.0 <= m && m < self.next(1 - side, other.1)
            }
        })
    }

    fn identities(&mut self) -> Result<(), Fallback> {
        let mut ids: HashMap<(u32, &'a str, &'a str), u32> = HashMap::default();
        let mut keyed: [Vec<(u32, u32, u32)>; 2] = Default::default();
        let mut seen: HashMap<u32, [(u32, u32); 2]> = HashMap::default();
        for side in [0, 1] {
            let tree = self.trees[side];
            self.spend(tree.items.len())?;
            let mut stack: Vec<(u32, u32, u32)> = Vec::new();
            for item in &tree.items {
                while stack.last().is_some_and(|&(end, _, _)| end <= item.node) {
                    stack.pop();
                }
                let (scope, parent) = stack.last().map_or((NONE, 0), |&(_, k, n)| (k, n));
                let next = ids.len() as u32;
                let key = *ids
                    .entry((scope, item.kind, tree.text(item.key.clone())))
                    .or_insert(next);
                let slot = &mut seen.entry(key).or_insert([(0, NONE); 2])[side];
                *slot = (slot.0 + 1, item.node);
                keyed[side].push((item.node, key, parent));
                stack.push((self.next(side, item.node), key, item.node));
            }
        }
        let parent_of: HashMap<u32, u32> = keyed[1].iter().map(|&(n, _, p)| (n, p)).collect();
        let mut pairs: HashMap<u32, u32> = HashMap::default();
        let mut groups: Vec<(u32, u32, u32)> = Vec::new();
        for &(x, key, parent) in &keyed[0] {
            let [(1, _), (1, y)] = seen[&key] else {
                continue;
            };
            let peer = parent_of[&y];
            let enclosed = parent == 0 && peer == 0 || pairs.get(&parent) == Some(&peer);
            if enclosed && self.feasible(x, y) {
                pairs.insert(x, y);
                groups.push((parent, x, y));
            }
        }
        groups.sort_unstable();
        let (old, new) = (self.trees[0], self.trees[1]);
        let mut placed: HashSet<u32> = HashSet::default();
        for group in groups.chunk_by(|p, q| p.0 == q.0) {
            let parent = group[0].0;
            if parent != 0 && !placed.contains(&parent) {
                continue;
            }
            let keys: Vec<(u32, u32, u64)> = group
                .iter()
                .enumerate()
                .map(|(k, &(_, x, y))| (k as u32, y, (self.total(0, x) + self.total(1, y)) as u64))
                .collect();
            let kept = heaviest_increasing(&keys);
            for (k, &(_, x, y)) in group.iter().enumerate() {
                if kept[k] {
                    self.anchors.push((x, y));
                    if let Some(body) = self.body(x, y) {
                        self.anchors.push(body);
                    }
                } else if whole_lines(old, x) && whole_lines(new, y) {
                    self.forced.push((x, y));
                } else {
                    continue;
                }
                placed.insert(x);
            }
        }
        self.anchors.sort_unstable();
        self.forced.sort_unstable();
        self.fixed = self.anchors.iter().chain(&self.forced).copied().collect();
        for side in [0, 1] {
            let mut roots: Vec<((u32, u32), u32)> = self
                .fixed
                .iter()
                .enumerate()
                .map(|(k, &(x, y))| (self.unit(side, if side == 0 { x } else { y }), k as u32))
                .collect();
            roots.sort_unstable();
            for ((lead, unit), k) in roots {
                let end = self.next(side, unit);
                self.spend((end - lead) as usize)?;
                self.owner[side][lead as usize..end as usize].fill(k);
            }
        }
        Ok(())
    }

    fn body(&self, x: u32, y: u32) -> Option<(u32, u32)> {
        let (old, new) = (self.trees[0], self.trees[1]);
        let lists = |tree: &Tree<'_>, item: u32| -> Vec<u32> {
            tree.children(item)
                .into_iter()
                .filter(|&c| tree.is_list(c) && !tree.nodes[c as usize].open.is_empty())
                .collect()
        };
        let (a, b) = (lists(old, x), lists(new, y));
        let kind = |tree: &Tree<'_>, n: u32| tree.nodes[n as usize].kind;
        let pairs: Vec<(u32, u32)> = a
            .iter()
            .filter_map(|&p| {
                let q = b.iter().copied().find(|&q| kind(new, q) == kind(old, p))?;
                let unique = a.iter().filter(|&&o| kind(old, o) == kind(old, p)).count() == 1
                    && b.iter().filter(|&&n| kind(new, n) == kind(new, q)).count() == 1;
                (unique && self.feasible(p, q)).then_some((p, q))
            })
            .collect();
        pairs.last().copied()
    }

    fn candidate_leaf(&self, x: u32, y: u32) -> bool {
        let (na, nb) = (
            &self.trees[0].nodes[x as usize],
            &self.trees[1].nodes[y as usize],
        );
        match na.class {
            Class::Plain => na.name && nb.name || self.text(0, x) == self.text(1, y),
            _ => true,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn wraps(
        &mut self,
        xs: &[u32],
        ys: &[u32],
        a: u32,
        b: u32,
        ctx: Ctx,
        deep: &[(usize, usize, u32, u32)],
        gaps: &[(Range<usize>, Range<usize>)],
        reached: &[Vec<bool>; 2],
        out: &mut Vec<Cell>,
    ) -> Result<(), Fallback> {
        let mut runs: Vec<(usize, usize, usize, u32, bool)> = Vec::new();
        for &(i, j, x, y) in deep {
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
            let bare = |k: usize| !reached[usize::from(!wrap)][k];
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
            let span = (from as u32, to as u32);
            let (old, new, act) = if wrap {
                let ends = (xs[from], xs[to]);
                (
                    (from, to),
                    (other, other),
                    Act::Wrap(span.0, span.1, host, ends),
                )
            } else {
                let ends = (ys[from], ys[to]);
                (
                    (other, other),
                    (from, to),
                    Act::Unwrap(span.0, span.1, host, ends),
                )
            };
            let mut cell = Cell {
                old,
                new,
                value: 0,
                exact: true,
                fixed: false,
                act,
            };
            let key = self.wrap_key(&cell);
            let entry = match self.wrapped.get(&key) {
                Some(&entry) => entry,
                None => {
                    let inner = if wrap {
                        self.trees[1].children(host)
                    } else {
                        self.trees[0].children(host)
                    };
                    let mut cells = if wrap {
                        self.cells(&xs[from..=to], &inner, a, host, (ctx.0, false))?
                    } else {
                        self.cells(&inner, &ys[from..=to], host, b, (false, ctx.1))?
                    };
                    let mut entry = Entry::default();
                    let chosen = self.select(&mut cells)?;
                    for &k in &chosen {
                        let c = self.features(&cells[k])?;
                        entry.d += c.d;
                        entry.hyp |= c.hyp;
                    }
                    let acts = chosen.iter().map(|&k| cells[k].act).collect();
                    self.wrap_acts.insert(key, acts);
                    self.wrapped.insert(key, entry);
                    entry
                }
            };
            cell.value = self.value(&entry.d);
            if cell.value > 0 {
                out.push(cell);
            }
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
        nodes: &[u32],
        open: &[bool],
    ) -> Result<Vec<(usize, u32)>, Fallback> {
        let other = 1 - side;
        let exhaustive = self.options.exhaustive;
        let index = |n: u32| nodes.binary_search(&n).ok().filter(|&i| open[i]);
        let fits = |m: &Self, target: u32| {
            if side == 0 {
                m.feasible(node, target)
            } else {
                m.feasible(target, node)
            }
        };
        let pins = if side == 0 {
            &self.pins
        } else {
            &self.pins_new
        };
        let end = self.next(side, node);
        let from = pins.partition_point(|&(x, _)| x < node);
        let to = pins.partition_point(|&(x, _)| x < end);
        let mut votes: Vec<(u32, i64)> = Vec::new();
        let mut steps = to - from;
        for k in from..to {
            let (p, q) = if side == 0 {
                self.pins[k]
            } else {
                self.pins_new[k]
            };
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
                    votes.push((at, mass));
                }
                at = self.parents[other][at as usize];
                steps += 1;
            }
        }
        self.spend(steps)?;
        votes.sort_unstable_by_key(|&(t, _)| t);
        votes.dedup_by(|b, a| {
            let same = a.0 == b.0;
            if same {
                a.1 += b.1;
            }
            same
        });
        if votes.is_empty() && self.significant(side, node) {
            let end = self.next(other, scope);
            let limit = if exhaustive {
                usize::MAX
            } else {
                MAX_HASH_HITS
            };
            let parent = self.parents[side][node as usize];
            let lone = self.is_list(side, node)
                || node == parent + 1 && self.next(side, node) == self.next(side, parent);
            let hits: Vec<u32> = match self.hashes[other].get(&self.hash(side, node)) {
                Some(hits) if lone => {
                    let from = hits.partition_point(|&h| h <= scope);
                    hits[from..]
                        .iter()
                        .copied()
                        .take_while(|&h| h < end)
                        .filter(|&h| index(h).is_none())
                        .take(limit)
                        .collect()
                }
                _ => Vec::new(),
            };
            self.spend(hits.len())?;
            for hit in hits {
                if fits(self, hit) {
                    votes.push((hit, self.total(side, node)));
                }
            }
        }
        if let Some(name) = item_at(self.trees[side], node) {
            let name = self.trees[side].text(name);
            self.spend(nodes.len())?;
            for (i, &target) in nodes.iter().enumerate() {
                if open[i]
                    && item_at(self.trees[other], target)
                        .is_some_and(|n| self.trees[other].text(n) == name)
                    && fits(self, target)
                {
                    votes.retain(|&(t, _)| t != target);
                    votes.push((target, i64::MAX));
                }
            }
        }
        votes.sort_unstable_by_key(|&(t, v)| (std::cmp::Reverse(v), t));
        let best = votes
            .iter()
            .map(|&(_, v)| v)
            .find(|&v| v != i64::MAX)
            .unwrap_or(0);
        let mut out = Vec::new();
        let mut steps = 0;
        for &(target, _) in votes
            .iter()
            .take(self.candidates())
            .filter(|&&(_, v)| exhaustive || !self.options.floor || 2 * v.min(i64::MAX / 2) >= best)
        {
            let mut top = target;
            while top != NONE && index(top).is_none() {
                top = self.parents[other][top as usize];
                steps += 1;
            }
            if let Some(i) = index(top) {
                out.push((i, target));
            }
        }
        self.spend(steps)?;
        Ok(out)
    }

    fn estimate(&mut self, a: u32, b: u32) -> Result<i64, Fallback> {
        for (side, node) in [(0, a), (1, b)] {
            if !self.signatures[side].contains_key(&node) {
                let signature = self.signature(side, node)?;
                self.signatures[side].insert(node, signature);
            }
        }
        let (sa, sb) = (&self.signatures[0][&a], &self.signatures[1][&b]);
        if sa.is_empty() && sb.is_empty() {
            return Ok(1);
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
        Ok(common)
    }

    fn signature(&mut self, side: usize, node: u32) -> Result<Vec<u64>, Fallback> {
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
        self.spend((at - node) as usize)?;
        out.sort_unstable();
        out.dedup();
        Ok(out)
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

    fn solve(&mut self) -> Result<(), Fallback> {
        let root = self.savings(0, 0)?;
        self.predicted = root.d;
        self.apply(0, 0)?;
        self.relocate()
    }

    fn apply(&mut self, a: u32, b: u32) -> Result<(), Fallback> {
        self.link(a, b);
        if self.same(a, b) || !self.is_list(0, a) {
            return Ok(());
        }
        let xs = self.trees[0].children(a);
        let ys = self.trees[1].children(b);
        if !self.acts.contains_key(&(a, b)) {
            self.savings(a, b)?;
        }
        let acts = self.acts.get(&(a, b)).cloned().unwrap_or_default();
        self.apply_acts(&xs, &ys, a, b, (true, true), &acts)
    }

    fn apply_acts(
        &mut self,
        xs: &[u32],
        ys: &[u32],
        a: u32,
        b: u32,
        ctx: Ctx,
        acts: &[Act],
    ) -> Result<(), Fallback> {
        if self.options.margins {
            let mut cells = self.cells(xs, ys, a, b, ctx)?;
            let chosen = self.select(&mut cells)?;
            self.margin(&cells, &chosen);
        }
        for &act in acts {
            match act {
                Act::Leaf(x, y, _) => self.link(x, y),
                Act::List(x, y) => self.apply(x, y)?,
                Act::Wrap(from, to, host, ends) => {
                    let inner = self.trees[1].children(host);
                    let run = &xs[from as usize..=to as usize];
                    let acts = self.wrap_acts[&(ends.0, ends.1, host, true)].clone();
                    self.apply_acts(run, &inner, a, host, (ctx.0, false), &acts)?;
                }
                Act::Unwrap(from, to, host, ends) => {
                    let inner = self.trees[0].children(host);
                    let run = &ys[from as usize..=to as usize];
                    let acts = self.wrap_acts[&(ends.0, ends.1, host, false)].clone();
                    self.apply_acts(&inner, run, host, b, (false, ctx.1), &acts)?;
                }
            }
        }
        Ok(())
    }

    fn margin(&mut self, cells: &[Cell], chosen: &[usize]) {
        let overlap = |p: (usize, usize), q: (usize, usize)| p.0 <= q.1 && q.0 <= p.1;
        for &k in chosen {
            let Act::List(x, y) = cells[k].act else {
                continue;
            };
            if item_at(self.trees[0], x).is_none() && item_at(self.trees[1], y).is_none() {
                continue;
            }
            let runner = cells
                .iter()
                .enumerate()
                .filter(|&(c, cell)| {
                    !chosen.contains(&c)
                        && (overlap(cell.old, cells[k].old) || overlap(cell.new, cells[k].new))
                })
                .map(|(_, cell)| cell.value)
                .max()
                .unwrap_or(0);
            self.margins.push(Margin {
                kind: "item",
                old: self.trees[0].nodes[x as usize].span.start,
                new: self.trees[1].nodes[y as usize].span.start,
                best: cells[k].value >> TIE,
                runner: runner >> TIE,
            });
        }
    }

    /// Pairs subtrees left unlinked whose correspondence pays for a move
    /// note: anchors that landed outside their container, equal subtrees
    /// and same-named items, best first.
    fn relocate(&mut self) -> Result<(), Fallback> {
        for (x, y) in self.forced.clone() {
            let (a, b) = (self.unit(0, x), self.unit(1, y));
            let mut pairs = vec![(x, y)];
            if self.feasible(a.1, b.1) {
                pairs = vec![(a.1, b.1)];
                let (p, q) = (self.siblings(0, a.0, a.1), self.siblings(1, b.0, b.1));
                if p.len() == q.len() {
                    pairs.extend(p.into_iter().zip(q));
                }
            }
            for (p, q) in pairs {
                if self.feasible(p, q)
                    && self.free(0, p)?
                    && self.free(1, q)?
                    && self.disjoint(p, q)
                {
                    let e = self.moved(p, q)?;
                    self.apply(p, q)?;
                    self.moves.push((p, q));
                    self.predicted += e.d;
                }
            }
        }
        let mut candidates: Vec<(u32, u32)> = Vec::new();
        let pins = self.pins.clone();
        self.spend(pins.len())?;
        for &(p, q) in &pins {
            if self.partner[0][p as usize] != NONE || self.partner[1][q as usize] != NONE {
                continue;
            }
            let olds = self.open_chain(0, p);
            let news = self.open_chain(1, q);
            self.spend(olds.len() * news.len())?;
            for &x in &olds {
                if let Some(&y) = news.iter().find(|&&y| self.feasible(x, y)) {
                    candidates.push((x, y));
                }
            }
        }
        let (old, new) = (self.trees[0], self.trees[1]);
        let mut named: HashMap<&str, Vec<u32>> = HashMap::default();
        self.spend(old.items.len() + new.items.len())?;
        for item in &new.items {
            if self.partner[1][item.node as usize] == NONE {
                named
                    .entry(new.text(item.name.clone()))
                    .or_default()
                    .push(item.node);
            }
        }
        for item in &old.items {
            if self.partner[0][item.node as usize] != NONE {
                continue;
            }
            if let Some([y]) = named.get(old.text(item.name.clone())).map(Vec::as_slice) {
                candidates.push((item.node, *y));
            }
        }
        candidates.extend(self.duplicates()?);
        candidates.sort_unstable();
        candidates.dedup();
        let notes = Notes::new(self.trees[0], self.trees[1]);
        let weights = self.options.weights;
        let w = Weights {
            mu: Self::scale(weights.mu),
            follow: Self::scale(weights.follow),
            ..weights
        };
        let mut valued = Vec::new();
        for (x, y) in candidates {
            if !self.feasible(x, y)
                || !self.respects(x, y)
                || !whole_lines(self.trees[0], x)
                || !whole_lines(self.trees[1], y)
                || !self.free(0, x)?
                || !self.free(1, y)?
            {
                continue;
            }
            let e = self.moved(x, y)?;
            let value = self.value(&e.d);
            if self.options.margins {
                self.margins.push(Margin {
                    kind: "move",
                    old: self.trees[0].nodes[x as usize].span.start,
                    new: self.trees[1].nodes[y as usize].span.start,
                    best: (value - w.mu) >> TIE,
                    runner: 0,
                });
            }
            if value > w.follow.min(w.mu) {
                valued.push((value, x, y, e.d));
            }
        }
        valued.sort_unstable_by_key(|&(v, x, y, _)| (std::cmp::Reverse(v), x, y));
        let notes_of: Vec<diffy_diff::Move> = valued
            .iter()
            .map(|&(_, x, y, _)| notes.place(x, y))
            .collect();
        let near = |a: &diffy_diff::Move, b: &diffy_diff::Move| {
            a.from.is_some()
                && a.from == b.from
                && a.from != a.into
                && b.from != b.into
                && a.new_line + a.lines + MOVE_GROUP_GAP >= b.new_line
                && b.new_line + b.lines + MOVE_GROUP_GAP >= a.new_line
        };
        self.spend(valued.len() * valued.len())?;
        let mut placed: Vec<usize> = Vec::new();
        let mut taken = vec![false; valued.len()];
        loop {
            let mut grew = false;
            for k in 0..valued.len() {
                let (value, x, y, d) = valued[k];
                if taken[k] || !self.free(0, x)? || !self.free(1, y)? || !self.disjoint(x, y) {
                    continue;
                }
                let joins =
                    placed.iter().any(|&p| near(&notes_of[p], &notes_of[k])) || notes.cheap(x, y);
                let pays = if joins {
                    value > w.follow
                } else {
                    let rest: i64 = (0..valued.len())
                        .filter(|&j| j != k && !taken[j] && near(&notes_of[k], &notes_of[j]))
                        .map(|j| (valued[j].0 - w.follow).max(0))
                        .sum();
                    value - w.mu + rest > 0
                };
                if !pays {
                    continue;
                }
                self.apply(x, y)?;
                self.moves.push((x, y));
                self.predicted += d;
                placed.push(k);
                taken[k] = true;
                grew = true;
            }
            if !grew {
                break;
            }
        }
        let (leaders, followers) = notes.count(&self.moves);
        self.predicted[MOVES] -= leaders;
        self.predicted[FOLLOWS] -= followers;
        Ok(())
    }

    fn unit(&self, side: usize, node: u32) -> (u32, u32) {
        let items = &self.trees[side].items;
        match items.binary_search_by_key(&node, |i| i.node) {
            Ok(at) => (items[at].lead, items[at].unit),
            Err(_) => (node, node),
        }
    }

    fn siblings(&self, side: usize, from: u32, to: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut at = from;
        while at < to {
            out.push(at);
            at = self.next(side, at);
        }
        out
    }

    fn moved(&mut self, x: u32, y: u32) -> Result<Entry, Fallback> {
        let mut e = self.savings(x, y)?;
        let unmatched = self.total(0, x) + self.total(1, y) - e.d[UNMATCHED];
        e.d[MOVED] -= unmatched;
        Ok(e)
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
        let mut found: [BTreeMap<u64, Vec<u32>>; 2] = Default::default();
        for side in [0, 1] {
            let tree = self.trees[side];
            self.spend(tree.nodes.len())?;
            let mut at = 1;
            while at < tree.nodes.len() as u32 {
                let end = tree.nodes[at as usize].next;
                if tree.is_list(at)
                    && whole_lines(tree, at)
                    && (at..end).all(|k| self.partner[side][k as usize] == NONE)
                {
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
            if let Some(targets) = news.get(hash) {
                out.extend(olds.iter().copied().zip(targets.iter().copied()));
            }
        }
        Ok(out)
    }

    /// Accepts each rename whose linked uses pay for its entry, and
    /// charges the rejected ones back to the prediction.
    fn finalize(&mut self) -> (Vec<(&'a str, &'a str, u32)>, bool) {
        let (old, new) = (self.trees[0], self.trees[1]);
        let mut uses: BTreeMap<(&'a str, &'a str), (Features, u32, i64)> = BTreeMap::new();
        for k in 0..old.nodes.len() as u32 {
            let b = self.partner[0][k as usize];
            if b == NONE {
                continue;
            }
            let (na, nb) = (&old.nodes[k as usize], &new.nodes[b as usize]);
            let (p, q) = (self.text(0, k), self.text(1, b));
            if na.class != Class::Plain || !na.name || !nb.name || !self.rename_ok(p, q) {
                continue;
            }
            let (wa, wb) = (self.own(0, k), self.own(1, b));
            let ctx = (
                self.linked(0, self.parents[0][k as usize]),
                self.linked(1, self.parents[1][b as usize]),
            );
            let entry = uses.entry((p, q)).or_default();
            entry.0[UNMATCHED] += wa + wb;
            entry.0[INSIDE] += if ctx.0 { wa } else { 0 } + if ctx.1 { wb } else { 0 };
            entry.0[RENAMED] -= wa + wb;
            if !ctx.0 && !ctx.1 {
                entry.0[ISOLATED] -= 1;
            }
            if self.split(na.span.start, nb.span.start) {
                entry.0[SPLIT] -= 1;
            }
            entry.1 += 1;
        }
        for item in &old.items {
            let b = self.partner[0][item.node as usize];
            if b == NONE {
                continue;
            }
            let Some(name) = item_at(new, b) else {
                continue;
            };
            let (p, q) = (old.text(item.name.clone()), new.text(name));
            if self.rename_ok(p, q) {
                uses.entry((p, q)).or_default().2 += 1;
            }
        }
        let mut targets: HashMap<&str, u32> = HashMap::default();
        for &(p, _) in uses.keys() {
            *targets.entry(p).or_default() += 1;
        }
        let vetoed = self.vetoed(uses.keys().copied());
        let mut accepted = Vec::new();
        let mut rejected = false;
        for ((p, q), (d, count, items)) in uses {
            let w = self.options.weights;
            let gain = self.value(&d) + Self::scale(w.iota * items);
            if count > 0
                && targets[p] == 1
                && gain > Self::scale(w.rho0)
                && !vetoed.contains(&(p, q))
            {
                self.predicted[ENTRIES] -= 1;
                accepted.push((p, q, count));
            } else {
                self.predicted -= d;
                self.predicted[IDENTITY] -= items;
                rejected = true;
            }
        }
        (accepted, rejected)
    }

    fn vetoed(
        &mut self,
        pairs: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> HashSet<(&'a str, &'a str)> {
        if let Some(done) = &self.vetoed {
            return done.clone();
        }
        let Some(vet) = self.vet else {
            return HashSet::default();
        };
        let called = |tree: &Tree<'a>| {
            let mut out: HashMap<&'a str, bool> = HashMap::default();
            for n in tree.nodes.iter().filter(|n| n.name) {
                *out.entry(tree.text(n.span.clone())).or_insert(true) &= n.callee;
            }
            out
        };
        let (old, new) = (called(self.trees[0]), called(self.trees[1]));
        let candidates: Vec<(&'a str, &'a str)> = pairs
            .filter(|(p, q)| old.get(p) == Some(&true) && new.get(q) == Some(&true))
            .collect();
        let started = Instant::now();
        let keep = if candidates.is_empty() {
            Vec::new()
        } else {
            vet(&candidates)
        };
        self.deadline += started.elapsed();
        let done: HashSet<(&'a str, &'a str)> = candidates
            .into_iter()
            .zip(keep.into_iter().chain(std::iter::repeat(false)))
            .filter(|&(_, keep)| !keep)
            .map(|(pair, _)| pair)
            .collect();
        self.vetoed = Some(done.clone());
        done
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
        let mut groups: BTreeMap<u32, Vec<(u32, u32, u32, u64)>> = BTreeMap::new();
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
                    entries.push((x.open.start, y.open.start, a, cost::DELIMITER as u64));
                    entries.push((x.close.start, y.close.start, a, cost::DELIMITER as u64));
                }
            } else {
                entries.push((x.span.start, y.span.start, a, self.own(0, a) as u64));
            }
        }
        for (_, mut entries) in groups {
            entries.sort_unstable();
            let keyed: Vec<(u32, u32, u64)> = entries
                .iter()
                .enumerate()
                .map(|(k, &(_, y, _, w))| (k as u32, y, w))
                .collect();
            let kept = heaviest_increasing(&keyed);
            for (k, &(_, _, node, _)) in entries.iter().enumerate() {
                if !kept[k] && self.partner[0][node as usize] != NONE {
                    self.unlink(node);
                }
            }
        }
        Ok(())
    }
}

fn fits(fixed: &[Anchor], old: (usize, usize), new: (usize, usize)) -> bool {
    let k = fixed.partition_point(|f| f.old < old.0);
    let covers = |f: &Anchor| f.deep && f.old <= old.1 && new.0 <= f.new && f.new <= new.1;
    let high = k + usize::from(fixed.get(k).is_some_and(covers));
    let before = k.checked_sub(1).is_none_or(|p| fixed[p].new < new.0);
    let after = fixed
        .get(high)
        .is_none_or(|f| old.1 < f.old && new.1 < f.new);
    before && after
}

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

fn item_at(tree: &Tree<'_>, node: u32) -> Option<Range<u32>> {
    let at = tree
        .items
        .binary_search_by_key(&node, |item| item.node)
        .ok()?;
    Some(tree.items[at].name.clone())
}

fn mass(tree: &Tree<'_>) -> Mass {
    let mut own = Vec::with_capacity(tree.nodes.len());
    let mut sum = Vec::with_capacity(tree.nodes.len() + 1);
    sum.push(0);
    for node in &tree.nodes {
        let w = cost::weight(tree, node);
        own.push(w);
        sum.push(sum.last().copied().unwrap_or(0) + w);
    }
    Mass { own, sum }
}

fn plain<'a>(tree: &Tree<'a>) -> HashSet<&'a str> {
    tree.nodes
        .iter()
        .filter(|node| node.class == Class::Plain)
        .map(|node| tree.text(node.span.clone()))
        .collect()
}
