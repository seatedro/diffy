use super::*;

impl<'a> Matcher<'_, 'a> {
    pub(super) fn respects(&self, x: u32, y: u32) -> bool {
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

    pub(super) fn identities(&mut self) -> Result<(), Fallback> {
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

    pub(super) fn unit(&self, side: usize, node: u32) -> (u32, u32) {
        let items = &self.trees[side].items;
        match items.binary_search_by_key(&node, |i| i.node) {
            Ok(at) => (items[at].lead, items[at].unit),
            Err(_) => (node, node),
        }
    }
}
