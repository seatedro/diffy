use std::collections::{HashMap, HashSet};
use std::ops::Range;

use diffy_semantic::{SemanticDiff, Token};
use tree_sitter as ts;

pub type Span = Range<u32>;

#[derive(Default, Clone, Copy, serde::Serialize)]
pub struct Score {
    pub right: u32,
    pub equivalent: u32,
    pub wrong: u32,
    pub missed: u32,
    pub items: u32,
    pub items_wrong: u32,
}

impl Score {
    pub fn add(&mut self, other: &Score) {
        self.right += other.right;
        self.equivalent += other.equivalent;
        self.wrong += other.wrong;
        self.missed += other.missed;
        self.items += other.items;
        self.items_wrong += other.items_wrong;
    }

    pub fn truth(&self) -> u32 {
        self.right + self.equivalent + self.wrong + self.missed
    }

    pub fn misleads(&self) -> bool {
        self.items_wrong > 0 || self.wrong * 50 > self.truth().max(50)
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

    fn partner(&self, side: usize, span: &Span) -> Option<Span> {
        self.sides[side]
            .get(&span.start)
            .and_then(|t| t.partner.clone())
    }

    pub fn tokens(&self, side: usize) -> impl Iterator<Item = &'d Token> + '_ {
        self.sides[side].values().copied()
    }
}

pub fn grade(
    texts: [&str; 2],
    partners: &Partners<'_>,
    truth: &[(Span, Span)],
    items: &HashSet<u32>,
) -> (Score, Option<String>) {
    let mut score = Score::default();
    let mut first = None;
    let text = |side: usize, span: &Span| &texts[side][span.start as usize..span.end as usize];
    let worded = |s: &str| s.chars().any(|c| c.is_alphanumeric() || c == '_');
    for (old, new) in truth {
        let item = items.contains(&new.start);
        score.items += u32::from(item);
        let forward = partners.partner(1, new);
        if forward.as_ref() == Some(old) {
            score.right += 1;
            continue;
        }
        let back = partners.partner(0, old);
        if forward.is_none() && back.is_none() {
            score.missed += 1;
            continue;
        }
        let same = forward.as_ref().is_none_or(|p| text(0, p) == text(0, old))
            && back.as_ref().is_none_or(|q| text(1, q) == text(1, new));
        if same && !worded(text(1, new)) && !item {
            score.equivalent += 1;
            continue;
        }
        score.wrong += 1;
        score.items_wrong += u32::from(item);
        if first.is_none() || item && score.items_wrong == 1 {
            let at = |side: usize, span: &Span| line_of(texts[side], span.start) + 1;
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
    (score, first)
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
        node.child_by_field_name("type")
            .map(|t| format!("impl {}", &text[t.byte_range()]))
            .map(|label| (label, None))
    } else if declares(kind) {
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

fn declares(kind: &str) -> bool {
    matches!(
        kind,
        "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "mod_item"
            | "union_item"
            | "type_item"
            | "macro_definition"
            | "function_declaration"
            | "method_declaration"
            | "type_spec"
            | "function_definition"
            | "class_definition"
            | "class_declaration"
            | "interface_declaration"
            | "method_definition"
            | "struct_specifier"
            | "enum_specifier"
    )
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
