use std::collections::HashMap;
use std::ops::Range;
use std::time::Instant;

use diffy_diff::myers;

use crate::Fallback;
use crate::syntax::{Class, NONE, Tree};

const SIGNATURE_LEAVES: usize = 64;
const MIN_SIMILARITY: f32 = 0.3;
const RENAMED_ITEM_SIMILARITY: f32 = 0.6;
const TEXT_SIMILARITY: f32 = 0.5;
const NAME_SIMILARITY: f32 = 0.4;
const MIN_DICE: f32 = 0.5;
const MIN_AGREEMENT: u32 = 2;
const MAX_PAIR_CELLS: usize = 1 << 16;
const MAX_WRAP_SEARCH: usize = 64;
const MIN_MOVE_NODES: u32 = 12;
const MOVE_KEPT: f32 = 0.6;
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
        leaves: [leaf_counts(old), leaf_counts(new)],
        names: [named(old), named(new)],
        moves: Vec::new(),
        work: 0,
        max_work,
        clock_at: CLOCK_STRIDE,
        deadline,
    };
    if old.nodes.is_empty() || new.nodes.is_empty() {
        return Ok(m.finish());
    }
    m.link(0, 0);
    m.anchor()?;
    m.containers()?;
    m.scope()?;
    m.recover()?;
    m.relocate()?;
    m.scope()?;
    m.order()?;
    m.monotone()?;
    Ok(m.finish())
}

struct Matcher<'t, 'a> {
    trees: [&'t Tree<'a>; 2],
    partner: [Vec<u32>; 2],
    parents: [Vec<u32>; 2],
    leaves: [Vec<u32>; 2],
    names: [HashMap<&'a str, Vec<u32>>; 2],
    moves: Vec<(u32, u32)>,
    work: u64,
    max_work: u64,
    clock_at: u64,
    deadline: Instant,
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

    fn leaves_in(&self, side: usize, node: u32) -> u32 {
        let leaves = &self.leaves[side];
        leaves[self.next(side, node) as usize] - leaves[node as usize]
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

    /// Links two equal subtrees node by node.
    fn same(&mut self, a: u32, b: u32) -> bool {
        let (old, new) = (self.trees[0], self.trees[1]);
        let len = self.next(0, a) - a;
        if self.hash(0, a) != self.hash(1, b) || self.next(1, b) - b != len {
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

    /// Lists pair when kind and brackets agree; an unbracketed list that
    /// only wraps one child pairs only with a list whose first child is of
    /// the same kind.
    fn pairable(&self, a: u32, b: u32) -> bool {
        let (ta, tb) = (self.trees[0], self.trees[1]);
        let (na, nb) = (&ta.nodes[a as usize], &tb.nodes[b as usize]);
        na.class == Class::List
            && nb.class == Class::List
            && na.kind == nb.kind
            && ta.text(na.open.clone()) == tb.text(nb.open.clone())
            && ta.text(na.close.clone()) == tb.text(nb.close.clone())
            && (!na.open.is_empty()
                || !(lone(ta, a) || lone(tb, b))
                || ta.nodes[a as usize + 1].kind == tb.nodes[b as usize + 1].kind)
    }

    /// Whether two lists hold children of the same kinds in the same order.
    fn same_shape(&self, a: u32, b: u32) -> bool {
        let (old, new) = (self.trees[0], self.trees[1]);
        let kinds = |tree: &Tree<'_>, list: u32| -> Vec<u16> {
            tree.children(list)
                .iter()
                .map(|&c| tree.nodes[c as usize].kind)
                .collect()
        };
        kinds(old, a) == kinds(new, b)
    }

    fn bracketed(&self, side: usize, node: u32) -> bool {
        !self.trees[side].nodes[node as usize].open.is_empty()
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

    /// Phase 1: subtrees whose hash occurs exactly once on each side.
    fn anchor(&mut self) -> Result<(), Fallback> {
        let (old, new) = (self.trees[0], self.trees[1]);
        self.spend(old.nodes.len() + new.nodes.len())?;
        let count = |side: usize| {
            let tree = self.trees[side];
            let mut out: HashMap<u64, (u32, u32)> = HashMap::new();
            for node in 1..tree.nodes.len() as u32 {
                if self.significant(side, node) && !self.bracketed(side, node) {
                    let entry = out.entry(self.hash(side, node)).or_insert((0, node));
                    entry.0 += 1;
                }
            }
            out
        };
        let (olds, news) = (count(0), count(1));
        let mut unique: Vec<(u32, u32)> = olds
            .iter()
            .filter(|(_, (n, _))| *n == 1)
            .filter_map(|(hash, &(_, o))| match news.get(hash) {
                Some(&(1, n)) => Some((o, n)),
                _ => None,
            })
            .collect();
        unique.sort_unstable_by_key(|&(o, _)| (std::cmp::Reverse(self.next(0, o) - o), o));
        for (o, n) in unique {
            if self.partner[0][o as usize] == NONE && self.partner[1][n as usize] == NONE {
                self.same(o, n);
            }
        }
        Ok(())
    }

    /// Phase 2: from the root down, each unlinked old list pairs with the
    /// item of the same name inside its parent's partner, else with the
    /// list there that holds most of its anchored descendants, the
    /// shallowest and then the closest on a tie.
    fn containers(&mut self) -> Result<(), Fallback> {
        let mut pairs = vec![(0u32, 0u32)];
        while let Some((p, q)) = pairs.pop() {
            let mut todo: Vec<u32> = self.trees[0].children(p);
            todo.reverse();
            while let Some(x) = todo.pop() {
                if !self.is_list(0, x) {
                    continue;
                }
                let px = self.partner[0][x as usize];
                if px != NONE {
                    if self.is_list(1, px) {
                        pairs.push((x, px));
                    }
                    continue;
                }
                let found = match self.namesake(x, p, q) {
                    Some(c) => Some(c),
                    None => self.candidate(x, q)?,
                };
                match found {
                    Some(c) => {
                        self.link(x, c);
                        pairs.push((x, c));
                    }
                    None => todo.extend(self.trees[0].children(x).into_iter().rev()),
                }
            }
        }
        Ok(())
    }

    /// The one unlinked item inside `q` named like `x`, when `x` is the
    /// one item of that name inside `p`.
    fn namesake(&self, x: u32, p: u32, q: u32) -> Option<u32> {
        let name = item_name(self.trees[0], x)?;
        let one = |side: usize, list: u32| {
            let end = self.next(side, list);
            let mut inside = self.names[side]
                .get(name)?
                .iter()
                .copied()
                .filter(|&n| n > list && n < end);
            let first = inside.next()?;
            inside.next().is_none().then_some(first)
        };
        one(0, p)?;
        let c = one(1, q)?;
        (self.partner[1][c as usize] == NONE && self.pairable(x, c)).then_some(c)
    }

    fn candidate(&mut self, x: u32, q: u32) -> Result<Option<u32>, Fallback> {
        let end = self.next(0, x);
        let scope = q..self.next(1, q);
        self.spend((end - x) as usize)?;
        let mut votes: Vec<(u32, u32, u32)> = Vec::new();
        let mut total = 0;
        let mut at = x + 1;
        while at < end {
            let y = self.partner[0][at as usize];
            if y == NONE {
                at += 1;
                continue;
            }
            let weight = self.leaves_in(0, at).max(1);
            at = self.next(0, at);
            if y <= scope.start || y >= scope.end {
                continue;
            }
            total += weight;
            let mut path = Vec::new();
            let mut c = self.parents[1][y as usize];
            while c != NONE && c != q {
                path.push(c);
                c = self.parents[1][c as usize];
            }
            let depth = path.len() as u32;
            for (k, &c) in path.iter().enumerate() {
                if self.partner[1][c as usize] != NONE || !self.pairable(x, c) {
                    continue;
                }
                let level = depth - k as u32;
                match votes.iter_mut().find(|(v, _, _)| *v == c) {
                    Some(entry) => entry.1 += weight,
                    None => votes.push((c, weight, level)),
                }
            }
        }
        let own = self.leaves_in(0, x);
        let best = votes
            .iter()
            .filter(|&&(c, common, level)| {
                let dice = 2.0 * common as f32 / (own + self.leaves_in(1, c)).max(1) as f32;
                dice >= MIN_DICE || common == total && (level == 1 || total >= MIN_AGREEMENT)
            })
            .max_by(|a, b| {
                let dice = |&(c, common, _): &(u32, u32, u32)| {
                    2.0 * common as f32 / (own + self.leaves_in(1, c)).max(1) as f32
                };
                let shape = |&(c, _, _): &(u32, u32, u32)| self.same_shape(x, c);
                let (depth, form) = (b.2.cmp(&a.2), shape(a).cmp(&shape(b)));
                let place = if self.bracketed(0, x) {
                    depth.then(form)
                } else {
                    form.then(depth)
                };
                a.1.cmp(&b.1)
                    .then(place)
                    .then(dice(a).total_cmp(&dice(b)))
                    .then(b.0.cmp(&a.0))
            });
        Ok(best.map(|&(c, _, _)| c))
    }

    /// Phase 3: inside every linked list pair, outer pairs first, matches
    /// the children no earlier phase linked.
    fn recover(&mut self) -> Result<(), Fallback> {
        let old = self.trees[0];
        let mut queue: Vec<(u32, u32)> = (0..old.nodes.len() as u32)
            .filter(|&o| old.is_list(o))
            .filter_map(|o| {
                let n = self.partner[0][o as usize];
                (n != NONE && self.is_list(1, n)).then_some((o, n))
            })
            .collect();
        let mut done = vec![false; old.nodes.len()];
        let mut i = 0;
        while i < queue.len() {
            let (o, n) = queue[i];
            i += 1;
            if done[o as usize] || self.partner[0][o as usize] != n {
                continue;
            }
            done[o as usize] = true;
            self.recover_pair(o, n, &mut queue)?;
        }
        Ok(())
    }

    fn recover_pair(
        &mut self,
        o: u32,
        n: u32,
        queue: &mut Vec<(u32, u32)>,
    ) -> Result<(), Fallback> {
        let (a, b) = (self.trees[0].children(o), self.trees[1].children(n));
        self.spend(a.len() + b.len())?;
        let at: HashMap<u32, usize> = b.iter().enumerate().map(|(j, &c)| (c, j)).collect();
        let mut fixed: Vec<(usize, usize)> = Vec::new();
        for (i, &c) in a.iter().enumerate() {
            let partner = self.partner[0][c as usize];
            if let Some(&j) = at.get(&partner)
                && fixed.last().is_none_or(|&(_, last)| j > last)
            {
                fixed.push((i, j));
            }
        }
        let (mut i, mut j) = (0, 0);
        for (fi, fj) in fixed.into_iter().chain([(a.len(), b.len())]) {
            let ga: Vec<u32> = a[i..fi]
                .iter()
                .copied()
                .filter(|&c| self.partner[0][c as usize] == NONE)
                .collect();
            let gb: Vec<u32> = b[j..fj]
                .iter()
                .copied()
                .filter(|&c| self.partner[1][c as usize] == NONE)
                .collect();
            self.seq(&ga, &gb, queue)?;
            (i, j) = (fi + 1, fj + 1);
        }
        Ok(())
    }

    fn seq(&mut self, a: &[u32], b: &[u32], queue: &mut Vec<(u32, u32)>) -> Result<(), Fallback> {
        if a.is_empty() || b.is_empty() {
            return Ok(());
        }
        self.spend(a.len() + b.len())?;
        let (ia, ib) = self.intern(a, b);
        let lead_a = |i: usize| self.leads(0, a[i]);
        let lead_b = |i: usize| self.leads(1, b[i]);
        let changes = slide(myers::diff(&ia, &ib), &ia, &ib, [&lead_a, &lead_b]);
        let (mut i, mut j) = (0, 0);
        for change in changes {
            let (old, new) = (
                change.old.start as usize..change.old.end as usize,
                change.new.start as usize..change.new.end as usize,
            );
            while i < old.start {
                self.same(a[i], b[j]);
                (i, j) = (i + 1, j + 1);
            }
            self.gap(&a[old.clone()], &b[new.clone()], queue)?;
            (i, j) = (old.end, new.end);
        }
        while i < a.len() {
            self.same(a[i], b[j]);
            (i, j) = (i + 1, j + 1);
        }
        Ok(())
    }

    /// Interned child hashes; a child with linked descendants gets an id
    /// of its own, since only free subtrees can match whole.
    fn intern(&self, a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
        let mut ids: HashMap<u64, u32> = HashMap::with_capacity(a.len() + b.len());
        let mut fresh = u32::MAX;
        let mut id = |side: usize, node: u32, ids: &mut HashMap<u64, u32>| {
            if !self.free(side, node) {
                fresh -= 1;
                return fresh;
            }
            let next = ids.len() as u32;
            *ids.entry(self.hash(side, node)).or_insert(next)
        };
        let ia = a.iter().map(|&n| id(0, n, &mut ids)).collect();
        let ib = b.iter().map(|&n| id(1, n, &mut ids)).collect();
        (ia, ib)
    }

    fn gap(&mut self, a: &[u32], b: &[u32], queue: &mut Vec<(u32, u32)>) -> Result<(), Fallback> {
        if a.is_empty() || b.is_empty() {
            return Ok(());
        }
        if a.len() == 1 && b.len() == 1 && self.pairable(a[0], b[0]) {
            let alone = self.bracketed(0, a[0])
                || self.unwrap_into(0, a, 1, b, queue)?.len() == 1
                    && self.unwrap_into(1, b, 0, a, queue)?.len() == 1
                    && self.same_shape(a[0], b[0]);
            if alone {
                self.link(a[0], b[0]);
                queue.push((a[0], b[0]));
                return Ok(());
            }
        }
        let a = self.unwrap_into(0, a, 1, b, queue)?;
        let b: Vec<u32> = b
            .iter()
            .copied()
            .filter(|&c| self.partner[1][c as usize] == NONE)
            .collect();
        let b = self.unwrap_into(1, &b, 0, &a, queue)?;
        let a: Vec<u32> = a
            .iter()
            .copied()
            .filter(|&c| self.partner[0][c as usize] == NONE)
            .collect();
        if a.is_empty() || b.is_empty() {
            return Ok(());
        }
        self.pair_similar(&a, &b, queue)
    }

    /// Finds the list inside an unlinked list among `others` that holds a
    /// subtree equal to one of `items`: the items were wrapped there, or
    /// unwrapped from there. Matches the items against that list's
    /// children in order and returns the items left over.
    fn unwrap_into(
        &mut self,
        side: usize,
        items: &[u32],
        other: usize,
        others: &[u32],
        queue: &mut Vec<(u32, u32)>,
    ) -> Result<Vec<u32>, Fallback> {
        if items.len() * others.len() > MAX_WRAP_SEARCH * MAX_WRAP_SEARCH {
            return Ok(items.to_vec());
        }
        let mut inner = self.linked_inner(side, items, other, others);
        for &item in items.iter().filter(|_| inner.is_none()) {
            let rival = self.bracketed(side, item)
                && others.iter().any(|&o| {
                    let (x, y) = if side == 0 { (item, o) } else { (o, item) };
                    self.pairable(x, y)
                });
            let lone_leaf = items.len() == 1 && !self.is_list(side, item);
            if self.significant(side, item)
                && (self.is_list(side, item) || lone_leaf)
                && !rival
                && self.free(side, item)
                && let Some(found) = self.find_inside(item, side, other, others)?
            {
                let list = self.parents[other][found as usize];
                if self.is_list(side, item) || self.bracketed(other, list) {
                    inner = Some(list);
                    break;
                }
            }
        }
        let Some(list) = inner else {
            return Ok(items.to_vec());
        };
        let children: Vec<u32> = self.trees[other]
            .children(list)
            .into_iter()
            .filter(|&c| self.partner[other][c as usize] == NONE)
            .collect();
        if side == 0 {
            self.seq(items, &children, queue)?;
        } else {
            self.seq(&children, items, queue)?;
        }
        Ok(items
            .iter()
            .copied()
            .filter(|&c| self.partner[side][c as usize] == NONE)
            .collect())
    }

    /// The list inside an unlinked list among `others` whose children are
    /// already linked to siblings of `items`.
    fn linked_inner(
        &self,
        side: usize,
        items: &[u32],
        other: usize,
        others: &[u32],
    ) -> Option<u32> {
        let parent = self.parents[side][*items.first()? as usize];
        for &host in others {
            if !self.is_list(other, host) || self.partner[other][host as usize] != NONE {
                continue;
            }
            let end = self.next(other, host);
            for k in host + 1..end {
                let p = self.partner[other][k as usize];
                if p != NONE && self.parents[side][p as usize] == parent {
                    return Some(self.parents[other][k as usize]);
                }
            }
        }
        None
    }

    fn find_inside(
        &mut self,
        item: u32,
        side: usize,
        other: usize,
        others: &[u32],
    ) -> Result<Option<u32>, Fallback> {
        let hash = self.hash(side, item);
        let len = self.next(side, item) - item;
        for &host in others {
            if !self.is_list(other, host) || self.partner[other][host as usize] != NONE {
                continue;
            }
            let end = self.next(other, host);
            self.spend((end - host) as usize)?;
            let mut at = host + 1;
            while at < end {
                if self.partner[other][at as usize] != NONE {
                    at = self.next(other, at);
                    continue;
                }
                if self.hash(other, at) == hash
                    && self.next(other, at) - at == len
                    && self.free(other, at)
                {
                    return Ok(Some(at));
                }
                at += 1;
            }
        }
        Ok(None)
    }

    fn pair_similar(
        &mut self,
        a: &[u32],
        b: &[u32],
        queue: &mut Vec<(u32, u32)>,
    ) -> Result<(), Fallback> {
        let (n, m) = (a.len(), b.len());
        if n * m > MAX_PAIR_CELLS {
            return Ok(());
        }
        self.spend(n * m)?;
        let sa: Vec<Vec<u64>> = a.iter().map(|&x| self.signature(0, x)).collect();
        let sb: Vec<Vec<u64>> = b.iter().map(|&y| self.signature(1, y)).collect();
        let width = m + 1;
        let mut best = vec![0f32; (n + 1) * width];
        let mut score = vec![0f32; n * m];
        for i in 1..=n {
            for j in 1..=m {
                let s = self.score(a[i - 1], b[j - 1], &sa[i - 1], &sb[j - 1]);
                score[(i - 1) * m + (j - 1)] = s;
                let diagonal = if s > 0.0 {
                    best[(i - 1) * width + j - 1] + s
                } else {
                    0.0
                };
                best[i * width + j] = best[(i - 1) * width + j]
                    .max(best[i * width + j - 1])
                    .max(diagonal);
            }
        }
        let (mut i, mut j) = (n, m);
        let mut pairs = Vec::new();
        while i > 0 && j > 0 {
            let s = score[(i - 1) * m + (j - 1)];
            if s > 0.0 && best[i * width + j] == best[(i - 1) * width + j - 1] + s {
                pairs.push((a[i - 1], b[j - 1]));
                (i, j) = (i - 1, j - 1);
            } else if best[i * width + j] == best[(i - 1) * width + j] {
                i -= 1;
            } else {
                j -= 1;
            }
        }
        for (x, y) in pairs.into_iter().rev() {
            self.link(x, y);
            if self.is_list(0, x) {
                queue.push((x, y));
            }
        }
        Ok(())
    }

    fn score(&self, x: u32, y: u32, sx: &[u64], sy: &[u64]) -> f32 {
        let (old, new) = (self.trees[0], self.trees[1]);
        let (nx, ny) = (&old.nodes[x as usize], &new.nodes[y as usize]);
        match (nx.class, ny.class) {
            (Class::List, Class::List) if self.pairable(x, y) => {
                if sx.is_empty() && sy.is_empty() {
                    return MIN_SIMILARITY;
                }
                let similarity = dice(sx, sy);
                let floor = match (item_name(old, x), item_name(new, y)) {
                    (Some(p), Some(q)) if p != q => RENAMED_ITEM_SIMILARITY,
                    _ if self.same_shape(x, y) => return similarity.max(MIN_SIMILARITY),
                    _ => MIN_SIMILARITY,
                };
                if similarity >= floor { similarity } else { 0.0 }
            }
            (Class::Text, Class::Text) | (Class::Comment, Class::Comment) if nx.kind == ny.kind => {
                TEXT_SIMILARITY
            }
            (Class::Plain, Class::Plain) if nx.name && ny.name => NAME_SIMILARITY,
            _ => 0.0,
        }
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

    fn leads(&self, side: usize, node: u32) -> bool {
        let tree = self.trees[side];
        let start = tree.nodes[node as usize].span.start as usize;
        tree.src.as_bytes()[..start]
            .iter()
            .rev()
            .take_while(|&&c| c != b'\n')
            .all(u8::is_ascii_whitespace)
    }

    /// Phase 3b: links whole free subtrees that could be moves to their
    /// equal free counterparts, the k-th to the k-th of each hash.
    fn relocate(&mut self) -> Result<(), Fallback> {
        let mut found: [HashMap<u64, Vec<u32>>; 2] = [HashMap::new(), HashMap::new()];
        for side in [0, 1] {
            let tree = self.trees[side];
            self.spend(tree.nodes.len())?;
            let mut at = 1;
            while at < tree.nodes.len() as u32 {
                let end = tree.nodes[at as usize].next;
                let big = end - at >= MIN_MOVE_NODES || is_item(tree, at);
                if tree.is_list(at) && big && whole_lines(tree, at) && self.free(side, at) {
                    found[side].entry(self.hash(side, at)).or_default().push(at);
                    at = end;
                    continue;
                }
                at += 1;
            }
        }
        let [olds, news] = found;
        let mut hashes: Vec<&u64> = olds.keys().collect();
        hashes.sort_unstable_by_key(|&h| olds[h][0]);
        for hash in hashes {
            let Some(targets) = news.get(hash) else {
                continue;
            };
            if olds[hash].len() != targets.len() {
                continue;
            }
            for (&o, &n) in olds[hash].iter().zip(targets) {
                self.same(o, n);
            }
        }
        Ok(())
    }

    fn movable(&self, a: u32, b: u32) -> bool {
        let (old, new) = (self.trees[0], self.trees[1]);
        let lines = |tree: &Tree<'_>, node: u32| {
            tree.text(tree.nodes[node as usize].span.clone())
                .trim_end()
                .lines()
                .count()
        };
        let item = is_item(old, a);
        old.is_list(a)
            && new.is_list(b)
            && (self.next(0, a) - a >= MIN_MOVE_NODES || item)
            && (item || lines(old, a) > 1 && lines(new, b) > 1)
            && whole_lines(old, a)
            && whole_lines(new, b)
            && self.kept(a, b) >= MOVE_KEPT
    }

    /// The share of leaves in `a` and `b` that correspond to each other
    /// within them.
    fn kept(&self, a: u32, b: u32) -> f32 {
        let (ea, eb) = (self.next(0, a), self.next(1, b));
        let (old, new) = (self.trees[0], self.trees[1]);
        let inside = (a..ea)
            .filter(|&k| {
                let p = self.partner[0][k as usize];
                old.nodes[k as usize].class != Class::List
                    && (b..eb).contains(&p)
                    && old.text(old.nodes[k as usize].span.clone())
                        == new.text(new.nodes[p as usize].span.clone())
            })
            .count() as f32;
        2.0 * inside / (self.leaves_in(0, a) + self.leaves_in(1, b)).max(1) as f32
    }

    /// Phase 4: every link must sit under linked ancestors that correspond.
    /// A link that does not is a move when it can be one, else dropped.
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
                let (o, n) = if side == 0 { (a, b) } else { (b, a) };
                if self.movable(o, n) {
                    self.add_move(o, n);
                    stack.push((own.nodes[a as usize].next, b));
                } else {
                    self.unlink(o);
                }
            }
        }
        Ok(())
    }

    /// Records a move unless it overlaps one already recorded. A move that
    /// lands inside another's destination is part of it; one that holds
    /// others in the same way absorbs them; any other overlap drops the
    /// newcomer's link.
    fn add_move(&mut self, o: u32, n: u32) {
        let inside = |x: u32, root: u32, side: usize| root <= x && x < self.next(side, root);
        let mut absorbed = Vec::new();
        for (k, &(mo, mn)) in self.moves.iter().enumerate() {
            let (holds_old, holds_new) = (inside(mo, o, 0), inside(mn, n, 1));
            let (held_old, held_new) = (inside(o, mo, 0), inside(n, mn, 1));
            if held_old && held_new {
                return;
            }
            if holds_old && holds_new {
                absorbed.push(k);
            } else if holds_old || holds_new || held_old || held_new {
                self.unlink(o);
                return;
            }
        }
        for k in absorbed.into_iter().rev() {
            self.moves.remove(k);
        }
        self.moves.push((o, n));
    }

    /// Phase 5: within each corresponding list pair, children keep their
    /// order; those outside the heaviest ordered subset are moves when
    /// they can be, else unlinked with everything inside them.
    fn order(&mut self) -> Result<(), Fallback> {
        let old = self.trees[0];
        for p in 0..old.nodes.len() as u32 {
            let q = self.partner[0][p as usize];
            if !old.is_list(p) || q == NONE || !self.is_list(1, q) {
                continue;
            }
            let children = old.children(p);
            self.spend(children.len())?;
            let mut keyed: Vec<(u32, u32, u64)> = Vec::new();
            for &c in &children {
                if self.moves.iter().any(|&(o, _)| o == c) {
                    continue;
                }
                if let Some(key) = self.first_partner(c) {
                    keyed.push((c, key, u64::from(self.leaves_in(0, c).max(1))));
                }
            }
            let kept = heaviest_increasing(&keyed);
            for (k, &(c, _, _)) in keyed.iter().enumerate() {
                if kept[k] {
                    continue;
                }
                let partner = self.partner[0][c as usize];
                if partner != NONE && self.movable(c, partner) {
                    self.add_move(c, partner);
                } else if partner != NONE {
                    self.unlink(c);
                } else {
                    self.unlink_inside(c);
                }
            }
        }
        Ok(())
    }

    fn first_partner(&self, node: u32) -> Option<u32> {
        let end = self.next(0, node);
        (node..end)
            .map(|k| self.partner[0][k as usize])
            .find(|&p| p != NONE)
    }

    fn unlink_inside(&mut self, node: u32) {
        let end = self.next(0, node);
        let mut k = node;
        while k < end {
            if self.moves.iter().any(|&(o, _)| o == k) {
                k = self.next(0, k);
                continue;
            }
            self.unlink(k);
            k += 1;
        }
    }

    /// Phase 6: tokens, list delimiters included, never cross. Checked
    /// outside moves and inside each move on its own.
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
        let mut groups: HashMap<u32, Vec<(u32, u32, u32)>> = HashMap::new();
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

/// Which entries of `items` (in order, keyed by position on the other
/// side) form the heaviest strictly increasing subsequence.
fn lone(tree: &Tree<'_>, list: u32) -> bool {
    let first = list as usize + 1;
    first < tree.nodes[list as usize].next as usize
        && tree.nodes[first].next == tree.nodes[list as usize].next
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

/// Prefix counts of leaf nodes: the leaves of a subtree are
/// `counts[next] - counts[start]`.
fn leaf_counts(tree: &Tree<'_>) -> Vec<u32> {
    let mut out = Vec::with_capacity(tree.nodes.len() + 1);
    let mut count = 0;
    out.push(0);
    for node in &tree.nodes {
        count += u32::from(node.class != Class::List);
        out.push(count);
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

fn named<'a>(tree: &Tree<'a>) -> HashMap<&'a str, Vec<u32>> {
    let mut out: HashMap<&'a str, Vec<u32>> = HashMap::new();
    for item in &tree.items {
        out.entry(tree.text(item.name.clone()))
            .or_default()
            .push(item.node);
    }
    out
}

fn is_item(tree: &Tree<'_>, node: u32) -> bool {
    tree.items
        .binary_search_by_key(&node, |item| item.node)
        .is_ok()
}

fn item_name<'a>(tree: &Tree<'a>, node: u32) -> Option<&'a str> {
    let at = tree
        .items
        .binary_search_by_key(&node, |item| item.node)
        .ok()?;
    Some(tree.text(tree.items[at].name.clone()))
}

fn dice(a: &[u64], b: &[u64]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let (mut i, mut j, mut common) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    2.0 * common as f32 / (a.len() + b.len()) as f32
}
