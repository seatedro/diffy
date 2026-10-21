use super::*;

impl<'a> Matcher<'_, 'a> {
    pub(super) fn select(&mut self, cells: &mut [Cell]) -> Result<Vec<usize>, Fallback> {
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

    fn slots(&self, xs: &[u32], ys: &[u32], a: u32, b: u32, fixed: &mut Vec<Anchor>) {
        let (old, new) = (self.trees[0], self.trees[1]);
        if self.parents[0][xs[0] as usize] != a || self.parents[1][ys[0] as usize] != b {
            return;
        }
        let kind = |tree: &Tree<'_>, n: u32| tree.nodes[n as usize].kind;
        let bracketed = |side: usize, n: u32| self.is_list(side, n) && self.bracketed(side, n);
        let lists = |side: usize, nodes: &[u32]| -> Vec<usize> {
            (0..nodes.len())
                .filter(|&k| bracketed(side, nodes[k]))
                .collect()
        };
        let (la, lb) = (lists(0, xs), lists(1, ys));
        let rest = |side: usize, nodes: &[u32]| -> Vec<u64> {
            nodes
                .iter()
                .filter(|&&n| !bracketed(side, n))
                .map(|&n| self.hash(side, n))
                .collect()
        };
        let paired = self.anchors.binary_search(&(a, b)).is_ok();
        if !paired && rest(0, xs) != rest(1, ys) {
            return;
        }
        for &i in &la {
            let same = |k: &&usize| kind(old, xs[**k]) == kind(old, xs[i]);
            if la.iter().filter(same).count() != 1 {
                continue;
            }
            let mut matches = lb.iter().filter(|&&j| kind(new, ys[j]) == kind(old, xs[i]));
            let (Some(&j), None) = (matches.next(), matches.next()) else {
                continue;
            };
            let taken = fixed.iter().any(|f| f.old == i || f.new == j);
            if taken || !self.feasible(xs[i], ys[j]) || !fits(fixed, (i, i), (j, j)) {
                continue;
            }
            let at = fixed.partition_point(|f| f.old < i);
            fixed.insert(
                at,
                Anchor {
                    old: i,
                    new: j,
                    x: xs[i],
                    y: ys[j],
                    deep: false,
                },
            );
        }
    }

    pub(super) fn cells(
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
        let mut fixed = self.anchored(xs, ys)?;
        self.slots(xs, ys, a, b, &mut fixed);
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

    pub(super) fn anchored(&mut self, xs: &[u32], ys: &[u32]) -> Result<Vec<Anchor>, Fallback> {
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

    pub(super) fn outermost(
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

    pub(super) fn candidate_leaf(&self, x: u32, y: u32) -> bool {
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
    pub(super) fn wraps(
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

    pub(super) fn reach(
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

    pub(super) fn estimate(&mut self, a: u32, b: u32) -> Result<i64, Fallback> {
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

    pub(super) fn signature(&mut self, side: usize, node: u32) -> Result<Vec<u64>, Fallback> {
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

    pub(super) fn intern(&self, a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
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

    pub(super) fn leads(&self, side: usize, node: u32) -> bool {
        let tree = self.trees[side];
        let start = tree.nodes[node as usize].span.start as usize;
        tree.src.as_bytes()[..start]
            .iter()
            .rev()
            .take_while(|&&c| c != b'\n')
            .all(u8::is_ascii_whitespace)
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
