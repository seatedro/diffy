use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Mutex, PoisonError};

use tree_sitter as ts;
use tree_sitter::StreamingIterator;

use crate::query::{self, Capture};
use crate::syntax::span;

const PAIRS: [(&str, &str); 4] = [("(", ")"), ("[", "]"), ("{", "}"), ("<", ">")];

#[derive(Debug, Clone, Copy)]
pub struct Queries<'q> {
    pub highlights: &'q str,
    pub locals: Option<&'q str>,
}

pub struct Rules {
    text: Vec<bool>,
    lead: Vec<bool>,
    unit: Vec<bool>,
    pub(crate) delimiters: Vec<(Box<str>, Box<str>)>,
    items: Option<Items>,
}

struct Items {
    query: ts::Query,
    roles: Vec<Role>,
}

#[derive(Clone, Copy)]
enum Role {
    None,
    Item(&'static str),
    Name,
    Key,
    Definition(&'static str),
}

pub(crate) struct Found {
    pub(crate) kind: &'static str,
    pub(crate) name: Range<u32>,
    pub(crate) key: Range<u32>,
    rank: (usize, u32),
}

impl Rules {
    pub fn new(
        language: &ts::Language,
        semantic: &str,
        queries: &Queries<'_>,
    ) -> Result<Self, ts::QueryError> {
        let mut rules = Self::empty(language);
        let patterns = query::patterns(semantic);
        let mut tokens: Vec<&str> = Vec::new();
        for capture in patterns.iter().flat_map(|p| &p.captures) {
            let set = match capture.name {
                "text" => &mut rules.text,
                "item.lead" => &mut rules.lead,
                "item.unit" => &mut rules.unit,
                "delimiter" => {
                    tokens.extend(unnamed(capture));
                    continue;
                }
                _ => continue,
            };
            mark(set, language, capture);
        }
        let mut query = ts::Query::new(language, semantic)?;
        let roles: Vec<Role> = query.capture_names().iter().map(|n| own_role(n)).collect();
        if keep(&mut query, &patterns, &roles) {
            rules.items = Some(Items { query, roles });
        }
        rules.delimiters = own_pairs(&tokens);
        rules.derive(language, queries);
        Ok(rules)
    }

    pub fn derived(language: &ts::Language, queries: &Queries<'_>) -> Self {
        let mut rules = Self::empty(language);
        rules.derive(language, queries);
        rules
    }

    fn empty(language: &ts::Language) -> Self {
        let count = language.node_kind_count();
        Self {
            text: vec![false; count],
            lead: vec![false; count],
            unit: vec![false; count],
            delimiters: Vec::new(),
            items: None,
        }
    }

    fn derive(&mut self, language: &ts::Language, queries: &Queries<'_>) {
        if self.delimiters.is_empty() {
            self.delimiters = derived_pairs(language, queries.highlights);
        }
        if !self.text.contains(&true) {
            for pattern in query::patterns(queries.highlights) {
                for capture in &pattern.captures {
                    let string = ["string", "character"].iter().any(|prefix| {
                        capture
                            .name
                            .strip_prefix(prefix)
                            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
                    });
                    if string && capture.root && !pattern.predicated {
                        mark(&mut self.text, language, capture);
                    }
                }
            }
        }
        if self.items.is_none()
            && let Some(source) = queries.locals
            && let Ok(mut query) = ts::Query::new(language, source)
        {
            let patterns = query::patterns(source);
            let roles: Vec<Role> = query
                .capture_names()
                .iter()
                .map(|n| local_role(n))
                .collect();
            if keep(&mut query, &patterns, &roles) {
                self.items = Some(Items { query, roles });
            }
        }
    }

    pub(crate) fn text(&self, kind: u16) -> bool {
        self.text.get(usize::from(kind)).copied().unwrap_or(false)
    }

    pub(crate) fn lead(&self, kind: u16) -> bool {
        self.lead.get(usize::from(kind)).copied().unwrap_or(false)
    }

    pub(crate) fn unit(&self, kind: u16) -> bool {
        self.unit.get(usize::from(kind)).copied().unwrap_or(false)
    }

    pub(crate) fn items(&self, tree: &ts::Tree, src: &str) -> HashMap<usize, Found> {
        let mut found: HashMap<usize, Found> = HashMap::new();
        let Some(items) = &self.items else {
            return found;
        };
        let mut cursor = ts::QueryCursor::new();
        let mut matches = cursor.matches(&items.query, tree.root_node(), src.as_bytes());
        while let Some(m) = matches.next() {
            let (mut node, mut kind, mut name, mut key) = (None, "", None, None);
            for capture in m.captures {
                match items.roles[capture.index as usize] {
                    Role::Item(label) => {
                        node = Some(capture.node);
                        kind = label;
                    }
                    Role::Name => name = Some(capture.node),
                    Role::Key => key = Some(cover(key, span(capture.node))),
                    Role::Definition(label) => {
                        node = capture.node.parent();
                        kind = label;
                        name = Some(capture.node);
                    }
                    Role::None => {}
                }
            }
            let (Some(node), Some(name)) = (node, name) else {
                continue;
            };
            let rank = (m.pattern_index, name.start_byte() as u32);
            if found.get(&node.id()).is_some_and(|f| f.rank <= rank) {
                continue;
            }
            found.insert(
                node.id(),
                Found {
                    kind,
                    name: span(name),
                    key: cover(key, span(name)),
                    rank,
                },
            );
        }
        found
    }
}

fn own_role(name: &str) -> Role {
    match name {
        "item.name" => Role::Name,
        "item.key" => Role::Key,
        "item.lead" | "item.unit" => Role::None,
        _ => match name.strip_prefix("item.") {
            Some(label) if !label.is_empty() => Role::Item(intern(label)),
            _ => Role::None,
        },
    }
}

fn local_role(name: &str) -> Role {
    match name.strip_prefix("local.definition.") {
        Some("function" | "method") => Role::Definition("fn"),
        Some("type" | "enum") => Role::Definition("type"),
        Some("namespace" | "module") => Role::Definition("module"),
        _ => Role::None,
    }
}

fn keep(query: &mut ts::Query, patterns: &[query::Pattern<'_>], roles: &[Role]) -> bool {
    let used: Vec<bool> = (0..query.pattern_count())
        .map(|index| {
            let start = query.start_byte_for_pattern(index);
            patterns.iter().find(|p| p.start == start).is_none_or(|p| {
                p.captures.iter().any(|capture| {
                    query
                        .capture_index_for_name(capture.name)
                        .is_some_and(|i| !matches!(roles[i as usize], Role::None))
                })
            })
        })
        .collect();
    let mut any = false;
    for (index, used) in used.into_iter().enumerate() {
        if used {
            any = true;
        } else {
            query.disable_pattern(index);
        }
    }
    any
}

fn mark(set: &mut [bool], language: &ts::Language, capture: &Capture<'_>) {
    for (kind, named) in &capture.kinds {
        for id in 0..set.len() as u16 {
            if language.node_kind_is_named(id) == *named
                && language.node_kind_for_id(id) == Some(kind.as_str())
            {
                set[usize::from(id)] = true;
            }
        }
    }
}

fn unnamed<'c>(capture: &'c Capture<'_>) -> impl Iterator<Item = &'c str> {
    capture
        .kinds
        .iter()
        .filter(|(_, named)| !named)
        .map(|(kind, _)| kind.as_str())
}

fn own_pairs(tokens: &[&str]) -> Vec<(Box<str>, Box<str>)> {
    let mut pairs = Vec::new();
    for &token in tokens {
        let pair = match PAIRS
            .iter()
            .find(|(open, close)| token == *open || token == *close)
        {
            Some(&(open, close)) if token == open && tokens.contains(&close) => (open, close),
            Some(_) => continue,
            None => (token, token),
        };
        if !pairs
            .iter()
            .any(|(open, _): &(Box<str>, Box<str>)| **open == *pair.0)
        {
            pairs.push((pair.0.into(), pair.1.into()));
        }
    }
    pairs
}

fn derived_pairs(language: &ts::Language, highlights: &str) -> Vec<(Box<str>, Box<str>)> {
    let mut brackets: Vec<String> = Vec::new();
    for pattern in query::patterns(highlights) {
        for capture in &pattern.captures {
            if capture.name == "punctuation.bracket" {
                brackets.extend(unnamed(capture).map(str::to_owned));
            }
        }
    }
    let token = |t: &str| language.id_for_node_kind(t, false) != 0;
    let marked = |t: &str| brackets.iter().any(|b| b == t);
    PAIRS
        .iter()
        .filter(|&&(open, close)| {
            token(open) && token(close) && (open != "<" || marked(open) && marked(close))
        })
        .map(|&(open, close)| (open.into(), close.into()))
        .collect()
}

fn intern(label: &str) -> &'static str {
    static LABELS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut labels = LABELS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(&found) = labels.iter().find(|&&l| l == label) {
        return found;
    }
    let label: &'static str = Box::leak(label.into());
    labels.push(label);
    label
}

fn cover(range: Option<Range<u32>>, other: Range<u32>) -> Range<u32> {
    match range {
        Some(range) => range.start.min(other.start)..range.end.max(other.end),
        None => other,
    }
}
