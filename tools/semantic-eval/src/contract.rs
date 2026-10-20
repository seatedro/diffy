use std::collections::HashMap;
use std::ops::Range;

use diffy::core::compare::semantic::semantic_language;
use diffy_semantic::{Limits, SemanticDiff, TokenState};

use crate::engine;

/// The engine's output contract, checked on its full explanation.
/// Returns one message per broken rule; empty when the input has no
/// grammar or falls back to a line diff.
pub fn check(path: &str, old: &[u8], new: &[u8]) -> Vec<String> {
    let (Ok(old), Ok(new)) = (std::str::from_utf8(old), std::str::from_utf8(new)) else {
        return Vec::new();
    };
    let Some(name) = semantic_language(path) else {
        return Vec::new();
    };
    let (Some(language), Some(config)) = (engine::language(name), diffy_semantic::config(name))
    else {
        return Vec::new();
    };
    let limits = Limits::default();
    let Ok(first) = diffy_semantic::diff(&language, config, old, new, &limits) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match diffy_semantic::diff(&language, config, old, new, &limits) {
        Ok(second) if same(&first, &second) => {}
        _ => out.push("two runs differ".to_owned()),
    }
    let lines = [Lines::new(old), Lines::new(new)];
    symmetric(&first, &mut out);
    ordered(&first, &mut out);
    placed(&first, &lines, &mut out);
    disjoint(&first, &mut out);
    renamed(&first, [old, new], &mut out);
    out.sort();
    out.dedup();
    out
}

fn same(a: &SemanticDiff, b: &SemanticDiff) -> bool {
    a.tokens == b.tokens
        && a.groups == b.groups
        && a.moves == b.moves
        && a.chunks.len() == b.chunks.len()
        && a.chunks.iter().zip(&b.chunks).all(|(x, y)| x == y)
}

fn symmetric(diff: &SemanticDiff, out: &mut Vec<String>) {
    let back: HashMap<(u32, u32), &Option<Range<u32>>> = diff.tokens[1]
        .iter()
        .map(|t| ((t.span.start, t.span.end), &t.partner))
        .collect();
    for token in &diff.tokens[0] {
        let Some(partner) = &token.partner else {
            continue;
        };
        match back.get(&(partner.start, partner.end)) {
            Some(Some(mine)) if *mine == token.span => {}
            _ => out.push(format!(
                "link at old byte {} is one-sided",
                token.span.start
            )),
        }
    }
}

fn ordered(diff: &SemanticDiff, out: &mut Vec<String>) {
    let mut groups: HashMap<Option<u32>, Vec<(u32, u32)>> = HashMap::new();
    for token in &diff.tokens[0] {
        if let Some(partner) = &token.partner {
            groups
                .entry(token.moved)
                .or_default()
                .push((token.span.start, partner.start));
        }
    }
    for (moved, mut pairs) in groups {
        pairs.sort_unstable();
        if pairs.windows(2).any(|w| w[1].1 <= w[0].1) {
            out.push(match moved {
                Some(m) => format!("links cross inside move {m}"),
                None => "links cross outside moves".to_owned(),
            });
        }
    }
}

/// A line shown as only removed or only added holds no token that
/// corresponds in place to one on the other side.
fn placed(diff: &SemanticDiff, lines: &[Lines; 2], out: &mut Vec<String>) {
    let mut kept: [HashMap<u32, u32>; 2] = [HashMap::new(), HashMap::new()];
    for (s, tokens) in diff.tokens.iter().enumerate() {
        for token in tokens {
            if token.partner.is_some() && token.moved.is_none() && token.state != TokenState::Novel
            {
                kept[s].insert(lines[s].of(token.span.start), token.span.start);
            }
        }
    }
    for row in diff.chunks.iter().flatten() {
        let alone = match (row.old, row.new) {
            (Some(o), None) => Some((0, o)),
            (None, Some(n)) => Some((1, n)),
            _ => None,
        };
        let grouped = |s: usize, line: u32| {
            diff.groups.iter().any(|g| {
                let range = if s == 0 { &g.old } else { &g.new };
                range.contains(&line)
            })
        };
        if let Some((s, line)) = alone
            && kept[s].contains_key(&line)
            && !grouped(s, line)
        {
            let side = if s == 0 { "old" } else { "new" };
            out.push(format!(
                "{side} line {} stands alone but keeps a token",
                line + 1
            ));
        }
    }
}

fn disjoint(diff: &SemanticDiff, out: &mut Vec<String>) {
    for (i, a) in diff.moves.iter().enumerate() {
        for b in &diff.moves[i + 1..] {
            let overlap = |x: u32, xl: u32, y: u32, yl: u32| x < y + yl && y < x + xl;
            if overlap(a.old_line, a.old_lines, b.old_line, b.old_lines)
                || overlap(a.new_line, a.lines, b.new_line, b.lines)
            {
                out.push(format!(
                    "moves overlap at old line {} and {}",
                    a.old_line + 1,
                    b.old_line + 1
                ));
            }
        }
    }
}

fn renamed(diff: &SemanticDiff, text: [&str; 2], out: &mut Vec<String>) {
    for token in &diff.tokens[0] {
        if token.state != TokenState::Renamed {
            continue;
        }
        let Some(partner) = &token.partner else {
            out.push(format!(
                "renamed token at old byte {} has no partner",
                token.span.start
            ));
            continue;
        };
        let old = &text[0][token.span.start as usize..token.span.end as usize];
        let new = &text[1][partner.start as usize..partner.end as usize];
        if !diff
            .renames
            .iter()
            .any(|r| &*r.old == old && &*r.new == new)
        {
            out.push(format!("rename {old} -> {new} is not reported"));
        }
    }
}

struct Lines {
    starts: Vec<u32>,
}

impl Lines {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n')
                .map(|(i, _)| i as u32 + 1),
        );
        Self { starts }
    }

    fn of(&self, byte: u32) -> u32 {
        (self.starts.partition_point(|&s| s <= byte) as u32).saturating_sub(1)
    }
}
