use std::collections::{HashMap, HashSet};
use std::ops::Range;

use diffy_semantic::{SemanticDiff, Token};
use tree_sitter as ts;

pub type Span = Range<u32>;

#[derive(Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Score {
    pub right: u32,
    pub equivalent: u32,
    pub wrong: u32,
    pub missed: u32,
    pub spurious: u32,
    pub items: u32,
    pub items_wrong: u32,
    pub lists: u32,
    pub lists_wrong: u32,
}

impl Score {
    pub fn add(&mut self, other: &Score) {
        self.right += other.right;
        self.equivalent += other.equivalent;
        self.wrong += other.wrong;
        self.missed += other.missed;
        self.spurious += other.spurious;
        self.items += other.items;
        self.items_wrong += other.items_wrong;
        self.lists += other.lists;
        self.lists_wrong += other.lists_wrong;
    }

    pub fn truth(&self) -> u32 {
        self.right + self.equivalent + self.wrong + self.missed
    }

    pub fn linked(&self) -> u32 {
        self.right + self.equivalent + self.wrong + self.spurious
    }

    pub fn misleads(&self) -> bool {
        self.items_wrong > 0 || (self.wrong + self.spurious) * 50 > self.truth().max(50)
    }

    pub fn incomplete(&self) -> bool {
        self.missed * 20 > self.truth().max(20)
    }
}

#[derive(Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Grade {
    pub all: Score,
    pub region: Score,
}

impl Grade {
    pub fn add(&mut self, other: &Grade) {
        self.all.add(&other.all);
        self.region.add(&other.region);
    }

    pub fn misleads(&self) -> bool {
        self.all.misleads() || self.region.misleads()
    }
}

pub struct Partners<'d> {
    sides: [HashMap<u32, &'d Token>; 2],
}

impl<'d> Partners<'d> {
    pub fn new(diff: &'d SemanticDiff) -> Self {
        let index = |tokens: &'d [Token]| tokens.iter().map(|t| (t.span.start, t)).collect();
        Self {
            sides: [index(&diff.tokens[0]), index(&diff.tokens[1])],
        }
    }

    pub fn token(&self, side: usize, span: &Span) -> Option<&'d Token> {
        self.sides[side]
            .get(&span.start)
            .copied()
            .filter(|t| t.span.end == span.end)
    }

    pub fn partner(&self, side: usize, span: &Span) -> Option<Span> {
        self.sides[side]
            .get(&span.start)
            .and_then(|t| t.partner.clone())
    }

    pub fn tokens(&self, side: usize) -> impl Iterator<Item = &'d Token> + '_ {
        self.sides[side].values().copied()
    }
}

pub struct Truth<'a> {
    pub links: &'a [(Span, Span)],
    pub items: &'a HashSet<u32>,
    pub delimiters: &'a [HashSet<u32>; 2],
    pub unlinked: [&'a [Span]; 2],
    pub region: Option<(usize, &'a [Span])>,
}

impl Truth<'_> {
    fn in_region(&self, old: &Span, new: &Span) -> bool {
        match self.region {
            None => true,
            Some((side, ranges)) => {
                let at = if side == 0 { old.start } else { new.start };
                ranges.iter().any(|r| r.contains(&at))
            }
        }
    }
}

pub fn grade(
    texts: [&str; 2],
    partners: &Partners<'_>,
    truth: &Truth<'_>,
) -> (Grade, Option<String>) {
    let mut grade = Grade::default();
    let mut first = None;
    let text = |side: usize, span: &Span| &texts[side][span.start as usize..span.end as usize];
    let worded = |s: &str| s.chars().any(|c| c.is_alphanumeric() || c == '_');
    let at = |side: usize, span: &Span| line_of(texts[side], span.start) + 1;
    for (old, new) in truth.links {
        let region = truth.in_region(old, new);
        let item = truth.items.contains(&new.start);
        let list = truth.delimiters[1].contains(&new.start);
        let mut tally = |f: &mut dyn FnMut(&mut Score)| {
            f(&mut grade.all);
            if region {
                f(&mut grade.region);
            }
        };
        tally(&mut |s| {
            s.items += u32::from(item);
            s.lists += u32::from(list);
        });
        let forward = partners.partner(1, new);
        if forward.as_ref() == Some(old) {
            tally(&mut |s| s.right += 1);
            continue;
        }
        let back = partners.partner(0, old);
        if forward.is_none() && back.is_none() {
            tally(&mut |s| s.missed += 1);
            continue;
        }
        let same = forward.as_ref().is_none_or(|p| text(0, p) == text(0, old))
            && back.as_ref().is_none_or(|q| text(1, q) == text(1, new));
        if same && !worded(text(1, new)) && !item && !list {
            tally(&mut |s| s.equivalent += 1);
            continue;
        }
        tally(&mut |s| {
            s.wrong += 1;
            s.items_wrong += u32::from(item);
            s.lists_wrong += u32::from(list);
        });
        if first.is_none() || item && grade.all.items_wrong == 1 {
            let rival = match (&forward, &back) {
                (Some(p), _) => format!("pairs with old line {}", at(0, p)),
                (None, Some(q)) => format!("old pairs with new line {}", at(1, q)),
                (None, None) => String::new(),
            };
            first = Some(format!(
                "`{}` old line {} -> new line {}: {rival}",
                text(1, new),
                at(0, old),
                at(1, new)
            ));
        }
    }
    let banned: [HashSet<u32>; 2] = truth
        .unlinked
        .map(|spans| spans.iter().map(|s| s.start).collect());
    for side in [1, 0] {
        for span in truth.unlinked[side] {
            let Some(peer) = partners.partner(side, span) else {
                continue;
            };
            if side == 0 && banned[1].contains(&peer.start) {
                continue;
            }
            let (old, new) = if side == 0 {
                (span.clone(), peer)
            } else {
                (peer, span.clone())
            };
            let region = truth.in_region(&old, &new);
            grade.all.spurious += 1;
            if region {
                grade.region.spurious += 1;
            }
            if first.is_none() && worded(text(side, span)) {
                first = Some(format!(
                    "`{}` old line {} -> new line {}: should stay unlinked",
                    text(side, span),
                    at(0, &old),
                    at(1, &new)
                ));
            }
        }
    }
    (grade, first)
}

pub fn line_of(text: &str, byte: u32) -> usize {
    text.as_bytes()[..(byte as usize).min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

pub struct Item {
    pub key: String,
    pub name: Span,
}

pub fn items(root: ts::Node<'_>, text: &str) -> Vec<Item> {
    let mut out = Vec::new();
    walk(root, text, &mut Vec::new(), &mut out);
    out
}

fn walk(node: ts::Node<'_>, text: &str, scope: &mut Vec<String>, out: &mut Vec<Item>) {
    let kind = node.kind();
    let named = if kind == "impl_item" {
        node.child_by_field_name("type").map(|t| {
            let ty = &text[t.byte_range()];
            let label = match node.child_by_field_name("trait") {
                Some(tr) => format!("impl {} for {ty}", &text[tr.byte_range()]),
                None => format!("impl {ty}"),
            };
            (label, None)
        })
    } else if declares(node) {
        name_node(node).map(|n| (format!("{kind} {}", &text[n.byte_range()]), Some(n)))
    } else {
        None
    };
    if let Some((label, name)) = &named {
        if let Some(name) = name {
            out.push(Item {
                key: format!("{}/{label}", scope.join("/")),
                name: name.start_byte() as u32..name.end_byte() as u32,
            });
        }
        scope.push(label.clone());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, text, scope, out);
    }
    if named.is_some() {
        scope.pop();
    }
}

fn declares(node: ts::Node<'_>) -> bool {
    match node.kind() {
        "struct_specifier" | "enum_specifier" | "union_specifier" => {
            node.child_by_field_name("body").is_some()
        }
        kind => matches!(
            kind,
            "function_item"
                | "function_signature_item"
                | "struct_item"
                | "enum_item"
                | "trait_item"
                | "mod_item"
                | "union_item"
                | "type_item"
                | "const_item"
                | "static_item"
                | "macro_definition"
                | "function_declaration"
                | "generator_function_declaration"
                | "method_declaration"
                | "type_spec"
                | "type_alias_declaration"
                | "enum_declaration"
                | "function_definition"
                | "class_definition"
                | "class_declaration"
                | "abstract_class_declaration"
                | "interface_declaration"
                | "method_definition"
        ),
    }
}

pub fn name_node(node: ts::Node<'_>) -> Option<ts::Node<'_>> {
    if let Some(name) = node.child_by_field_name("name") {
        return (name.child_count() == 0).then_some(name);
    }
    let mut at = node.child_by_field_name("declarator")?;
    loop {
        if at.child_count() == 0 {
            return at.kind().contains("identifier").then_some(at);
        }
        at = at.child_by_field_name("declarator")?;
    }
}

pub fn same_named(old: &[Item], new: &[Item]) -> Vec<(Span, Span)> {
    fn count(items: &[Item]) -> HashMap<&str, (u32, Span)> {
        let mut out: HashMap<&str, (u32, Span)> = HashMap::new();
        for item in items {
            let entry = out.entry(&item.key).or_insert((0, item.name.clone()));
            entry.0 += 1;
        }
        out
    }
    let (olds, news) = (count(old), count(new));
    let mut out: Vec<(Span, Span)> = olds
        .iter()
        .filter_map(|(key, (n, o))| match news.get(key) {
            Some((1, m)) if *n == 1 => Some((o.clone(), m.clone())),
            _ => None,
        })
        .collect();
    out.sort_by_key(|(o, _)| o.start);
    out
}
