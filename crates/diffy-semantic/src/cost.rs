use std::ops::{AddAssign, Index, IndexMut, Range, SubAssign};

use crate::syntax::{Class, Node, Tree};

pub(crate) const WORD: i64 = 4;
pub(crate) const PUNCT: i64 = 1;
pub(crate) const DELIMITER: i64 = 1;

pub(crate) const UNMATCHED: usize = 0;
pub(crate) const INSIDE: usize = 1;
pub(crate) const RENAMED: usize = 2;
pub(crate) const ENTRIES: usize = 3;
pub(crate) const IDENTITY: usize = 4;
pub(crate) const MOVES: usize = 5;
pub(crate) const MOVED: usize = 6;
pub(crate) const ISOLATED: usize = 7;
pub(crate) const FOLLOWS: usize = 8;
pub(crate) const SPLIT: usize = 9;

#[cfg(feature = "eval")]
pub const TERMS: [&str; 10] = [
    "unmatched",
    "inside",
    "renamed",
    "entries",
    "identity",
    "moves",
    "moved",
    "isolated",
    "follows",
    "split",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Features(pub [i64; 10]);

impl Index<usize> for Features {
    type Output = i64;

    fn index(&self, term: usize) -> &i64 {
        &self.0[term]
    }
}

impl IndexMut<usize> for Features {
    fn index_mut(&mut self, term: usize) -> &mut i64 {
        &mut self.0[term]
    }
}

impl AddAssign for Features {
    fn add_assign(&mut self, other: Self) {
        for (a, b) in self.0.iter_mut().zip(other.0) {
            *a += b;
        }
    }
}

impl SubAssign for Features {
    fn sub_assign(&mut self, other: Self) {
        for (a, b) in self.0.iter_mut().zip(other.0) {
            *a -= b;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weights {
    pub gamma: i64,
    pub rho: i64,
    pub rho0: i64,
    pub iota: i64,
    pub mu: i64,
    pub follow: i64,
    pub beta: i64,
    pub phi: i64,
}

impl Weights {
    pub const DEFAULT: Self = Self {
        gamma: 4,
        rho: 1,
        rho0: 64,
        iota: 64,
        mu: 320,
        follow: 64,
        beta: 6,
        phi: 0,
    };

    pub fn cost(&self, f: &Features) -> i64 {
        4 * f[UNMATCHED]
            + self.gamma * f[INSIDE]
            + self.rho * f[RENAMED]
            + self.rho0 * f[ENTRIES]
            + self.iota * f[IDENTITY]
            + self.mu * f[MOVES]
            + self.follow * f[FOLLOWS]
            + self.beta * f[MOVED]
            + self.phi * f[ISOLATED]
    }
}

impl Default for Weights {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub(crate) fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
}

pub(crate) fn weight(tree: &Tree<'_>, node: &Node) -> i64 {
    let text = tree.text(node.span.clone());
    match node.class {
        Class::List => {
            DELIMITER * (i64::from(!node.open.is_empty()) + i64::from(!node.close.is_empty()))
        }
        Class::Text | Class::Comment => WORD * (words(text).count() as i64).max(1),
        Class::Plain if text.bytes().any(|b| b.is_ascii_alphanumeric() || b >= 0x80) => WORD,
        Class::Plain => PUNCT,
    }
}

pub(crate) fn edited(text: &str, ranges: &[Range<u32>]) -> i64 {
    ranges
        .iter()
        .map(
            |r| match words(&text[r.start as usize..r.end as usize]).count() as i64 {
                0 => PUNCT,
                n => WORD * n,
            },
        )
        .sum()
}

pub(crate) fn line_keys(src: &str) -> (Vec<u32>, Vec<u64>) {
    let mut starts = vec![0u32];
    let mut keys = Vec::new();
    for line in src.split_inclusive('\n') {
        keys.push(crate::syntax::hash_bytes(line.trim().as_bytes()));
        starts.push(starts.last().copied().unwrap_or(0) + line.len() as u32);
    }
    keys.push(0);
    (starts, keys)
}

pub(crate) fn line_key(lines: &(Vec<u32>, Vec<u64>), byte: u32) -> u64 {
    lines.1[lines.0.partition_point(|&s| s <= byte).saturating_sub(1)]
}

pub(crate) fn same_text(class: Class, a: &str, b: &str) -> bool {
    if class == Class::Comment {
        a.lines().map(str::trim).eq(b.lines().map(str::trim))
    } else {
        a == b
    }
}

#[cfg(feature = "eval")]
pub(crate) fn evaluate(
    trees: [&Tree<'_>; 2],
    partner: &[Vec<u32>; 2],
    moves: &[(u32, u32)],
    tokens: &[Vec<crate::Token>; 2],
    renames: &[diffy_diff::Rename],
) -> Features {
    use std::collections::{HashMap, HashSet};

    use diffy_diff::{InlineDetail, InlineOptions, paired_inline_diff};

    use crate::TokenState;
    use crate::syntax::NONE;

    let parents = [
        crate::matcher::parents(trees[0]),
        crate::matcher::parents(trees[1]),
    ];
    let linked = |side: usize, node: u32| node != NONE && partner[side][node as usize] != NONE;
    let lines = [line_keys(trees[0].src), line_keys(trees[1].src)];
    let accepted: HashSet<(&str, &str)> = renames.iter().map(|r| (&*r.old, &*r.new)).collect();
    let mut f = Features::default();
    for side in 0..2 {
        let (tree, other) = (trees[side], trees[1 - side]);
        let index: HashMap<u32, &crate::Token> =
            tokens[side].iter().map(|t| (t.span.start, t)).collect();
        for (k, node) in tree.nodes.iter().enumerate() {
            let parts = if node.class == Class::List {
                vec![(node.open.clone(), false), (node.close.clone(), false)]
            } else {
                vec![(node.span.clone(), true)]
            };
            for (range, leaf) in parts {
                if range.is_empty() {
                    continue;
                }
                let Some(token) = index.get(&range.start) else {
                    continue;
                };
                let w = if leaf { weight(tree, node) } else { DELIMITER };
                let u = match (token.state, &token.partner) {
                    (TokenState::Novel, _) => w,
                    (TokenState::Edited, Some(peer)) => {
                        let (mine, theirs) = (tree.text(range.clone()), other.text(peer.clone()));
                        let (old, new) = if side == 0 {
                            (mine, theirs)
                        } else {
                            (theirs, mine)
                        };
                        let found = paired_inline_diff(old, new, &InlineOptions::default());
                        if found.detail == InlineDetail::Exact {
                            edited(mine, if side == 0 { &found.old } else { &found.new })
                        } else {
                            w
                        }
                    }
                    _ => 0,
                };
                f[UNMATCHED] += u;
                if leaf && linked(side, parents[side][k]) {
                    f[INSIDE] += u;
                }
                if token.state == TokenState::Renamed {
                    f[RENAMED] += w;
                }
                if token.moved.is_some() {
                    f[MOVED] += u;
                }
                if let (0, Some(peer)) = (side, &token.partner)
                    && token.state != TokenState::Novel
                    && line_key(&lines[0], range.start) != line_key(&lines[1], peer.start)
                {
                    f[SPLIT] += 1;
                }
                let peer = partner[side][k];
                if side == 0
                    && leaf
                    && token.state != TokenState::Novel
                    && peer != NONE
                    && !linked(0, parents[0][k])
                    && !linked(1, parents[1][peer as usize])
                {
                    f[ISOLATED] += 1;
                }
            }
        }
    }
    f[ENTRIES] = renames.len() as i64;
    let (lead, follow) = crate::align::Notes::new(trees[0], trees[1]).count(moves);
    f[MOVES] = lead;
    f[FOLLOWS] = follow;
    for item in &trees[0].items {
        let peer = partner[0][item.node as usize];
        if peer == NONE {
            continue;
        }
        let Ok(at) = trees[1].items.binary_search_by_key(&peer, |i| i.node) else {
            continue;
        };
        let (p, q) = (
            trees[0].text(item.name.clone()),
            trees[1].text(trees[1].items[at].name.clone()),
        );
        if p != q && !accepted.contains(&(p, q)) {
            f[IDENTITY] += 1;
        }
    }
    f
}
