use super::*;

impl<'a> Matcher<'_, 'a> {
    pub(super) fn solve(&mut self) -> Result<(), Fallback> {
        let root = self.savings(0, 0)?;
        self.predicted = root.d;
        self.apply(0, 0)?;
        self.relocate()
    }

    pub(super) fn apply(&mut self, a: u32, b: u32) -> Result<(), Fallback> {
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

    pub(super) fn apply_acts(
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

    pub(super) fn margin(&mut self, cells: &[Cell], chosen: &[usize]) {
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
}
