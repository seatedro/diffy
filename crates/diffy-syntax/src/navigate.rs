use crate::Symbol;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePos {
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionLookup {
    pub word: Box<str>,
    pub at: SourcePos,
    pub local: Option<SourcePos>,
    pub import: Option<SourcePos>,
    pub symbols: Vec<Symbol>,
}

#[cfg(feature = "engine")]
pub(crate) struct LocalsQuery {
    query: tree_sitter::Query,
    scope: Option<u32>,
    definitions: Vec<Option<bool>>,
}

#[cfg(feature = "engine")]
impl LocalsQuery {
    pub(crate) fn compile(grammar: &tree_sitter::Language, source: &str) -> Option<Self> {
        let query = match tree_sitter::Query::new(grammar, source) {
            Ok(query) => query,
            Err(error) => {
                tracing::warn!(%error, "locals query does not compile");
                return None;
            }
        };
        let definitions = query
            .capture_names()
            .iter()
            .map(|name| {
                name.strip_prefix("local.definition")
                    .map(|kind| kind == ".import")
            })
            .collect();
        Some(Self {
            scope: query.capture_index_for_name("local.scope"),
            definitions,
            query,
        })
    }

    fn resolve(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
        target: tree_sitter::Node<'_>,
        imports: bool,
    ) -> Option<SourcePos> {
        use tree_sitter::StreamingIterator;

        let word = &source[target.byte_range()];
        let mut scopes: Vec<tree_sitter::Node<'_>> = Vec::new();
        let mut definitions: Vec<tree_sitter::Node<'_>> = Vec::new();
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source.as_bytes());
        while let Some(m) = matches.next() {
            for capture in m.captures {
                if Some(capture.index) == self.scope {
                    scopes.push(capture.node);
                } else if self.definitions[capture.index as usize] == Some(imports)
                    && (imports || capture.node.kind_id() == target.kind_id())
                    && source.get(capture.node.byte_range()) == Some(word)
                {
                    definitions.push(capture.node);
                }
            }
        }
        scopes.sort_by_key(|node| (node.start_byte(), std::cmp::Reverse(node.end_byte())));
        scopes.dedup_by_key(|node| node.id());
        let innermost = |start: usize, end: usize, skip: Option<usize>| {
            scopes
                .iter()
                .filter(|scope| {
                    scope.start_byte() <= start
                        && end <= scope.end_byte()
                        && Some(scope.id()) != skip
                })
                .min_by_key(|scope| scope.end_byte() - scope.start_byte())
                .copied()
        };
        let at = target.start_byte();
        let mut visible: Vec<(usize, tree_sitter::Node<'_>)> = Vec::new();
        for definition in definitions {
            let start = definition.start_byte();
            let mut scope = innermost(start, definition.end_byte(), None);
            if let Some(owner) = scope
                && definition.parent().map(|parent| parent.id()) == Some(owner.id())
            {
                scope = innermost(owner.start_byte(), owner.end_byte(), Some(owner.id()));
            }
            let (low, high) = scope.map_or((0, usize::MAX), |s| (s.start_byte(), s.end_byte()));
            if at < low || at > high {
                continue;
            }
            visible.push((high - low, definition));
        }
        let narrowest = visible.iter().map(|(size, _)| *size).min()?;
        let candidates = visible
            .into_iter()
            .filter(|(size, _)| *size == narrowest)
            .map(|(_, node)| node);
        let (before, after): (Vec<_>, Vec<_>) =
            candidates.partition(|node| node.start_byte() <= at);
        let chosen = before
            .into_iter()
            .max_by_key(|node| node.start_byte())
            .or_else(|| after.into_iter().min_by_key(|node| node.start_byte()));
        chosen.map(|node| {
            let point = node.start_position();
            SourcePos {
                line: point.row as u32,
                column: point.column as u32,
            }
        })
    }
}

#[cfg(feature = "engine")]
fn line_start(source: &str, line: u32) -> Option<usize> {
    if line == 0 {
        return Some(0);
    }
    source
        .match_indices('\n')
        .nth(line as usize - 1)
        .map(|(index, _)| index + 1)
}

#[cfg(feature = "engine")]
fn word_node(tree: &tree_sitter::Tree, byte: usize) -> Option<tree_sitter::Node<'_>> {
    let root = tree.root_node();
    let leaf = |at: usize| {
        root.descendant_for_byte_range(at, at)
            .filter(|node| node.is_named() && node.child_count() == 0 && node.start_byte() <= at)
            .filter(|node| at < node.end_byte())
    };
    leaf(byte).or_else(|| byte.checked_sub(1).and_then(leaf))
}

#[cfg(feature = "engine")]
impl crate::GrammarStore {
    pub fn lookup_definition(
        &self,
        language: &crate::LanguageId,
        source: &str,
        line: u32,
        byte: u32,
    ) -> Option<DefinitionLookup> {
        let crate::store::Tag::Ready(grammar) = self.grammar(language) else {
            return None;
        };
        let offset = line_start(source, line)? + byte as usize;
        let tree = grammar.parse_document(source, &|| false)?;
        let node = word_node(&tree, offset.min(source.len()))?;
        let word = source.get(node.byte_range())?.trim();
        if word.is_empty() || word.contains(char::is_whitespace) {
            return None;
        }
        let start = node.start_position();
        let locals = grammar.locals_query();
        let local = locals
            .as_ref()
            .and_then(|query| query.resolve(&tree, source, node, false));
        let import = locals
            .as_ref()
            .and_then(|query| query.resolve(&tree, source, node, true));
        let symbols = grammar
            .tags_query()
            .map(|query| query.symbols(&tree, source))
            .unwrap_or_default();
        Some(DefinitionLookup {
            word: Box::from(word),
            at: SourcePos {
                line: start.row as u32,
                column: start.column as u32,
            },
            local,
            import,
            symbols,
        })
    }
}
