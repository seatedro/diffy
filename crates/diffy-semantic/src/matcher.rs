use std::collections::HashMap;
use std::ops::Range;
use std::time::Instant;

use diffy_diff::myers;

use crate::Fallback;
use crate::syntax::{Class, NONE, Tree};

const MAX_PAIR_CELLS: usize = 1 << 16;
const SIGNATURE_LEAVES: usize = 64;
const MIN_SIMILARITY: f32 = 0.3;
const RENAMED_ITEM_SIMILARITY: f32 = 0.6;
const TEXT_SIMILARITY: f32 = 0.5;
const SIMILAR_DEPTH: usize = 2;
const CLOCK_STRIDE: u64 = 1 << 16;
const WIDEN_SIMILARITY: f32 = 0.5;
const MIN_MOVE_NODES: u32 = 12;
const MOVE_SIMILARITY: f32 = 0.8;
const MAX_MOVE_CANDIDATES: usize = 512;

pub(crate) struct Matching {
    pub(crate) partner: [Vec<u32>; 2],
    pub(crate) replaced: Vec<(u32, u32)>,
    pub(crate) renamed: Vec<(u32, u32)>,
    pub(crate) moved: Vec<(u32, u32)>,
}

pub(crate) fn run(
    old: &Tree<'_>,
    new: &Tree<'_>,
    max_work: u64,
    deadline: Instant,
) -> Result<Matching, Fallback> {
    let mut matcher = Matcher {
        trees: [old, new],
        partner: [vec![NONE; old.nodes.len()], vec![NONE; new.nodes.len()]],
        replaced: Vec::new(),
        renamed: Vec::new(),
        moved: Vec::new(),
        work: 0,
        max_work,
        clock_at: CLOCK_STRIDE,
        deadline,
    };
    matcher.pair_lists(0, 0)?;
    matcher.detect_moves()?;
    Ok(Matching {
        partner: matcher.partner,
        replaced: matcher.replaced,
        renamed: matcher.renamed,
        moved: matcher.moved,
    })
}

struct Splice {
    item: usize,
    list: u32,
    score: u32,
    wanted: Range<usize>,
}

struct Matcher<'t, 'a> {
    trees: [&'t Tree<'a>; 2],
    partner: [Vec<u32>; 2],
    replaced: Vec<(u32, u32)>,
    renamed: Vec<(u32, u32)>,
    moved: Vec<(u32, u32)>,
    work: u64,
    max_work: u64,
    clock_at: u64,
    deadline: Instant,
}

impl Matcher<'_, '_> {
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

    fn hash(&self, side: usize, node: u32) -> u64 {
        self.trees[side].nodes[node as usize].hash
    }

    fn link(&mut self, a: u32, b: u32) {
        self.partner[0][a as usize] = b;
        self.partner[1][b as usize] = a;
    }

    fn same(&mut self, a: u32, b: u32) -> bool {
        let (na, nb) = (
            &self.trees[0].nodes[a as usize],
            &self.trees[1].nodes[b as usize],
        );
        let len = na.next - a;
        if na.hash != nb.hash || nb.next - b != len {
            return false;
        }
        for k in 0..len {
            self.link(a + k, b + k);
        }
        true
    }

    fn pair_lists(&mut self, a: u32, b: u32) -> Result<(), Fallback> {
        self.link(a, b);
        let (left, right) = (self.trees[0].children(a), self.trees[1].children(b));
        self.seq(&left, &right)
    }

    fn pairable_lists(&self, a: u32, b: u32) -> bool {
        let (ta, tb) = (self.trees[0], self.trees[1]);
        let (na, nb) = (&ta.nodes[a as usize], &tb.nodes[b as usize]);
        na.class == Class::List
            && nb.class == Class::List
            && na.kind == nb.kind
            && ta.text(na.open.clone()) == tb.text(nb.open.clone())
            && ta.text(na.close.clone()) == tb.text(nb.close.clone())
    }

    fn seq(&mut self, a: &[u32], b: &[u32]) -> Result<(), Fallback> {
        self.spend(a.len() + b.len())?;
        let mut start = 0;
        while start < a.len()
            && start < b.len()
            && self.hash(0, a[start]) == self.hash(1, b[start])
            && self.same(a[start], b[start])
        {
            start += 1;
        }
        let (a, b) = (&a[start..], &b[start..]);
        if a.is_empty() || b.is_empty() {
            return Ok(());
        }
        let (ia, ib) = self.intern(a, b);
        let (mut i, mut j) = (0, 0);
        for change in slide(myers::diff(&ia, &ib), &ia, &ib) {
            let (old, new) = (
                change.old.start as usize..change.old.end as usize,
                change.new.start as usize..change.new.end as usize,
            );
            while i < old.start {
                self.same(a[i], b[j]);
                i += 1;
                j += 1;
            }
            self.gap(&a[old.clone()], &b[new.clone()])?;
            (i, j) = (old.end, new.end);
        }
        while i < a.len() {
            self.same(a[i], b[j]);
            i += 1;
            j += 1;
        }
        Ok(())
    }

    fn intern(&self, a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
        let mut ids: HashMap<u64, u32> = HashMap::with_capacity(a.len() + b.len());
        let mut id = |hash: u64| {
            let next = ids.len() as u32;
            *ids.entry(hash).or_insert(next)
        };
        let ia = a.iter().map(|&n| id(self.hash(0, n))).collect();
        let ib = b.iter().map(|&n| id(self.hash(1, n))).collect();
        (ia, ib)
    }

    fn gap(&mut self, a: &[u32], b: &[u32]) -> Result<(), Fallback> {
        if a.is_empty() || b.is_empty() {
            return Ok(());
        }
        if a.len() == 1 && b.len() == 1 && self.pairable_lists(a[0], b[0]) {
            return self.pair_lists(a[0], b[0]);
        }
        if a.len() == 1 && b.len() == 1 && self.identifier(0, a[0]) && self.identifier(1, b[0]) {
            self.link(a[0], b[0]);
            self.renamed.push((a[0], b[0]));
            return Ok(());
        }
        if self.splice(a, b, false)? || self.pair_similar(a, b)? || self.splice(a, b, true)? {
            return Ok(());
        }
        if self.identifier(0, a[0]) && self.identifier(1, b[0]) {
            self.link(a[0], b[0]);
            self.renamed.push((a[0], b[0]));
            return self.gap(&a[1..], &b[1..]);
        }
        if a.len() == 1
            && b.len() == 1
            && self.trees[0].is_list(a[0])
            && self.trees[1].is_list(b[0])
        {
            let (left, right) = (self.trees[0].children(a[0]), self.trees[1].children(b[0]));
            return self.seq(&left, &right);
        }
        Ok(())
    }

    fn splice(&mut self, a: &[u32], b: &[u32], similar: bool) -> Result<bool, Fallback> {
        let into_new = self.wrapper(1, b, 0, a, similar)?;
        let into_old = self.wrapper(0, a, 1, b, similar)?;
        let new_wins = match (&into_new, &into_old) {
            (Some(n), Some(o)) => n.score >= o.score,
            (Some(_), None) => true,
            _ => false,
        };
        if let Some(mut s) = into_new.filter(|_| new_wins) {
            let children = self.trees[1].children(s.list);
            s.wanted = self.widen(0, a, s.wanted, 1, &children);
            self.gap(&a[..s.wanted.start], &b[..s.item])?;
            self.seq(&a[s.wanted.clone()], &children)?;
            self.gap(&a[s.wanted.end..], &b[s.item + 1..])?;
            return Ok(true);
        }
        if let Some(mut s) = into_old {
            let children = self.trees[0].children(s.list);
            s.wanted = self.widen(1, b, s.wanted, 0, &children);
            self.gap(&a[..s.item], &b[..s.wanted.start])?;
            self.seq(&children, &b[s.wanted.clone()])?;
            self.gap(&a[s.item + 1..], &b[s.wanted.end..])?;
            return Ok(true);
        }
        Ok(false)
    }

    fn widen(
        &self,
        side: usize,
        items: &[u32],
        range: Range<usize>,
        other: usize,
        children: &[u32],
    ) -> Range<usize> {
        let taken: Vec<u64> = items.iter().map(|&n| self.hash(side, n)).collect();
        let signatures: Vec<(u32, Vec<u64>)> = children
            .iter()
            .filter(|&&c| {
                self.trees[other].is_list(c)
                    && self.partner[other][c as usize] == NONE
                    && !taken.contains(&self.hash(other, c))
            })
            .map(|&c| (c, self.signature(other, c)))
            .collect();
        let fits = |item: u32| {
            let own = self.signature(side, item);
            signatures.iter().any(|(c, signature)| {
                let pairable = if side == 0 {
                    self.pairable_lists(item, *c)
                } else {
                    self.pairable_lists(*c, item)
                };
                pairable && dice(&own, signature) >= WIDEN_SIMILARITY
            })
        };
        let (mut start, mut end) = (range.start, range.end);
        while start > 0 && fits(items[start - 1]) {
            start -= 1;
        }
        while end < items.len() && fits(items[end]) {
            end += 1;
        }
        start..end
    }

    fn wrapper(
        &mut self,
        side: usize,
        items: &[u32],
        other: usize,
        wanted: &[u32],
        similar: bool,
    ) -> Result<Option<Splice>, Fallback> {
        let mut exact: HashMap<u64, usize> = HashMap::new();
        for (index, &n) in wanted.iter().enumerate() {
            if self.significant(other, n) {
                exact.entry(self.hash(other, n)).or_insert(index);
            }
        }
        if exact.is_empty() {
            return Ok(None);
        }
        let candidates: Vec<(usize, u32, Vec<u64>)> = wanted
            .iter()
            .enumerate()
            .filter(|&(_, &n)| similar && self.trees[other].is_list(n))
            .map(|(index, &n)| (index, n, self.signature(other, n)))
            .collect();
        let tree = self.trees[side];
        let mut best: Option<Splice> = None;
        for (position, &item) in items.iter().enumerate() {
            if !tree.is_list(item) {
                continue;
            }
            let end = tree.nodes[item as usize].next;
            self.spend((end - item) as usize)?;
            let mut lists: HashMap<u32, (u32, Range<usize>)> = HashMap::new();
            let mut parents = vec![item];
            let mut at = item + 1;
            while at < end {
                while parents
                    .last()
                    .is_some_and(|&p| at >= tree.nodes[p as usize].next)
                {
                    parents.pop();
                }
                let node = &tree.nodes[at as usize];
                let parent = parents.last().copied().unwrap_or(item);
                let hit = match exact.get(&node.hash) {
                    Some(&index) => Some((node.next - at, index)),
                    None if node.class == Class::List && parents.len() <= SIMILAR_DEPTH => {
                        self.similar_hit(side, at, &candidates)
                    }
                    None => None,
                };
                if let Some((score, index)) = hit {
                    let entry = lists.entry(parent).or_insert((0, index..index + 1));
                    entry.0 += score;
                    entry.1 = entry.1.start.min(index)..entry.1.end.max(index + 1);
                    at = node.next;
                    continue;
                }
                if node.class == Class::List {
                    parents.push(at);
                }
                at += 1;
            }
            for (list, (score, range)) in lists {
                if best.as_ref().is_none_or(|b| score > b.score) {
                    best = Some(Splice {
                        item: position,
                        list,
                        score,
                        wanted: range,
                    });
                }
            }
        }
        Ok(best)
    }

    fn similar_hit(
        &self,
        side: usize,
        node: u32,
        candidates: &[(usize, u32, Vec<u64>)],
    ) -> Option<(u32, usize)> {
        let size = self.trees[side].nodes[node as usize].next - node;
        let mut signature = None;
        let mut best: Option<(f32, usize)> = None;
        for (index, candidate, candidate_signature) in candidates {
            let pairable = if side == 0 {
                self.pairable_lists(node, *candidate)
            } else {
                self.pairable_lists(*candidate, node)
            };
            if !pairable {
                continue;
            }
            let own = signature.get_or_insert_with(|| self.signature(side, node));
            let similarity = dice(own, candidate_signature);
            if similarity >= MIN_SIMILARITY && best.is_none_or(|(b, _)| similarity > b) {
                best = Some((similarity, *index));
            }
        }
        best.map(|(similarity, index)| (((similarity * size as f32) as u32).max(1), index))
    }

    fn detect_moves(&mut self) -> Result<(), Fallback> {
        let old = self.unmatched_blocks(0)?;
        let mut new = self.unmatched_blocks(1)?;
        let mut by_hash: HashMap<u64, Vec<u32>> = HashMap::new();
        for &o in &old {
            by_hash.entry(self.hash(0, o)).or_default().push(o);
        }
        let mut taken = vec![false; self.trees[0].nodes.len()];
        new.retain(|&n| {
            let Some(o) = by_hash.get_mut(&self.hash(1, n)).and_then(Vec::pop) else {
                return true;
            };
            if self.same(o, n) {
                taken[o as usize] = true;
                self.moved.push((o, n));
                return false;
            }
            true
        });
        let old: Vec<(u32, Vec<u64>)> = old
            .into_iter()
            .filter(|&o| !taken[o as usize])
            .map(|o| (o, self.signature(0, o)))
            .collect();
        let mut used = vec![false; old.len()];
        for n in new {
            self.spend(old.len())?;
            let own = self.signature(1, n);
            let best = old
                .iter()
                .enumerate()
                .filter(|&(i, (o, _))| !used[i] && self.pairable_lists(*o, n))
                .map(|(i, (o, signature))| (i, *o, dice(&own, signature)))
                .filter(|&(_, _, similarity)| similarity >= MOVE_SIMILARITY)
                .max_by(|a, b| a.2.total_cmp(&b.2));
            if let Some((i, o, _)) = best {
                used[i] = true;
                self.pair_lists(o, n)?;
                self.moved.push((o, n));
            }
        }
        Ok(())
    }

    fn unmatched_blocks(&mut self, side: usize) -> Result<Vec<u32>, Fallback> {
        let tree = self.trees[side];
        let mut out = Vec::new();
        let mut at = 1;
        while at < tree.nodes.len() as u32 && out.len() < MAX_MOVE_CANDIDATES {
            let node = &tree.nodes[at as usize];
            let size = node.next - at;
            if node.class == Class::List && size >= MIN_MOVE_NODES {
                self.spend(size as usize)?;
                if (at..node.next).all(|k| self.partner[side][k as usize] == NONE) {
                    out.push(at);
                    at = node.next;
                    continue;
                }
            }
            at += 1;
        }
        Ok(out)
    }

    fn identifier(&self, side: usize, node: u32) -> bool {
        let tree = self.trees[side];
        let n = &tree.nodes[node as usize];
        let text = tree.text(n.span.clone());
        n.class == Class::Plain
            && text
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
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

    fn pair_similar(&mut self, a: &[u32], b: &[u32]) -> Result<bool, Fallback> {
        let (n, m) = (a.len(), b.len());
        if n * m > MAX_PAIR_CELLS {
            return Ok(false);
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
        let mut pairs = Vec::new();
        let (mut i, mut j) = (n, m);
        while i > 0 && j > 0 {
            let s = score[(i - 1) * m + (j - 1)];
            if s > 0.0 && best[i * width + j] == best[(i - 1) * width + j - 1] + s {
                pairs.push((i - 1, j - 1));
                i -= 1;
                j -= 1;
            } else if best[i * width + j] == best[(i - 1) * width + j] {
                i -= 1;
            } else {
                j -= 1;
            }
        }
        if pairs.is_empty() {
            return Ok(false);
        }
        pairs.reverse();
        let (mut i, mut j) = (0, 0);
        for (pi, pj) in pairs.iter().copied().chain([(n, m)]) {
            if pi > i && pj > j {
                self.gap(&a[i..pi], &b[j..pj])?;
            }
            if pi < n && pj < m {
                let (x, y) = (a[pi], b[pj]);
                if self.trees[0].is_list(x) {
                    self.pair_lists(x, y)?;
                } else {
                    self.link(x, y);
                    self.replaced.push((x, y));
                }
            }
            (i, j) = (pi + 1, pj + 1);
        }
        Ok(true)
    }

    fn score(&self, x: u32, y: u32, sx: &[u64], sy: &[u64]) -> f32 {
        let (nx, ny) = (
            &self.trees[0].nodes[x as usize],
            &self.trees[1].nodes[y as usize],
        );
        match (nx.class, ny.class) {
            (Class::List, Class::List) if self.pairable_lists(x, y) => {
                if sx.is_empty() || sy.is_empty() {
                    return MIN_SIMILARITY;
                }
                let similarity = dice(sx, sy);
                let floor = match (self.item_name(0, x), self.item_name(1, y)) {
                    (Some(a), Some(b)) if a != b => RENAMED_ITEM_SIMILARITY,
                    _ => MIN_SIMILARITY,
                };
                if similarity >= floor { similarity } else { 0.0 }
            }
            (Class::Text, Class::Text) if nx.kind == ny.kind => TEXT_SIMILARITY,
            (Class::Comment, Class::Comment) => TEXT_SIMILARITY,
            _ => 0.0,
        }
    }

    fn item_name(&self, side: usize, node: u32) -> Option<&str> {
        let tree = self.trees[side];
        let at = tree
            .items
            .binary_search_by_key(&node, |item| item.node)
            .ok()?;
        Some(tree.text(tree.items[at].name.clone()))
    }

    fn signature(&self, side: usize, node: u32) -> Vec<u64> {
        let tree = self.trees[side];
        let end = tree.nodes[node as usize].next;
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
}

fn slide(mut changes: Vec<myers::Change>, a: &[u32], b: &[u32]) -> Vec<myers::Change> {
    for k in 0..changes.len() {
        let limit = changes
            .get(k + 1)
            .map_or((a.len() as u32, b.len() as u32), |next| {
                (next.old.start, next.new.start)
            });
        let c = &mut changes[k];
        if c.old.is_empty() {
            while c.new.end < limit.1 && b[c.new.start as usize] == b[c.new.end as usize] {
                c.new = c.new.start + 1..c.new.end + 1;
                c.old = c.old.start + 1..c.old.end + 1;
            }
        } else if c.new.is_empty() {
            while c.old.end < limit.0 && a[c.old.start as usize] == a[c.old.end as usize] {
                c.old = c.old.start + 1..c.old.end + 1;
                c.new = c.new.start + 1..c.new.end + 1;
            }
        }
    }
    changes
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
