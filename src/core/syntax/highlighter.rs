use std::path::Path;

use crate::core::error::{DiffyError, Result};
use crate::core::text::{DiffTokenSpan, SyntaxTokenKind};
use vendored_difftastic::{HighlightKind, HighlightSpan};

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

    pub fn highlight(&self, path: &str, source: &str) -> Result<Vec<DiffTokenSpan>> {
        vendored_difftastic::highlight_ranges_for_path(Path::new(path), source)
            .map(|spans| spans.into_iter().map(map_span).collect())
            .map_err(|error| DiffyError::Syntax(error.to_string()))
    }
}

fn map_span(span: HighlightSpan) -> DiffTokenSpan {
    DiffTokenSpan {
        offset: span.offset,
        length: span.length,
        kind: map_kind(span.kind),
        ..DiffTokenSpan::default()
    }
}

fn map_kind(kind: HighlightKind) -> SyntaxTokenKind {
    match kind {
        HighlightKind::Normal => SyntaxTokenKind::Normal,
        HighlightKind::Keyword => SyntaxTokenKind::Keyword,
        HighlightKind::String => SyntaxTokenKind::String,
        HighlightKind::Comment => SyntaxTokenKind::Comment,
        HighlightKind::Number => SyntaxTokenKind::Number,
        HighlightKind::Type => SyntaxTokenKind::Type,
        HighlightKind::Function => SyntaxTokenKind::Function,
        HighlightKind::Operator => SyntaxTokenKind::Operator,
        HighlightKind::Punctuation => SyntaxTokenKind::Punctuation,
        HighlightKind::Variable => SyntaxTokenKind::Variable,
        HighlightKind::Constant => SyntaxTokenKind::Constant,
        HighlightKind::Builtin => SyntaxTokenKind::Builtin,
        HighlightKind::Attribute => SyntaxTokenKind::Attribute,
        HighlightKind::Tag => SyntaxTokenKind::Tag,
        HighlightKind::Property => SyntaxTokenKind::Property,
        HighlightKind::Namespace => SyntaxTokenKind::Namespace,
        HighlightKind::Label => SyntaxTokenKind::Label,
        HighlightKind::Preprocessor => SyntaxTokenKind::Preprocessor,
    }
}
