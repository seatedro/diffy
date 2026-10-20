use std::collections::{HashMap, HashSet};
use std::ops::Range;

use diffy_diff::{
    ChangeIntensity, InlineDetail, InlineOptions, ItemChange, Move, OutlineItem, Rename,
    SemanticSpan, Side as DiffSide, move_leaders, myers, paired_inline_diff,
};

use crate::cost::same_text;
use crate::matcher::Matching;
use crate::syntax::{Class, NONE, Tree};
use crate::{Group, Row, Token, TokenState};

const CONTEXT: usize = 3;

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

    fn last(&self, range: &Range<u32>) -> u32 {
        self.of(range.end.saturating_sub(1).max(range.start))
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

    fn text<'a>(&self, text: &'a str, line: u32) -> &'a str {
        let range = self.content(text, line);
        &text[range.start as usize..range.end as usize]
    }
}

struct Side<'t, 'a> {
    tree: &'t Tree<'a>,
    lines: Lines,
    spans: Vec<Vec<SemanticSpan>>,
    tokens: Vec<Token>,
    /// Per node, the move whose subtree holds it, or [`NONE`].
    moved: Vec<u32>,
    /// Per line, whether a moved token starts on it.
    moved_lines: Vec<bool>,
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

    fn text(&self, node: u32) -> &str {
        self.tree.text(self.tree.nodes[node as usize].span.clone())
    }
}

pub(crate) struct Output {
    pub(crate) chunks: Vec<Vec<Row>>,
    pub(crate) outline: Vec<OutlineItem>,
    pub(crate) renames: Vec<Rename>,
    pub(crate) moves: Vec<Move>,
    pub(crate) groups: Vec<Group>,
    pub(crate) tokens: [Vec<Token>; 2],
}

pub(crate) fn rows(old: &Tree<'_>, new: &Tree<'_>, matching: &Matching) -> Output {
    let mut sides = [(old, 0usize), (new, 1usize)].map(|(tree, side)| {
        let lines = Lines::new(tree.src);
        let spans = vec![Vec::new(); lines.count() as usize];
        let mut moved = vec![NONE; tree.nodes.len()];
        for (index, &(o, n)) in matching.moves.iter().enumerate() {
            let root = if side == 0 { o } else { n };
            let end = tree.nodes[root as usize].next;
            for slot in &mut moved[root as usize..end as usize] {
                if *slot == NONE {
                    *slot = index as u32;
                }
            }
        }
        let moved_lines = vec![false; lines.count() as usize];
        Side {
            tree,
            lines,
            spans,
            tokens: Vec::new(),
            moved,
            moved_lines,
        }
    });
    let renames = states(&mut sides, matching);
    for side in &mut sides {
        for token in &side.tokens {
            if token.moved.is_some() {
                let line = side.lines.of(token.span.start) as usize;
                side.moved_lines[line] = true;
            }
        }
    }
    for side in &mut sides {
        for spans in &mut side.spans {
            spans.sort_by_key(|span| span.start);
            join_touching(spans);
        }
    }
    let outline = outline(&sides, matching);
    let moves = moves(&sides, matching);
    let groups = groups(&sides, matching);
    let aligned = align(&sides, &groups);
    let multi = groups
        .iter()
        .filter(|g| g.old.len() > 1 || g.new.len() > 1)
        .cloned()
        .collect();
    let tokens = sides
        .each_mut()
        .map(|side| std::mem::take(&mut side.tokens));
    Output {
        chunks: chunks(&mut sides, aligned),
        outline,
        renames,
        moves,
        groups: multi,
        tokens,
    }
}

/// Every token's state, as spans, and the matcher's renames.
fn states(sides: &mut [Side<'_, '_>; 2], matching: &Matching) -> Vec<Rename> {
    let accepted: HashSet<(&str, &str)> = matching
        .renames
        .iter()
        .map(|r| (&*r.old, &*r.new))
        .collect();
    let mut replaced: Vec<(u32, u32)> = Vec::new();
    let mut rename_of: HashMap<u32, bool> = HashMap::new();
    let side = &sides[0];
    for (k, node) in side.tree.nodes.iter().enumerate() {
        if node.class == Class::List {
            continue;
        }
        let partner = matching.partner[0][k];
        if partner == NONE {
            continue;
        }
        let other = &sides[1].tree.nodes[partner as usize];
        let (a, b) = (side.text(k as u32), sides[1].text(partner));
        if same_text(node.class, a, b) {
            continue;
        }
        if node.name && other.name {
            rename_of.insert(k as u32, accepted.contains(&(a, b)));
        } else if matches!(node.class, Class::Text | Class::Comment) {
            replaced.push((k as u32, partner));
        }
    }
    let mut renames = matching.renames.clone();
    let mut words: Vec<[Vec<Range<u32>>; 2]> = Vec::new();
    let mut edited: HashSet<u32> = HashSet::new();
    for &(a, b) in &replaced {
        let (ra, rb) = (
            sides[0].tree.nodes[a as usize].span.clone(),
            sides[1].tree.nodes[b as usize].span.clone(),
        );
        let found = paired_inline_diff(
            sides[0].tree.text(ra.clone()),
            sides[1].tree.text(rb.clone()),
            &InlineOptions::default(),
        );
        if found.detail == InlineDetail::Exact && (!found.old.is_empty() || !found.new.is_empty()) {
            let shift = |base: u32, ranges: &[Range<u32>]| {
                ranges
                    .iter()
                    .map(|r| base + r.start..base + r.end)
                    .collect()
            };
            words.push([shift(ra.start, &found.old), shift(rb.start, &found.new)]);
            edited.insert(a);
        }
    }
    for s in 0..2 {
        let len = sides[s].tree.nodes.len();
        for k in 0..len {
            let (own, other) = (&sides[s], &sides[1 - s]);
            let node = &own.tree.nodes[k];
            let partner = matching.partner[s][k];
            let moved = (own.moved[k] != NONE).then_some(own.moved[k]);
            let theirs = (partner != NONE).then(|| &other.tree.nodes[partner as usize]);
            let mut found: Vec<(Range<u32>, Option<Range<u32>>)> = Vec::new();
            if node.class == Class::List {
                for (mine, pick) in [(node.open.clone(), true), (node.close.clone(), false)] {
                    if !mine.is_empty() {
                        let peer = theirs.map(|t| {
                            if pick {
                                t.open.clone()
                            } else {
                                t.close.clone()
                            }
                        });
                        found.push((mine, peer));
                    }
                }
            } else {
                found.push((node.span.clone(), theirs.map(|t| t.span.clone())));
            }
            let list = node.class == Class::List;
            let old_node = if s == 0 { k as u32 } else { partner };
            let states: Vec<(Range<u32>, TokenState, Option<Range<u32>>)> = found
                .into_iter()
                .map(|(range, peer)| {
                    let state = match &peer {
                        None => TokenState::Novel,
                        Some(peer)
                            if list
                                || same_text(
                                    node.class,
                                    own.tree.text(range.clone()),
                                    other.tree.text(peer.clone()),
                                ) =>
                        {
                            TokenState::Kept
                        }
                        Some(_) => match rename_of.get(&old_node) {
                            Some(true) => TokenState::Renamed,
                            Some(false) => TokenState::Novel,
                            None if edited.contains(&old_node) => TokenState::Edited,
                            None => TokenState::Novel,
                        },
                    };
                    (range, state, peer)
                })
                .collect();
            for (range, state, partner) in states {
                match state {
                    TokenState::Novel => sides[s].mark(range.clone(), ChangeIntensity::Novel),
                    TokenState::Renamed => sides[s].mark(range.clone(), ChangeIntensity::Renamed),
                    _ => {}
                }
                sides[s].tokens.push(Token {
                    span: range,
                    state,
                    partner,
                    moved,
                });
            }
        }
    }
    mark_words(sides, words, &renames);
    renames.sort_by(|a, b| b.uses.cmp(&a.uses).then(a.old.cmp(&b.old)));
    renames
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

fn join_touching(spans: &mut Vec<SemanticSpan>) {
    let mut out: Vec<SemanticSpan> = Vec::with_capacity(spans.len());
    for span in spans.drain(..) {
        match out.last_mut() {
            Some(last)
                if last.intensity == span.intensity
                    && span.intensity == ChangeIntensity::Novel
                    && span.start <= last.end =>
            {
                last.end = last.end.max(span.end);
            }
            Some(last) if span.start < last.end => {}
            _ => out.push(span),
        }
    }
    *spans = out;
}

fn moves(sides: &[Side<'_, '_>; 2], matching: &Matching) -> Vec<Move> {
    let trees = [sides[0].tree, sides[1].tree];
    let lines = [&sides[0].lines, &sides[1].lines];
    let mut out: Vec<Move> = matching
        .moves
        .iter()
        .map(|&(o, n)| place(trees, lines, o, n))
        .collect();
    out.sort_by_key(|m| (m.new_line, m.old_line));
    out
}

fn place(trees: [&Tree<'_>; 2], lines: [&Lines; 2], o: u32, n: u32) -> Move {
    let (a, b) = (
        trees[0].nodes[o as usize].span.clone(),
        trees[1].nodes[n as usize].span.clone(),
    );
    let old_line = lines[0].of(a.start);
    let new_line = lines[1].of(b.start);
    Move {
        old_line,
        old_lines: lines[0].last(&a) + 1 - old_line,
        new_line,
        lines: lines[1].last(&b) + 1 - new_line,
        from: enclosing(trees[0], o),
        into: enclosing(trees[1], n),
        item: own_item(trees[0], o),
    }
}

/// Where move notes land: the moves a reader sees, grouped as the
/// display groups them.
pub(crate) struct Notes<'t, 'a> {
    trees: [&'t Tree<'a>; 2],
    lines: [Lines; 2],
}

impl<'t, 'a> Notes<'t, 'a> {
    pub(crate) fn new(old: &'t Tree<'a>, new: &'t Tree<'a>) -> Self {
        Self {
            trees: [old, new],
            lines: [Lines::new(old.src), Lines::new(new.src)],
        }
    }

    pub(crate) fn place(&self, o: u32, n: u32) -> Move {
        place(self.trees, [&self.lines[0], &self.lines[1]], o, n)
    }

    pub(crate) fn count(&self, moves: &[(u32, u32)]) -> (i64, i64) {
        let list: Vec<Move> = moves.iter().map(|&(o, n)| self.place(o, n)).collect();
        let leaders = move_leaders(&list);
        let lead = leaders
            .iter()
            .enumerate()
            .filter(|&(i, &l)| l as usize == i)
            .count() as i64;
        (lead, list.len() as i64 - lead)
    }
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

/// Line groups: lines joined by corresponding tokens outside moves.
/// Tokens never cross, so each group is a range on each side and the
/// groups are ordered on both.
fn groups(sides: &[Side<'_, '_>; 2], matching: &Matching) -> Vec<Group> {
    let (old, new) = (&sides[0], &sides[1]);
    let mut pairs: Vec<(Range<u32>, Range<u32>)> = Vec::new();
    for (k, node) in old.tree.nodes.iter().enumerate() {
        let partner = matching.partner[0][k];
        if partner == NONE || old.moved[k] != NONE || new.moved[partner as usize] != NONE {
            continue;
        }
        let other = &new.tree.nodes[partner as usize];
        if node.class == Class::List {
            for (a, b) in [(&node.open, &other.open), (&node.close, &other.close)] {
                if !a.is_empty() && !b.is_empty() {
                    let (a, b) = (old.lines.of(a.start), new.lines.of(b.start));
                    pairs.push((a..a + 1, b..b + 1));
                }
            }
        } else {
            pairs.push((
                old.lines.of(node.span.start)..old.lines.last(&node.span) + 1,
                new.lines.of(other.span.start)..new.lines.last(&other.span) + 1,
            ));
        }
    }
    pairs.sort_unstable_by_key(|(a, b)| (a.start, b.start, a.end, b.end));
    let mut out: Vec<Group> = Vec::new();
    for (a, b) in pairs {
        match out.last_mut() {
            Some(g) if a.start < g.old.end || b.start < g.new.end => {
                g.old.end = g.old.end.max(a.end);
                g.new.end = g.new.end.max(b.end);
            }
            _ => out.push(Group { old: a, new: b }),
        }
    }
    out
}

fn align(sides: &[Side<'_, '_>; 2], groups: &[Group]) -> Vec<(Option<u32>, Option<u32>)> {
    let mut rows = Vec::new();
    let (mut o, mut n) = (0, 0);
    for group in groups {
        fill(sides, o..group.old.start, n..group.new.start, &mut rows);
        fill(sides, group.old.clone(), group.new.clone(), &mut rows);
        (o, n) = (group.old.end, group.new.end);
    }
    fill(
        sides,
        o..sides[0].lines.count(),
        n..sides[1].lines.count(),
        &mut rows,
    );
    rows
}

/// Rows for the lines between two groups: lines of equal text pair, a
/// moved line stands alone, and the rest pair in order as replacements.
fn fill<'a>(
    sides: &[Side<'_, 'a>; 2],
    olds: Range<u32>,
    news: Range<u32>,
    rows: &mut Vec<(Option<u32>, Option<u32>)>,
) {
    if olds.is_empty() || news.is_empty() {
        rows.extend(olds.map(|x| (Some(x), None)));
        rows.extend(news.map(|y| (None, Some(y))));
        return;
    }
    let moved = |s: usize, line: u32| sides[s].moved_lines[line as usize];
    let mut ids: HashMap<&'a str, u32> = HashMap::new();
    let mut fresh = u32::MAX;
    let mut id = |s: usize, line: u32, ids: &mut HashMap<&'a str, u32>| {
        if moved(s, line) {
            fresh -= 1;
            return fresh;
        }
        let next = ids.len() as u32;
        *ids.entry(sides[s].lines.text(sides[s].tree.src, line))
            .or_insert(next)
    };
    let a: Vec<u32> = olds.clone().map(|x| id(0, x, &mut ids)).collect();
    let b: Vec<u32> = news.clone().map(|y| id(1, y, &mut ids)).collect();
    let (mut i, mut j) = (0u32, 0u32);
    for change in myers::diff(&a, &b) {
        while i < change.old.start {
            rows.push((Some(olds.start + i), Some(news.start + j)));
            (i, j) = (i + 1, j + 1);
        }
        let old: Vec<u32> = (change.old.clone()).map(|x| olds.start + x).collect();
        let new: Vec<u32> = (change.new.clone()).map(|y| news.start + y).collect();
        let (mut p, mut q) = (0, 0);
        while p < old.len() || q < new.len() {
            if p < old.len() && moved(0, old[p]) {
                rows.push((Some(old[p]), None));
                p += 1;
            } else if q < new.len() && moved(1, new[q]) {
                rows.push((None, Some(new[q])));
                q += 1;
            } else if p < old.len() && q < new.len() {
                rows.push((Some(old[p]), Some(new[q])));
                (p, q) = (p + 1, q + 1);
            } else if p < old.len() {
                rows.push((Some(old[p]), None));
                p += 1;
            } else {
                rows.push((None, Some(new[q])));
                q += 1;
            }
        }
        (i, j) = (change.old.end, change.new.end);
    }
    while i < a.len() as u32 {
        rows.push((Some(olds.start + i), Some(news.start + j)));
        (i, j) = (i + 1, j + 1);
    }
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
            Owned {
                lines: side.lines.of(span.start)..side.lines.last(span) + 1,
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
    let moved_old: HashSet<u32> = matching.moves.iter().map(|&(o, _)| o).collect();
    let moved_new: HashSet<u32> = matching.moves.iter().map(|&(_, n)| n).collect();
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
