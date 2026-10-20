use super::*;

impl<'a> Matcher<'_, 'a> {
    /// Pairs subtrees left unlinked whose correspondence pays for a move
    /// note: anchors that landed outside their container, equal subtrees
    /// and same-named items, best first.
    pub(super) fn relocate(&mut self) -> Result<(), Fallback> {
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

    pub(super) fn siblings(&self, side: usize, from: u32, to: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut at = from;
        while at < to {
            out.push(at);
            at = self.next(side, at);
        }
        out
    }

    pub(super) fn moved(&mut self, x: u32, y: u32) -> Result<Entry, Fallback> {
        let mut e = self.savings(x, y)?;
        let unmatched = self.total(0, x) + self.total(1, y) - e.d[UNMATCHED];
        e.d[MOVED] -= unmatched;
        Ok(e)
    }

    pub(super) fn disjoint(&self, x: u32, y: u32) -> bool {
        let inside = |n: u32, root: u32, side: usize| root <= n && n < self.next(side, root);
        self.moves.iter().all(|&(mo, mn)| {
            !(inside(mo, x, 0) || inside(x, mo, 0) || inside(mn, y, 1) || inside(y, mn, 1))
        })
    }

    pub(super) fn open_chain(&self, side: usize, node: u32) -> Vec<u32> {
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

    pub(super) fn duplicates(&mut self) -> Result<Vec<(u32, u32)>, Fallback> {
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
}
