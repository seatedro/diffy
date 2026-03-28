use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

use crate::core::error::{DiffyError, Result};
use crate::core::syntax::language_registry::Grammar;
use crate::core::text::{DiffTokenSpan, SyntaxTokenKind};

#[derive(Debug)]
pub struct Highlighter;

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Highlighter {
    pub fn new() -> Self {
        Self
    }

    pub fn highlight(&self, grammar: &Grammar, source: &str) -> Result<Vec<DiffTokenSpan>> {
        if source.is_empty() || grammar.highlights_query.is_empty() {
            return Ok(Vec::new());
        }

        let mut parser = Parser::new();
        parser
            .set_language(&grammar.language)
            .map_err(|error| DiffyError::Syntax(error.to_string()))?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| DiffyError::Syntax("tree-sitter parse failed".to_owned()))?;

        let query = Query::new(&grammar.language, grammar.highlights_query)
            .map_err(|error| DiffyError::Syntax(error.to_string()))?;
        let capture_kinds: Vec<_> = query
            .capture_names()
            .iter()
            .map(|name| capture_name_to_kind(name))
            .collect();

        let root = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut captures = cursor.captures(&query, root, source.as_bytes());
        let mut raw_spans = Vec::new();

        while let Some((query_match, capture_index)) = captures.next() {
            let capture = query_match.captures[*capture_index];
            let kind = capture_kinds
                .get(capture.index as usize)
                .copied()
                .unwrap_or_default();
            if kind == SyntaxTokenKind::Normal {
                continue;
            }
            let node = capture.node;
            let start = node.start_byte();
            let end = node.end_byte();
            if end > start {
                raw_spans.push((start, end, kind, query_match.pattern_index));
            }
        }

        raw_spans.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| right.3.cmp(&left.3)));
        let mut covered = 0usize;
        let mut result = Vec::with_capacity(raw_spans.len());
        for (start, end, kind, _) in raw_spans {
            if start < covered {
                continue;
            }
            result.push(DiffTokenSpan {
                offset: start as u32,
                length: (end - start) as u32,
                kind,
            });
            covered = end;
        }
        Ok(result)
    }
}

fn capture_name_to_kind(name: &str) -> SyntaxTokenKind {
    if name.starts_with("keyword") {
        SyntaxTokenKind::Keyword
    } else if name.starts_with("string") || name.starts_with("escape") {
        SyntaxTokenKind::String
    } else if name.starts_with("comment") {
        SyntaxTokenKind::Comment
    } else if name.starts_with("number") {
        SyntaxTokenKind::Number
    } else if name.starts_with("type") || name.starts_with("constructor") {
        SyntaxTokenKind::Type
    } else if name.starts_with("function") {
        SyntaxTokenKind::Function
    } else if name.starts_with("operator") {
        SyntaxTokenKind::Operator
    } else if name.starts_with("punctuation") {
        SyntaxTokenKind::Punctuation
    } else if name.starts_with("variable") {
        SyntaxTokenKind::Variable
    } else if name.starts_with("constant") || name.starts_with("boolean") {
        SyntaxTokenKind::Constant
    } else if name.starts_with("builtin") {
        SyntaxTokenKind::Builtin
    } else if name.starts_with("attribute") {
        SyntaxTokenKind::Attribute
    } else if name.starts_with("tag") {
        SyntaxTokenKind::Tag
    } else if name.starts_with("property") {
        SyntaxTokenKind::Property
    } else if name.starts_with("module") || name.starts_with("namespace") {
        SyntaxTokenKind::Namespace
    } else if name.starts_with("label") {
        SyntaxTokenKind::Label
    } else if name.starts_with("preproc") {
        SyntaxTokenKind::Preprocessor
    } else {
        SyntaxTokenKind::Normal
    }
}
