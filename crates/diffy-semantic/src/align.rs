use std::collections::HashMap;
use std::ops::Range;

use diffy_diff::{
    ChangeIntensity, InlineDetail, InlineOptions, SemanticSpan, myers, paired_inline_diff,
};

use crate::Row;
use crate::matcher::Matching;
use crate::syntax::{Class, NONE, Tree};

const CONTEXT: usize = 3;
const LEADING_WEIGHT: u64 = 4;

struct Lines {
    starts: Vec<u32>,
    len: u32,
}

impl Lines {
    fn new(text: &str) -> Self {
        let mut starts = Vec::new();
        if !text.is_empty() {
            starts.push(0);
        }
        for (i, byte) in text.bytes().enumerate() {
            if byte == b'\n' && i + 1 < text.len() {
                starts.push(i as u32 + 1);
            }
        }
        Self {
            starts,
            len: text.len() as u32,
        }
    }

    fn count(&self) -> u32 {
        self.starts.len() as u32
    }

    fn of(&self, byte: u32) -> u32 {
        (self.starts.partition_point(|&s| s <= byte) as u32).saturating_sub(1)
    }

    fn content(&self, text: &str, line: u32) -> Range<u32> {
        let start = self.starts[line as usize];
        let next = self
            .starts
            .get(line as usize + 1)
            .copied()
            .unwrap_or(self.len);
        let bytes = text.as_bytes();
        let mut end = next;
        if end > start && bytes[end as usize - 1] == b'\n' {
            end -= 1;
        }
        if end > start && bytes[end as usize - 1] == b'\r' {
            end -= 1;
        }
        start..end
    }

    fn leads(&self, text: &str, byte: u32) -> bool {
        let start = self.starts[self.of(byte) as usize];
        text[start as usize..byte as usize]
            .bytes()
            .all(|b| b.is_ascii_whitespace())
    }

    fn text<'a>(&self, text: &'a str, line: u32) -> &'a str {
        let range = self.content(text, line);
        &text[range.start as usize..range.end as usize]
    }
}

struct Side<'t, 'a> {
    tree: &'t Tree<'a>,
    lines: Lines,
    spans: Vec<Vec<SemanticSpan>>,
}

impl Side<'_, '_> {
    fn mark(&mut self, range: Range<u32>, intensity: ChangeIntensity) {
        if range.is_empty() {
            return;
        }
        let first = self.lines.of(range.start);
        let last = self.lines.of(range.end - 1);
        for line in first..=last {
            let content = self.lines.content(self.tree.src, line);
            let start = range.start.max(content.start);
            let end = range.end.min(content.end);
            if start < end {
                self.spans[line as usize].push(SemanticSpan {
                    start: start - content.start,
                    end: end - content.start,
                    intensity,
                });
            }
        }
    }
}

pub(crate) fn rows(old: &Tree<'_>, new: &Tree<'_>, matching: &Matching) -> Vec<Vec<Row>> {
    let mut sides = [old, new].map(|tree| {
        let lines = Lines::new(tree.src);
        let spans = vec![Vec::new(); lines.count() as usize];
        Side { tree, lines, spans }
    });
    for (s, side) in sides.iter_mut().enumerate() {
        novel(side, &matching.partner[s]);
    }
    for &(x, y) in &matching.replaced {
        replaced(&mut sides, x, y);
    }
    for side in &mut sides {
        for spans in &mut side.spans {
            spans.sort_by_key(|span| span.start);
        }
    }
    let anchors = anchors(&sides, matching);
    let aligned = align(&sides, &anchors);
    chunks(&mut sides, aligned)
}

fn novel(side: &mut Side<'_, '_>, partner: &[u32]) {
    for (index, node) in side.tree.nodes.iter().enumerate() {
        if partner[index] != NONE {
            continue;
        }
        if node.class == Class::List {
            side.mark(node.open.clone(), ChangeIntensity::Novel);
            side.mark(node.close.clone(), ChangeIntensity::Novel);
        } else {
            side.mark(node.span.clone(), ChangeIntensity::Novel);
        }
    }
}

fn replaced(sides: &mut [Side<'_, '_>; 2], x: u32, y: u32) {
    let (ra, rb) = (
        sides[0].tree.nodes[x as usize].span.clone(),
        sides[1].tree.nodes[y as usize].span.clone(),
    );
    let (ta, tb) = (
        sides[0].tree.text(ra.clone()),
        sides[1].tree.text(rb.clone()),
    );
    let words = paired_inline_diff(ta, tb, &InlineOptions::default());
    if words.detail == InlineDetail::Exact && (!words.old.is_empty() || !words.new.is_empty()) {
        for (side, base, ranges) in [(0, ra.start, &words.old), (1, rb.start, &words.new)] {
            for range in ranges {
                sides[side].mark(
                    base + range.start..base + range.end,
                    ChangeIntensity::NovelWord,
                );
            }
        }
    } else if ta != tb {
        sides[0].mark(ra, ChangeIntensity::Novel);
        sides[1].mark(rb, ChangeIntensity::Novel);
    }
}

fn anchors(sides: &[Side<'_, '_>; 2], matching: &Matching) -> Vec<(u32, u32, u64)> {
    let (old, new) = (&sides[0], &sides[1]);
    let mut out = Vec::new();
    let mut push = |a: &Range<u32>, b: &Range<u32>, weight: u64| {
        if a.is_empty() || b.is_empty() {
            return;
        }
        let (la, lb) = (old.lines.of(a.start), new.lines.of(b.start));
        let (ea, eb) = (old.lines.of(a.end - 1), new.lines.of(b.end - 1));
        if ea - la == eb - lb {
            for k in 0..=ea - la {
                out.push((la + k, lb + k, weight));
            }
        } else {
            out.push((la, lb, weight));
        }
    };
    for (index, node) in old.tree.nodes.iter().enumerate() {
        let partner = matching.partner[0][index];
        if partner == NONE {
            continue;
        }
        let other = &new.tree.nodes[partner as usize];
        if node.class == Class::List {
            push(&node.open, &other.open, 1);
            push(&node.close, &other.close, 1);
        } else {
            let mut weight = solid(old.tree.text(node.span.clone())).max(1) as u64;
            if old.lines.leads(old.tree.src, node.span.start)
                && new.lines.leads(new.tree.src, other.span.start)
            {
                weight *= LEADING_WEIGHT;
            }
            push(&node.span, &other.span, weight);
        }
    }
    out.sort_unstable();
    out.dedup_by(|next, kept| {
        let same = next.0 == kept.0 && next.1 == kept.1;
        if same {
            kept.2 += next.2;
        }
        same
    });
    heaviest_chain(out, new.lines.count())
}

fn heaviest_chain(mut candidates: Vec<(u32, u32, u64)>, new_lines: u32) -> Vec<(u32, u32, u64)> {
    candidates.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let size = new_lines as usize + 1;
    let mut tree = vec![(0u64, usize::MAX); size + 1];
    let mut previous = vec![usize::MAX; candidates.len()];
    let mut best = (0u64, usize::MAX);
    for (index, &(_, line, weight)) in candidates.iter().enumerate() {
        let mut found = (0u64, usize::MAX);
        let mut i = line as usize;
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
        let mut i = line as usize + 1;
        while i <= size {
            if total.0 > tree[i].0 {
                tree[i] = total;
            }
            i += i & i.wrapping_neg();
        }
    }
    let mut chain = Vec::new();
    let mut at = best.1;
    while at != usize::MAX {
        chain.push(candidates[at]);
        at = previous[at];
    }
    chain.reverse();
    chain
}

fn align(
    sides: &[Side<'_, '_>; 2],
    anchors: &[(u32, u32, u64)],
) -> Vec<(Option<u32>, Option<u32>)> {
    let mut rows = Vec::new();
    let (mut o, mut n) = (0, 0);
    for &(ao, an, _) in anchors {
        fill(sides, o..ao, n..an, &mut rows);
        rows.push((Some(ao), Some(an)));
        (o, n) = (ao + 1, an + 1);
    }
    fill(
        sides,
        o..sides[0].lines.count(),
        n..sides[1].lines.count(),
        &mut rows,
    );
    rows
}

fn fill(
    sides: &[Side<'_, '_>; 2],
    olds: Range<u32>,
    news: Range<u32>,
    rows: &mut Vec<(Option<u32>, Option<u32>)>,
) {
    if olds.is_empty() || news.is_empty() {
        rows.extend(olds.map(|o| (Some(o), None)));
        rows.extend(news.map(|n| (None, Some(n))));
        return;
    }
    let mut ids = HashMap::new();
    let a = intern(&mut ids, &sides[0], olds.clone());
    let b = intern(&mut ids, &sides[1], news.clone());
    let (mut i, mut j) = (0, 0);
    let run = |rows: &mut Vec<_>, old: Range<u32>, new: Range<u32>| {
        let paired = old.len().min(new.len()) as u32;
        for k in 0..paired {
            rows.push((Some(old.start + k), Some(new.start + k)));
        }
        rows.extend((old.start + paired..old.end).map(|o| (Some(o), None)));
        rows.extend((new.start + paired..new.end).map(|n| (None, Some(n))));
    };
    for change in myers::diff(&a, &b) {
        while i < change.old.start {
            rows.push((Some(olds.start + i), Some(news.start + j)));
            i += 1;
            j += 1;
        }
        run(
            rows,
            olds.start + change.old.start..olds.start + change.old.end,
            news.start + change.new.start..news.start + change.new.end,
        );
        (i, j) = (change.old.end, change.new.end);
    }
    while i < a.len() as u32 {
        rows.push((Some(olds.start + i), Some(news.start + j)));
        i += 1;
        j += 1;
    }
}

fn intern<'a>(ids: &mut HashMap<&'a str, u32>, side: &Side<'_, 'a>, lines: Range<u32>) -> Vec<u32> {
    lines
        .map(|line| {
            let next = ids.len() as u32;
            *ids.entry(side.lines.text(side.tree.src, line))
                .or_insert(next)
        })
        .collect()
}

fn chunks(
    sides: &mut [Side<'_, '_>; 2],
    aligned: Vec<(Option<u32>, Option<u32>)>,
) -> Vec<Vec<Row>> {
    let changed: Vec<bool> = aligned
        .iter()
        .map(|&(o, n)| match (o, n) {
            (Some(o), Some(n)) => {
                !sides[0].spans[o as usize].is_empty()
                    || !sides[1].spans[n as usize].is_empty()
                    || sides[0].lines.text(sides[0].tree.src, o)
                        != sides[1].lines.text(sides[1].tree.src, n)
            }
            _ => true,
        })
        .collect();
    let mut keep = vec![false; aligned.len()];
    for (index, _) in changed.iter().enumerate().filter(|(_, c)| **c) {
        let start = index.saturating_sub(CONTEXT);
        let end = (index + CONTEXT + 1).min(aligned.len());
        keep[start..end].fill(true);
    }
    let mut out: Vec<Vec<Row>> = Vec::new();
    let mut open = false;
    for (index, &(old, new)) in aligned.iter().enumerate() {
        if !keep[index] {
            open = false;
            continue;
        }
        if !open {
            out.push(Vec::new());
            open = true;
        }
        let take = |side: &mut Side<'_, '_>, line: Option<u32>| {
            line.map(|l| std::mem::take(&mut side.spans[l as usize]))
                .unwrap_or_default()
        };
        let spans = [take(&mut sides[0], old), take(&mut sides[1], new)];
        if let Some(chunk) = out.last_mut() {
            chunk.push(Row { old, new, spans });
        }
    }
    out
}

fn solid(text: &str) -> usize {
    text.bytes().filter(|b| !b.is_ascii_whitespace()).count()
}
