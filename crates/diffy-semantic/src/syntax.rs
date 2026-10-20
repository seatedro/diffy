use std::ops::Range;

use tree_sitter as ts;

use crate::config::Config;

pub(crate) const NONE: u32 = u32::MAX;
const MAX_DEPTH: u32 = 512;
const LIST_SEED: u64 = 0x9e37_79b9_7f4a_7c15;
const ATOM_SEED: u64 = 0xc2b2_ae3d_27d4_eb4f;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    Plain,
    Text,
    Comment,
    List,
}

#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub(crate) span: Range<u32>,
    pub(crate) open: Range<u32>,
    pub(crate) close: Range<u32>,
    pub(crate) next: u32,
    pub(crate) hash: u64,
    pub(crate) kind: u16,
    pub(crate) class: Class,
    /// An identifier token, the only kind of token a rename can change.
    pub(crate) name: bool,
}

pub(crate) struct Item {
    pub(crate) node: u32,
    pub(crate) kind: &'static str,
    pub(crate) name: Range<u32>,
    pub(crate) key: Range<u32>,
    pub(crate) unit: u32,
    pub(crate) lead: u32,
}

pub(crate) struct Tree<'a> {
    pub(crate) src: &'a str,
    pub(crate) nodes: Vec<Node>,
    pub(crate) items: Vec<Item>,
    pub(crate) units: Vec<u32>,
    pub(crate) errors: usize,
}

impl<'a> Tree<'a> {
    pub(crate) fn lower(tree: &ts::Tree, src: &'a str, config: &Config) -> Self {
        let mut out = Self {
            src,
            nodes: Vec::new(),
            items: Vec::new(),
            units: Vec::new(),
            errors: 0,
        };
        let root = tree.root_node();
        out.errors += usize::from(root.is_error());
        let at = out.open_list(root.kind_id(), span(root), 0..0, 0..0);
        let mut cursor = root.walk();
        let children: Vec<_> = root.children(&mut cursor).collect();
        for child in children {
            out.node(child, config, 1);
        }
        out.close_list(at);
        for item in &out.items {
            out.units.extend([item.node, item.unit]);
            let mut at = item.lead;
            while at < item.unit {
                out.units.push(at);
                at = out.nodes[at as usize].next;
            }
        }
        out.units.sort_unstable();
        out.units.dedup();
        out
    }

    pub(crate) fn text(&self, range: Range<u32>) -> &'a str {
        &self.src[range.start as usize..range.end as usize]
    }

    pub(crate) fn children(&self, list: u32) -> Vec<u32> {
        self.children_until(list, self.nodes[list as usize].next)
    }

    pub(crate) fn is_list(&self, node: u32) -> bool {
        self.nodes[node as usize].class == Class::List
    }

    fn node(&mut self, node: ts::Node<'_>, config: &Config, depth: u32) {
        if node.is_missing() {
            self.errors += 1;
            return;
        }
        if node.is_error() {
            self.errors += 1;
        }
        if node.start_byte() == node.end_byte() || node.kind() == "\n" {
            return;
        }
        let comment = node.is_extra() || node.kind().contains("comment");
        let text = config.atoms.contains(&node.kind());
        if node.child_count() == 0 || comment || text || depth >= MAX_DEPTH {
            let class = if comment {
                Class::Comment
            } else if text {
                Class::Text
            } else {
                Class::Plain
            };
            let name =
                class == Class::Plain && node.is_named() && node.kind().ends_with("identifier");
            self.atom(node.kind_id(), span(node), class, name);
            return;
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        let delims = self.delimiters(&children, config);
        self.note_item(node);
        let Some((i, j)) = delims else {
            let at = self.open_list(node.kind_id(), span(node), 0..0, 0..0);
            for child in children {
                self.node(child, config, depth + 1);
            }
            self.close_list(at);
            return;
        };
        let (open, close) = (span(children[i]), span(children[j]));
        let outer = (i > 0 || j + 1 < children.len())
            .then(|| self.open_list(node.kind_id(), span(node), 0..0, 0..0));
        for &child in &children[..i] {
            self.node(child, config, depth + 1);
        }
        let inner = self.open_list(node.kind_id(), open.start..close.end, open, close);
        for &child in &children[i + 1..j] {
            self.node(child, config, depth + 1);
        }
        self.close_list(inner);
        for &child in &children[j + 1..] {
            self.node(child, config, depth + 1);
        }
        if let Some(outer) = outer {
            self.close_list(outer);
        }
    }

    fn note_item(&mut self, node: ts::Node<'_>) {
        let Some(kind) = item_kind(node.kind()) else {
            return;
        };
        if node.kind().ends_with("_specifier") && node.child_by_field_name("body").is_none() {
            return;
        }
        let (kind, name) = match node.child_by_field_name("name") {
            Some(name) => (kind, Some(name)),
            None if node.kind() == "impl_item" => (kind, node.child_by_field_name("type")),
            None if node.kind() == "function_definition" => ("fn", declared_name(node)),
            None => (kind, None),
        };
        let Some(name) = name else {
            return;
        };
        let node_at = self.nodes.len() as u32;
        let key = match node.child_by_field_name("trait") {
            Some(t) if node.kind() == "impl_item" => t.start_byte() as u32..name.end_byte() as u32,
            _ => span(name),
        };
        let wrapper = node
            .parent()
            .filter(|p| p.kind() == "decorated_definition")
            .and_then(|p| self.lowered(span(p)));
        let unit = wrapper.unwrap_or(node_at);
        let outer = match wrapper {
            Some(_) => node.parent(),
            None => Some(node),
        };
        let mut first = None;
        let mut at = outer.and_then(|n| n.prev_sibling());
        while let Some(sibling) = at.filter(|s| matches!(s.kind(), "attribute_item" | "decorator"))
        {
            first = Some(sibling);
            at = sibling.prev_sibling();
        }
        let lead = first.and_then(|f| self.lowered(span(f))).unwrap_or(unit);
        self.items.push(Item {
            node: node_at,
            kind,
            name: span(name),
            key,
            unit,
            lead,
        });
    }

    fn lowered(&self, range: Range<u32>) -> Option<u32> {
        let mut found = None;
        for (k, n) in self.nodes.iter().enumerate().rev() {
            if n.span.start < range.start {
                break;
            }
            if n.span == range {
                found = Some(k as u32);
            }
        }
        found
    }

    fn delimiters(&self, children: &[ts::Node<'_>], config: &Config) -> Option<(usize, usize)> {
        let token = |node: &ts::Node<'_>| {
            (node.child_count() <= 1 && !node.is_extra()).then(|| self.text(span(*node)))
        };
        for (i, child) in children.iter().enumerate() {
            let Some(text) = token(child) else {
                continue;
            };
            for &(open, close) in config.delimiters {
                if text != open {
                    continue;
                }
                if let Some(j) = children[i + 1..]
                    .iter()
                    .position(|c| token(c) == Some(close))
                {
                    return Some((i, i + 1 + j));
                }
            }
        }
        None
    }

    fn atom(&mut self, kind: u16, span: Range<u32>, class: Class, name: bool) {
        let text = self.text(span.clone());
        let seed = mix(mix(ATOM_SEED, u64::from(kind)), class as u64);
        let hash = if class == Class::Comment {
            text.lines()
                .fold(seed, |h, line| mix(h, hash_bytes(line.trim().as_bytes())))
        } else {
            mix(seed, hash_bytes(text.as_bytes()))
        };
        let next = self.nodes.len() as u32 + 1;
        self.nodes.push(Node {
            span,
            open: 0..0,
            close: 0..0,
            next,
            hash,
            kind,
            class,
            name,
        });
    }

    fn open_list(
        &mut self,
        kind: u16,
        span: Range<u32>,
        open: Range<u32>,
        close: Range<u32>,
    ) -> u32 {
        let at = self.nodes.len() as u32;
        self.nodes.push(Node {
            span,
            open,
            close,
            next: NONE,
            hash: 0,
            kind,
            class: Class::List,
            name: false,
        });
        at
    }

    fn close_list(&mut self, at: u32) {
        let node = &self.nodes[at as usize];
        let mut hash = mix(
            mix(LIST_SEED, u64::from(node.kind)),
            hash_bytes(self.text(node.open.clone()).as_bytes()),
        );
        hash = mix(hash, hash_bytes(self.text(node.close.clone()).as_bytes()));
        for child in self.children_until(at, self.nodes.len() as u32) {
            hash = mix(hash, self.nodes[child as usize].hash);
        }
        let next = self.nodes.len() as u32;
        let node = &mut self.nodes[at as usize];
        node.hash = hash;
        node.next = next;
    }

    fn children_until(&self, list: u32, end: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut at = list + 1;
        while at < end {
            out.push(at);
            at = self.nodes[at as usize].next;
        }
        out
    }
}

fn item_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "function_item" | "function_signature_item" => "fn",
        "struct_item" | "struct_specifier" => "struct",
        "enum_item" | "enum_declaration" | "enum_specifier" => "enum",
        "union_item" => "union",
        "trait_item" => "trait",
        "impl_item" => "impl",
        "mod_item" => "mod",
        "const_item" => "const",
        "static_item" => "static",
        "type_item" | "type_alias_declaration" | "type_spec" => "type",
        "macro_definition" => "macro",
        "function_declaration" | "generator_function_declaration" => "function",
        "method_definition" | "method_declaration" => "method",
        "class_declaration"
        | "abstract_class_declaration"
        | "class_definition"
        | "class_specifier" => "class",
        "interface_declaration" => "interface",
        "function_definition" => "def",
        _ => return None,
    })
}

fn declared_name(node: ts::Node<'_>) -> Option<ts::Node<'_>> {
    let mut at = node.child_by_field_name("declarator")?;
    while let Some(inner) = at.child_by_field_name("declarator") {
        at = inner;
    }
    at.kind().ends_with("identifier").then_some(at)
}

fn span(node: ts::Node<'_>) -> Range<u32> {
    node.start_byte() as u32..node.end_byte() as u32
}

pub(crate) fn mix(hash: u64, word: u64) -> u64 {
    (hash.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95)
}

pub(crate) fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hash = bytes.len() as u64;
    let (chunks, rest) = bytes.as_chunks::<8>();
    for chunk in chunks {
        hash = mix(hash, u64::from_le_bytes(*chunk));
    }
    let mut tail = [0; 8];
    tail[..rest.len()].copy_from_slice(rest);
    mix(hash, u64::from_le_bytes(tail))
}
