#[cfg(feature = "engine")]
use std::sync::Arc;

#[cfg(feature = "engine")]
use crate::{GrammarStore, LanguageId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Interface,
    Module,
    Macro,
    Constant,
    Type,
    Field,
    Other,
}

impl SymbolKind {
    pub const ALL: [Self; 10] = [
        Self::Function,
        Self::Method,
        Self::Class,
        Self::Interface,
        Self::Module,
        Self::Macro,
        Self::Constant,
        Self::Type,
        Self::Field,
        Self::Other,
    ];

    pub fn from_u8(value: u8) -> Self {
        Self::ALL
            .get(usize::from(value))
            .copied()
            .unwrap_or(Self::Other)
    }

    #[cfg(feature = "engine")]
    fn from_capture(name: &str) -> Option<Self> {
        let kind = name.strip_prefix("definition.")?;
        Some(match kind {
            "function" => Self::Function,
            "method" => Self::Method,
            "class" | "struct" | "enum" | "union" => Self::Class,
            "interface" | "trait" => Self::Interface,
            "module" | "namespace" | "package" => Self::Module,
            "macro" => Self::Macro,
            "constant" | "const" => Self::Constant,
            "type" | "typedef" => Self::Type,
            "field" | "property" => Self::Field,
            _ => Self::Other,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: Box<str>,
    pub kind: SymbolKind,
    pub line: u32,
    pub column: u32,
    pub start_line: u32,
    pub end_line: u32,
}

#[cfg(feature = "engine")]
pub(crate) struct TagsQuery {
    query: tree_sitter::Query,
    kinds: Vec<Option<SymbolKind>>,
    name: u32,
    fingerprint: u64,
}

#[cfg(feature = "engine")]
impl TagsQuery {
    pub(crate) fn compile(
        grammar: &tree_sitter::Language,
        source: &str,
        library: Option<&str>,
    ) -> Option<Self> {
        let query = match tree_sitter::Query::new(grammar, source) {
            Ok(query) => query,
            Err(error) => {
                tracing::warn!(%error, "tags query does not compile");
                return None;
            }
        };
        let kinds = query
            .capture_names()
            .iter()
            .map(|name| SymbolKind::from_capture(name))
            .collect();
        Some(Self {
            name: query.capture_index_for_name("name")?,
            kinds,
            fingerprint: fnv1a(&[library.unwrap_or_default().as_bytes(), source.as_bytes()]),
            query,
        })
    }

    pub(crate) fn symbols(&self, tree: &tree_sitter::Tree, source: &str) -> Vec<Symbol> {
        use tree_sitter::StreamingIterator;

        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source.as_bytes());
        let mut found: Vec<(usize, usize, Symbol)> = Vec::new();
        while let Some(m) = matches.next() {
            let mut name = None;
            let mut definition = None;
            for capture in m.captures {
                if capture.index == self.name {
                    name.get_or_insert(capture.node);
                } else if let Some(kind) = self.kinds[capture.index as usize] {
                    definition.get_or_insert((kind, capture.node));
                }
            }
            let (Some(name), Some((kind, node))) = (name, definition) else {
                continue;
            };
            let Some(text) = source.get(name.byte_range()).map(str::trim) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let at = name.start_position();
            found.push((
                name.start_byte(),
                m.pattern_index,
                Symbol {
                    name: Box::from(text),
                    kind,
                    line: at.row as u32,
                    column: at.column as u32,
                    start_line: node.start_position().row as u32,
                    end_line: node.end_position().row as u32,
                },
            ));
        }
        found.sort_by_key(|(start, pattern, _)| (*start, *pattern));
        found.dedup_by_key(|(start, _, _)| *start);
        found.into_iter().map(|(_, _, symbol)| symbol).collect()
    }
}

#[cfg(feature = "engine")]
fn fnv1a(parts: &[&[u8]]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in *part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(feature = "engine")]
#[derive(Clone)]
pub struct Tagger {
    grammar: Arc<crate::engine::Grammar>,
    query: Arc<TagsQuery>,
}

#[cfg(feature = "engine")]
impl std::fmt::Debug for Tagger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tagger")
            .field("language", &self.grammar.name())
            .field("fingerprint", &self.query.fingerprint)
            .finish()
    }
}

#[cfg(feature = "engine")]
pub enum TaggerLookup {
    Ready(Tagger),
    Pending,
    Unsupported,
}

#[cfg(feature = "engine")]
impl Tagger {
    pub fn fingerprint(&self) -> u64 {
        self.query.fingerprint
    }

    pub fn language(&self) -> &str {
        self.grammar.name().unwrap_or_default()
    }

    pub fn symbols(&self, source: &str, cancelled: &dyn Fn() -> bool) -> Vec<Symbol> {
        match self.grammar.parse_document(source, cancelled) {
            Some(tree) => self.query.symbols(&tree, source),
            None => Vec::new(),
        }
    }
}

#[cfg(feature = "engine")]
impl GrammarStore {
    pub fn tagger(&self, language: &LanguageId) -> TaggerLookup {
        if !self.ships_tags(language) {
            return TaggerLookup::Unsupported;
        }
        match self.grammar(language) {
            crate::store::Tag::Ready(grammar) => match grammar.tags_query() {
                Some(query) => TaggerLookup::Ready(Tagger { grammar, query }),
                None => TaggerLookup::Unsupported,
            },
            crate::store::Tag::Pending => TaggerLookup::Pending,
            crate::store::Tag::Unavailable => TaggerLookup::Unsupported,
        }
    }
}
