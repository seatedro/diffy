use super::*;

impl<'a> Matcher<'_, 'a> {
    pub(super) fn rename_ok(&self, p: &'a str, q: &'a str) -> bool {
        p != q
            && !self.plain[1].contains(p)
            && !self.plain[0].contains(q)
            && self.allowed.as_ref().is_none_or(|r| r.contains(&(p, q)))
    }

    pub(super) fn identity(&self, a: u32, b: u32) -> Option<bool> {
        let (ta, tb) = (self.trees[0], self.trees[1]);
        let (p, q) = (ta.text(item_at(ta, a)?), tb.text(item_at(tb, b)?));
        (p != q).then(|| self.rename_ok(p, q))
    }

    pub(super) fn leaf(
        &mut self,
        a: u32,
        b: u32,
        ctx: Ctx,
        exact: bool,
    ) -> Result<Option<Entry>, Fallback> {
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

    pub(super) fn inexact(&self, a: u32, b: u32) -> bool {
        let class = self.trees[0].nodes[a as usize].class;
        matches!(class, Class::Text | Class::Comment)
            && !cost::same_text(class, self.text(0, a), self.text(1, b))
    }

    pub(super) fn guess(&mut self, a: u32, b: u32) -> Result<Option<(i64, i64)>, Fallback> {
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

    pub(super) fn edit(&mut self, a: u32, b: u32) -> Result<Option<(i64, i64)>, Fallback> {
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

    pub(super) fn direct(&mut self, side: usize, list: u32) -> Result<i64, Fallback> {
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

    pub(super) fn savings(&mut self, a: u32, b: u32) -> Result<Entry, Fallback> {
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

    pub(super) fn delimiter_splits(&self, a: u32, b: u32) -> i64 {
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

    pub(super) fn splits(&mut self, a: u32, b: u32) -> Result<i64, Fallback> {
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

    pub(super) fn features(&mut self, cell: &Cell) -> Result<Entry, Fallback> {
        Ok(match cell.act {
            Act::Leaf(x, y, ctx) => self.leaf(x, y, ctx, cell.exact)?.unwrap_or_default(),
            Act::List(x, y) => self.savings(x, y)?,
            Act::Wrap(..) | Act::Unwrap(..) => self.wrapped[&self.wrap_key(cell)],
        })
    }

    pub(super) fn wrap_key(&self, cell: &Cell) -> (u32, u32, u32, bool) {
        match cell.act {
            Act::Wrap(_, _, host, ends) => (ends.0, ends.1, host, true),
            Act::Unwrap(_, _, host, ends) => (ends.0, ends.1, host, false),
            _ => (NONE, NONE, NONE, false),
        }
    }
}
