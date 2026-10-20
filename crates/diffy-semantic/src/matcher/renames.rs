use super::*;

impl<'a> Matcher<'_, 'a> {
    /// Accepts each rename whose linked uses pay for its entry, and
    /// charges the rejected ones back to the prediction.
    pub(super) fn finalize(&mut self) -> (Vec<(&'a str, &'a str, u32)>, bool) {
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

    pub(super) fn vetoed(
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
}
