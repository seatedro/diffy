use super::*;

impl<'a> Matcher<'_, 'a> {
    pub(super) fn scope(&mut self) -> Result<(), Fallback> {
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

    pub(super) fn monotone(&mut self) -> Result<(), Fallback> {
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
