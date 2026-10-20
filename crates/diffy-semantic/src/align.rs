use std::collections::{HashMap, HashSet};
use std::ops::Range;

use diffy_diff::{
    ChangeIntensity, InlineDetail, InlineOptions, ItemChange, Move, OutlineItem, Rename,
    SemanticSpan, Side as DiffSide, myers, paired_inline_diff,
};

use crate::Row;
use crate::matcher::Matching;
use crate::syntax::{Class, NONE, Tree};

const CONTEXT: usize = 3;
const MIN_RENAME_USES: u32 = 2;
const LEADING_WEIGHT: u64 = 4;
const MOVE_KEPT: f32 = 0.6;
const MIN_PIECE_NODES: u32 = 6;
const MAX_PAIR_CELLS: usize = 40_000;

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

pub(crate) fn rows(
    old: &Tree<'_>,
    new: &Tree<'_>,
    matching: &Matching,
) -> (Vec<Vec<Row>>, Vec<OutlineItem>, Vec<Rename>, Vec<Move>) {
    let mut sides = [old, new].map(|tree| {
        let lines = Lines::new(tree.src);
        let spans = vec![Vec::new(); lines.count() as usize];
        Side { tree, lines, spans }
    });
    for (s, side) in sides.iter_mut().enumerate() {
        novel(side, &matching.partner[s]);
    }
    let words: Vec<[Vec<Range<u32>>; 2]> = matching
        .replaced
        .iter()
        .filter_map(|&(x, y)| replaced(&mut sides, x, y))
        .collect();
    let renames = renames(&mut sides, matching);
    mark_words(&mut sides, words, &renames);
    for side in &mut sides {
        for (line, spans) in side.spans.iter_mut().enumerate() {
            spans.sort_by_key(|span| span.start);
            let content = side.lines.content(side.tree.src, line as u32);
            join_novel(
                &side.tree.src[content.start as usize..content.end as usize],
                spans,
            );
        }
    }
    let outline = outline(&sides, matching);
    let displaced = displaced(&sides, matching);
    let anchors = anchors(&sides, matching, &displaced);
    let moves = moves(&sides, matching, &anchors, &displaced);
    let links = links(&sides, matching);
    let aligned = align(&sides, &anchors, &links);
    mark_silent(&mut sides, &aligned, &moves, matching);
    (chunks(&mut sides, aligned), outline, renames, moves)
}

fn moves(
    sides: &[Side<'_, '_>; 2],
    matching: &Matching,
    chain: &[(u32, u32, u64)],
    displaced: &[(u32, u32)],
) -> Vec<Move> {
    let mut out = Vec::new();
    let lines = |side: usize, node: u32| {
        let span = sides[side].tree.nodes[node as usize].span.clone();
        let of = |at: u32| sides[side].lines.of(at);
        of(span.start)..of(span.end.saturating_sub(1).max(span.start)) + 1
    };
    let stranded = matching.relocated.iter().filter(|&&(o, n)| {
        let (old, new) = (lines(0, o), lines(1, n));
        let from = chain.partition_point(|&(a, _, _)| a < old.start);
        let before = from.checked_sub(1).map_or(0, |i| chain[i].1 + 1);
        let after = chain[from..]
            .iter()
            .find(|&&(a, _, _)| a >= old.end)
            .map_or(u32::MAX, |&(_, b, _)| b);
        let kept = chain[from..]
            .iter()
            .take_while(|&&(a, _, _)| a < old.end)
            .any(|&(_, b, _)| new.contains(&b));
        !kept && (new.start < before || new.end > after) && !in_place(sides, matching, o, n)
    });
    let mut pairs = Vec::new();
    for &(o, n) in displaced.iter().chain(stranded) {
        refine(sides, matching, o, n, true, &mut pairs);
    }
    for (o, n) in pairs {
        let (a, b) = (
            sides[0].tree.nodes[o as usize].span.clone(),
            sides[1].tree.nodes[n as usize].span.clone(),
        );
        let old_line = sides[0].lines.of(a.start);
        let new_line = sides[1].lines.of(b.start);
        let old_lines = sides[0].lines.of(a.end.saturating_sub(1).max(a.start)) + 1 - old_line;
        let lines = sides[1].lines.of(b.end.saturating_sub(1).max(b.start)) + 1 - new_line;
        out.push(Move {
            old_line,
            old_lines,
            new_line,
            lines,
            from: enclosing(sides[0].tree, o),
            into: enclosing(sides[1].tree, n),
            item: own_item(sides[0].tree, o),
        });
    }
    out.sort_by_key(|m| (m.new_line, std::cmp::Reverse(m.lines)));
    let mut kept: Vec<Move> = Vec::with_capacity(out.len());
    for m in out {
        let inside = kept.iter().any(|k| {
            (k.new_line..k.new_line + k.lines).contains(&m.new_line)
                && (k.old_line..k.old_line + k.old_lines).contains(&m.old_line)
        });
        if inside {
            continue;
        }
        match kept.last_mut() {
            Some(k)
                if blank_between(&sides[0], k.old_line + k.old_lines, m.old_line)
                    && blank_between(&sides[1], k.new_line + k.lines, m.new_line) =>
            {
                k.old_lines = m.old_line + m.old_lines - k.old_line;
                k.lines = m.new_line + m.lines - k.new_line;
            }
            _ => kept.push(m),
        }
    }
    kept.retain(|m| {
        let novel = (m.new_line..m.new_line + m.lines)
            .filter(|&line| mostly_novel(&sides[1], line))
            .count() as u32;
        (m.lines > 1 || m.old_lines > 1 || m.item.is_some()) && novel * 3 <= m.lines
    });
    kept
}

fn mostly_novel(side: &Side<'_, '_>, line: u32) -> bool {
    let text = side.lines.text(side.tree.src, line);
    let solid = |s: &str| s.bytes().filter(|b| !b.is_ascii_whitespace()).count();
    let novel: usize = side.spans[line as usize]
        .iter()
        .filter(|s| s.intensity == ChangeIntensity::Novel)
        .map(|s| solid(&text[s.start as usize..s.end as usize]))
        .sum();
    novel * 2 > solid(text)
}

fn refine(
    sides: &[Side<'_, '_>; 2],
    matching: &Matching,
    o: u32,
    n: u32,
    top: bool,
    out: &mut Vec<(u32, u32)>,
) {
    let (old, new) = (sides[0].tree, sides[1].tree);
    let (eo, en) = (old.nodes[o as usize].next, new.nodes[n as usize].next);
    let inside = (o..eo)
        .filter(|&k| (n..en).contains(&matching.partner[0][k as usize]))
        .count();
    if inside as f32 >= MOVE_KEPT * (eo - o).max(en - n) as f32 {
        if top || whole_line(&sides[0], o) && whole_line(&sides[1], n) {
            out.push((o, n));
        }
        return;
    }
    for child in old.children(o) {
        let partner = matching.partner[0][child as usize];
        let size = old.nodes[child as usize].next - child;
        if old.is_list(child)
            && size >= MIN_PIECE_NODES
            && (n..en).contains(&partner)
            && new.is_list(partner)
        {
            refine(sides, matching, child, partner, false, out);
        }
    }
}

/// Whether `node` fills its lines: nothing but space before it on its
/// first line, and nothing but space or a separator after it on its last.
fn whole_line(side: &Side<'_, '_>, node: u32) -> bool {
    let span = &side.tree.nodes[node as usize].span;
    let src = side.tree.src;
    let after = &src[span.end as usize..];
    let rest = &after[..after.find('\n').unwrap_or(after.len())];
    side.lines.leads(src, span.start) && rest.trim().trim_matches([';', ',']).is_empty()
}

fn blank_between(side: &Side<'_, '_>, from: u32, to: u32) -> bool {
    from <= to
        && (from..to).all(|line| {
            let content = side.lines.content(side.tree.src, line);
            side.tree.src[content.start as usize..content.end as usize]
                .trim()
                .is_empty()
        })
}

fn enclosing(tree: &Tree<'_>, node: u32) -> Option<std::sync::Arc<str>> {
    let end = tree.nodes[node as usize].next;
    let item = tree
        .items
        .iter()
        .rev()
        .find(|item| item.node < node && tree.nodes[item.node as usize].next >= end)?;
    Some(item_label(tree, item, true))
}

fn own_item(tree: &Tree<'_>, node: u32) -> Option<std::sync::Arc<str>> {
    let item = tree.items.iter().find(|item| item.node == node)?;
    Some(item_label(tree, item, false))
}

fn item_label(tree: &Tree<'_>, item: &crate::syntax::Item, call: bool) -> std::sync::Arc<str> {
    let name = tree.text(item.name.clone());
    let callable = matches!(item.kind, "fn" | "function" | "method" | "def");
    match (callable, call) {
        (true, true) => format!("{name}()"),
        _ => format!("{} {name}", item.kind),
    }
    .into()
}

fn parent(tree: &Tree<'_>, node: u32) -> Option<u32> {
    (0..node)
        .rev()
        .find(|&p| tree.nodes[p as usize].next > node && tree.is_list(p))
}

/// The detected moves that are out of order among their siblings. When
/// a move only crosses smaller siblings, those siblings are what moved.
fn displaced(sides: &[Side<'_, '_>; 2], matching: &Matching) -> Vec<(u32, u32)> {
    let (old, new) = (sides[0].tree, sides[1].tree);
    let size = |c: u32| {
        let span = &old.nodes[c as usize].span;
        let last = span.end.saturating_sub(1).max(span.start);
        sides[0].lines.of(last) + 1 - sides[0].lines.of(span.start)
    };
    let mut out = Vec::new();
    for &(o, n) in &matching.moved {
        let crossing = match (parent(old, o), parent(new, n)) {
            (Some(p), Some(q)) if matching.partner[0][p as usize] == q => {
                let (olds, news) = (old.children(p), new.children(q));
                let (Some(i), Some(j)) = (
                    olds.iter().position(|&c| c == o),
                    news.iter().position(|&c| c == n),
                ) else {
                    out.push((o, n));
                    continue;
                };
                olds.iter()
                    .enumerate()
                    .filter(|&(k, _)| k != i)
                    .filter_map(|(k, &c)| {
                        let partner = matching.partner[0][c as usize];
                        let at = news.iter().position(|&x| x == partner)?;
                        ((k < i) != (at < j)).then_some((c, partner))
                    })
                    .collect::<Vec<_>>()
            }
            _ => {
                out.push((o, n));
                continue;
            }
        };
        if crossing.iter().all(|&(c, _)| size(c) < size(o)) {
            out.extend(crossing);
        } else {
            out.push((o, n));
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn in_place(sides: &[Side<'_, '_>; 2], matching: &Matching, o: u32, n: u32) -> bool {
    let (old, new) = (sides[0].tree, sides[1].tree);
    let (Some(p), Some(q)) = (parent(old, o), parent(new, n)) else {
        return false;
    };
    if matching.partner[0][p as usize] != q {
        return false;
    }
    let (olds, news) = (old.children(p), new.children(q));
    let (Some(i), Some(j)) = (
        olds.iter().position(|&c| c == o),
        news.iter().position(|&c| c == n),
    ) else {
        return false;
    };
    let index = |c: u32| {
        news.iter()
            .position(|&x| x == matching.partner[0][c as usize])
            .map(|at| (c, at))
    };
    let size = |c: u32| {
        let span = &old.nodes[c as usize].span;
        let last = span.end.saturating_sub(1).max(span.start);
        sides[0].lines.of(last) + 1 - sides[0].lines.of(span.start)
    };
    let fits = |found: Option<(u32, usize)>, ok: &dyn Fn(usize) -> bool| {
        found.is_none_or(|(c, at)| ok(at) || size(c) < size(o))
    };
    let before = olds[..i].iter().rev().find_map(|&c| index(c));
    let after = olds[i + 1..].iter().find_map(|&c| index(c));
    fits(before, &|at| at < j) && fits(after, &|at| at > j)
}

fn plain_counts<'a>(tree: &Tree<'a>) -> HashMap<&'a str, u32> {
    let mut out: HashMap<&'a str, u32> = HashMap::new();
    for node in &tree.nodes {
        if node.class == Class::Plain {
            *out.entry(tree.text(node.span.clone())).or_default() += 1;
        }
    }
    out
}

fn renames(sides: &mut [Side<'_, '_>; 2], matching: &Matching) -> Vec<Rename> {
    let (old_count, new_count) = (plain_counts(sides[0].tree), plain_counts(sides[1].tree));
    let mut pairs: HashMap<(&str, &str), Vec<(u32, u32)>> = HashMap::new();
    for &(x, y) in &matching.renamed {
        let old = sides[0]
            .tree
            .text(sides[0].tree.nodes[x as usize].span.clone());
        let new = sides[1]
            .tree
            .text(sides[1].tree.nodes[y as usize].span.clone());
        pairs.entry((old, new)).or_default().push((x, y));
    }
    let mut targets: HashMap<&str, u32> = HashMap::new();
    for (old, _) in pairs.keys() {
        *targets.entry(old).or_default() += 1;
    }
    let mut out = Vec::new();
    let mut marks = Vec::new();
    for ((old, new), uses) in pairs {
        let uses_n = uses.len() as u32;
        let consistent = uses_n >= MIN_RENAME_USES
            && targets.get(old) == Some(&1)
            && !old_count.contains_key(new)
            && !new_count.contains_key(old);
        if consistent {
            out.push(Rename {
                old: old.into(),
                new: new.into(),
                uses: uses_n,
            });
        }
        let intensity = if consistent {
            ChangeIntensity::Renamed
        } else {
            ChangeIntensity::Novel
        };
        marks.extend(uses.into_iter().map(|pair| (pair, intensity)));
    }
    for ((x, y), intensity) in marks {
        let (a, b) = (
            sides[0].tree.nodes[x as usize].span.clone(),
            sides[1].tree.nodes[y as usize].span.clone(),
        );
        sides[0].mark(a, intensity);
        sides[1].mark(b, intensity);
    }
    out.sort_by(|a, b| b.uses.cmp(&a.uses).then(a.old.cmp(&b.old)));
    out
}

struct Owned {
    lines: Range<u32>,
    node: u32,
    kind: &'static str,
    name: Range<u32>,
    edits: u32,
    old_edits: u32,
    edited: bool,
    unmatched: bool,
    inside_unmatched: bool,
}

fn owned(
    side: &Side<'_, '_>,
    partner: &[u32],
    moved: &HashSet<u32>,
) -> (Vec<Owned>, Vec<Option<usize>>) {
    let tree = side.tree;
    let mut items: Vec<Owned> = tree
        .items
        .iter()
        .map(|item| {
            let span = &tree.nodes[item.node as usize].span;
            let last = span.end.saturating_sub(1).max(span.start);
            Owned {
                lines: side.lines.of(span.start)..side.lines.of(last) + 1,
                node: item.node,
                kind: item.kind,
                name: item.name.clone(),
                edits: 0,
                old_edits: 0,
                edited: false,
                unmatched: partner[item.node as usize] == NONE,
                inside_unmatched: false,
            }
        })
        .collect();
    let mut open: Vec<(u32, bool)> = Vec::new();
    for item in &mut items {
        while open.last().is_some_and(|&(end, _)| item.node >= end) {
            open.pop();
        }
        item.inside_unmatched = open.iter().any(|&(_, folded)| folded);
        let folded = item.unmatched || moved.contains(&item.node);
        open.push((tree.nodes[item.node as usize].next, folded));
    }
    let mut owner = vec![None; side.lines.count() as usize];
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(items[i].lines.len()));
    for i in order {
        for line in items[i].lines.clone() {
            if let Some(slot) = owner.get_mut(line as usize) {
                *slot = Some(i);
            }
        }
    }
    (items, owner)
}

fn outline(sides: &[Side<'_, '_>; 2], matching: &Matching) -> Vec<OutlineItem> {
    let moved_old: HashSet<u32> = matching.moved.iter().map(|&(o, _)| o).collect();
    let moved_new: HashSet<u32> = matching.moved.iter().map(|&(_, n)| n).collect();
    let (mut old_items, old_owner) = owned(&sides[0], &matching.partner[0], &moved_old);
    let (mut new_items, new_owner) = owned(&sides[1], &matching.partner[1], &moved_new);
    let marked = |side: &Side<'_, '_>, line: usize| {
        side.spans[line]
            .iter()
            .any(|s| s.intensity != ChangeIntensity::UnchangedContext)
    };
    let edited = |side: &Side<'_, '_>, line: usize| {
        side.spans[line].iter().any(|s| {
            !matches!(
                s.intensity,
                ChangeIntensity::UnchangedContext | ChangeIntensity::Renamed
            )
        })
    };
    for (line, owner) in new_owner.iter().enumerate() {
        if let Some(i) = owner
            && marked(&sides[1], line)
        {
            new_items[*i].edits += 1;
            new_items[*i].edited |= edited(&sides[1], line);
        }
    }
    let by_node: HashMap<u32, usize> = new_items
        .iter()
        .enumerate()
        .map(|(i, item)| (item.node, i))
        .collect();
    for (line, owner) in old_owner.iter().enumerate() {
        let Some(i) = *owner else {
            continue;
        };
        if !marked(&sides[0], line) {
            continue;
        }
        let partner = matching.partner[0][old_items[i].node as usize];
        let other = edited(&sides[0], line);
        match by_node.get(&partner) {
            Some(&j) => {
                new_items[j].old_edits += 1;
                new_items[j].edited |= other;
            }
            None => old_items[i].edits += 1,
        }
    }
    let mut out = Vec::new();
    let mut emit = |items: &[Owned], side: &Side<'_, '_>, which: DiffSide| {
        for item in items {
            if item.inside_unmatched {
                continue;
            }
            let change = match (item.unmatched, which) {
                (false, DiffSide::New) if moved_new.contains(&item.node) => ItemChange::Moved,
                (true, DiffSide::New) => ItemChange::Added,
                (true, DiffSide::Old) => ItemChange::Removed,
                (false, DiffSide::New) if item.edits.max(item.old_edits) > 0 => {
                    if item.edited {
                        ItemChange::Edited(item.edits.max(item.old_edits))
                    } else {
                        ItemChange::Renamed
                    }
                }
                _ => continue,
            };
            out.push(OutlineItem {
                side: which,
                line: item.lines.start,
                kind: item.kind.into(),
                name: side.tree.text(item.name.clone()).into(),
                change,
            });
        }
    };
    emit(&new_items, &sides[1], DiffSide::New);
    emit(&old_items, &sides[0], DiffSide::Old);
    out
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

fn replaced(sides: &mut [Side<'_, '_>; 2], x: u32, y: u32) -> Option<[Vec<Range<u32>>; 2]> {
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
        let shift = |base: u32, ranges: &[Range<u32>]| {
            ranges
                .iter()
                .map(|r| base + r.start..base + r.end)
                .collect()
        };
        return Some([shift(ra.start, &words.old), shift(rb.start, &words.new)]);
    }
    if ta != tb {
        sides[0].mark(ra, ChangeIntensity::Novel);
        sides[1].mark(rb, ChangeIntensity::Novel);
    }
    None
}

fn join_novel(text: &str, spans: &mut Vec<SemanticSpan>) {
    let mut i = 0;
    while i + 1 < spans.len() {
        let (a, b) = (spans[i], spans[i + 1]);
        let novel = a.intensity == ChangeIntensity::Novel && b.intensity == ChangeIntensity::Novel;
        let gap = text
            .get(a.end as usize..b.start as usize)
            .unwrap_or_default();
        let width = gap.len() as u32;
        if !novel || gap.contains(char::is_whitespace) {
            i += 1;
            continue;
        }
        let joined = if gap.is_empty() {
            Some(a.start..a.end.max(b.end))
        } else if a.end - a.start > width && text[a.start as usize..].starts_with(gap) {
            Some(a.start + width..b.end)
        } else if b.end - b.start > width && text[..b.end as usize].ends_with(gap) {
            Some(a.start..b.end - width)
        } else {
            None
        };
        match joined {
            Some(range) => {
                spans[i] = SemanticSpan {
                    start: range.start,
                    end: range.end,
                    intensity: ChangeIntensity::Novel,
                };
                spans.remove(i + 1);
            }
            None => i += 1,
        }
    }
}

fn mark_words(sides: &mut [Side<'_, '_>; 2], words: Vec<[Vec<Range<u32>>; 2]>, renames: &[Rename]) {
    for ranges in words {
        let mut renamed = [vec![false; ranges[0].len()], vec![false; ranges[1].len()]];
        for rename in renames {
            let hits = |side: usize, name: &str| -> Vec<usize> {
                (0..ranges[side].len())
                    .filter(|&k| sides[side].tree.text(ranges[side][k].clone()) == name)
                    .collect()
            };
            let (old, new) = (hits(0, &rename.old), hits(1, &rename.new));
            if !old.is_empty() && old.len() == new.len() {
                old.into_iter().for_each(|k| renamed[0][k] = true);
                new.into_iter().for_each(|k| renamed[1][k] = true);
            }
        }
        for side in 0..2 {
            for (k, range) in ranges[side].iter().enumerate() {
                let intensity = if renamed[side][k] {
                    ChangeIntensity::Renamed
                } else {
                    ChangeIntensity::NovelWord
                };
                sides[side].mark(range.clone(), intensity);
            }
        }
    }
}

fn anchors(
    sides: &[Side<'_, '_>; 2],
    matching: &Matching,
    displaced: &[(u32, u32)],
) -> Vec<(u32, u32, u64)> {
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
    let mut skipped = vec![false; old.tree.nodes.len()];
    for &(o, _) in displaced {
        let end = old.tree.nodes[o as usize].next;
        skipped[o as usize..end as usize].fill(true);
    }
    let mut relocated = vec![false; old.tree.nodes.len()];
    for &(o, _) in matching.relocated.iter().chain(&matching.lifted) {
        let end = old.tree.nodes[o as usize].next;
        relocated[o as usize..end as usize].fill(true);
    }
    for (index, node) in old.tree.nodes.iter().enumerate() {
        let partner = matching.partner[0][index];
        if partner == NONE || skipped[index] {
            continue;
        }
        let other = &new.tree.nodes[partner as usize];
        if relocated[index] {
            if node.class != Class::List {
                push(&node.span, &other.span, 1);
            }
            continue;
        }
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

/// Matched tokens between each old line and each new line.
type Links = HashMap<(u32, u32), u32>;

fn links(sides: &[Side<'_, '_>; 2], matching: &Matching) -> Links {
    let (old, new) = (&sides[0], &sides[1]);
    let mut out = Links::new();
    for (k, node) in old.tree.nodes.iter().enumerate() {
        let partner = matching.partner[0][k];
        if node.class == Class::List || partner == NONE {
            continue;
        }
        let a = old.lines.of(node.span.start);
        let b = new.lines.of(new.tree.nodes[partner as usize].span.start);
        *out.entry((a, b)).or_default() += 1;
    }
    out
}

fn align(
    sides: &[Side<'_, '_>; 2],
    anchors: &[(u32, u32, u64)],
    links: &Links,
) -> Vec<(Option<u32>, Option<u32>)> {
    let mut rows = Vec::new();
    let (mut o, mut n) = (0, 0);
    for &(ao, an, _) in anchors {
        fill(sides, links, o..ao, n..an, &mut rows);
        rows.push((Some(ao), Some(an)));
        (o, n) = (ao + 1, an + 1);
    }
    fill(
        sides,
        links,
        o..sides[0].lines.count(),
        n..sides[1].lines.count(),
        &mut rows,
    );
    rows
}

fn fill(
    sides: &[Side<'_, '_>; 2],
    links: &Links,
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
    for change in myers::diff(&a, &b) {
        while i < change.old.start {
            rows.push((Some(olds.start + i), Some(news.start + j)));
            i += 1;
            j += 1;
        }
        pair_lines(
            links,
            olds.start + change.old.start..olds.start + change.old.end,
            news.start + change.new.start..news.start + change.new.end,
            rows,
        );
        (i, j) = (change.old.end, change.new.end);
    }
    while i < a.len() as u32 {
        rows.push((Some(olds.start + i), Some(news.start + j)));
        i += 1;
        j += 1;
    }
}

/// Rows for a run of changed lines: lines paired where their tokens
/// matched each other, most shared tokens first and in order; lines that
/// matched nothing pair by position, as a replacement.
fn pair_lines(
    links: &Links,
    old: Range<u32>,
    new: Range<u32>,
    rows: &mut Vec<(Option<u32>, Option<u32>)>,
) {
    let (m, n) = (old.len(), new.len());
    let linked = |a: u32| (new.start..new.end).any(|b| links.contains_key(&(a, b)));
    let linked_new = |b: u32| (old.start..old.end).any(|a| links.contains_key(&(a, b)));
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    if m * n <= MAX_PAIR_CELLS {
        let weight = |i: usize, j: usize| {
            links
                .get(&(old.start + i as u32, new.start + j as u32))
                .copied()
                .unwrap_or(0)
        };
        let mut best = vec![0u32; (m + 1) * (n + 1)];
        let at = |i: usize, j: usize| i * (n + 1) + j;
        for i in 1..=m {
            for j in 1..=n {
                let w = weight(i - 1, j - 1);
                let diagonal = if w > 0 { best[at(i - 1, j - 1)] + w } else { 0 };
                best[at(i, j)] = best[at(i - 1, j)].max(best[at(i, j - 1)]).max(diagonal);
            }
        }
        let (mut i, mut j) = (m, n);
        while i > 0 && j > 0 {
            let w = weight(i - 1, j - 1);
            if w > 0 && best[at(i, j)] == best[at(i - 1, j - 1)] + w {
                pairs.push((old.start + i as u32 - 1, new.start + j as u32 - 1));
                (i, j) = (i - 1, j - 1);
            } else if best[at(i - 1, j)] >= best[at(i, j - 1)] {
                i -= 1;
            } else {
                j -= 1;
            }
        }
        pairs.reverse();
    }
    let (mut a, mut b) = (old.start, new.start);
    for (pa, pb) in pairs.into_iter().chain([(old.end, new.end)]) {
        let lone_old: Vec<u32> = (a..pa).collect();
        let lone_new: Vec<u32> = (b..pb).collect();
        let free_old: Vec<u32> = lone_old.iter().copied().filter(|&x| !linked(x)).collect();
        let free_new: Vec<u32> = lone_new
            .iter()
            .copied()
            .filter(|&y| !linked_new(y))
            .collect();
        let together = free_old.len().min(free_new.len());
        let partner_of = |x: u32| {
            free_old
                .iter()
                .position(|&f| f == x)
                .filter(|&k| k < together)
        };
        let mut k_new = 0;
        for &x in &lone_old {
            match partner_of(x) {
                Some(k) => {
                    while lone_new[k_new] != free_new[k] {
                        rows.push((None, Some(lone_new[k_new])));
                        k_new += 1;
                    }
                    rows.push((Some(x), Some(free_new[k])));
                    k_new += 1;
                }
                None => rows.push((Some(x), None)),
            }
        }
        for &y in &lone_new[k_new..] {
            rows.push((None, Some(y)));
        }
        if pa < old.end {
            rows.push((Some(pa), Some(pb)));
        }
        (a, b) = (pa + 1, pb + 1);
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

fn mark_silent(
    sides: &mut [Side<'_, '_>; 2],
    aligned: &[(Option<u32>, Option<u32>)],
    moves: &[Move],
    matching: &Matching,
) {
    let mut region = [
        vec![u32::MAX; sides[0].lines.count() as usize],
        vec![u32::MAX; sides[1].lines.count() as usize],
    ];
    let mut current = 0;
    let mut open = false;
    for &(o, n) in aligned {
        let changed = match (o, n) {
            (Some(o), Some(n)) => {
                !sides[0].spans[o as usize].is_empty()
                    || !sides[1].spans[n as usize].is_empty()
                    || sides[0].lines.text(sides[0].tree.src, o)
                        != sides[1].lines.text(sides[1].tree.src, n)
            }
            _ => true,
        };
        if !changed {
            open = false;
            continue;
        }
        if !open {
            current += 1;
            open = true;
        }
        if let Some(o) = o {
            region[0][o as usize] = current;
        }
        if let Some(n) = n {
            region[1][n as usize] = current;
        }
    }
    let mut far = [
        vec![false; sides[0].lines.count() as usize],
        vec![false; sides[1].lines.count() as usize],
    ];
    for s in 0..2 {
        let (own, other) = (&sides[s], &sides[1 - s]);
        for (k, node) in own.tree.nodes.iter().enumerate() {
            let partner = matching.partner[s][k];
            if node.class == Class::List || partner == NONE {
                continue;
            }
            let line = own.lines.of(node.span.start);
            let theirs = other
                .lines
                .of(other.tree.nodes[partner as usize].span.start);
            if region[s][line as usize] != region[1 - s][theirs as usize] {
                far[s][line as usize] = true;
            }
        }
    }
    for &(o, n) in aligned {
        let (s, line) = match (o, n) {
            (Some(o), None) => (0, o),
            (None, Some(n)) => (1, n),
            _ => continue,
        };
        let moved = moves.iter().any(|m| {
            let (start, len) = if s == 0 {
                (m.old_line, m.old_lines)
            } else {
                (m.new_line, m.lines)
            };
            (start..start + len).contains(&line)
        });
        let side = &mut sides[s];
        if moved || !far[s][line as usize] || !side.spans[line as usize].is_empty() {
            continue;
        }
        let content = side.lines.content(side.tree.src, line);
        let text = &side.tree.src[content.start as usize..content.end as usize];
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let start = (text.len() - text.trim_start().len()) as u32;
        side.spans[line as usize].push(SemanticSpan {
            start,
            end: start + trimmed.len() as u32,
            intensity: ChangeIntensity::Novel,
        });
    }
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
