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

mod apply;
mod cells;
mod identity;
mod moves;
mod renames;
mod score;
mod validate;

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
