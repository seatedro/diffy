use std::path::Path;

use crate::core::error::{DiffyError, Result};
use crate::core::text::{DiffTokenSpan, SyntaxTokenKind};
use carbon::TextStore;
use phosphor::{
    HighlightKind, HighlightSpan, Highlighter as PhosphorHighlighter,
    LanguageId as PhosphorLanguageId, TextByteRange,
};

#[derive(Debug)]
pub struct Highlighter {
    inner: PhosphorHighlighter,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Highlighter {
    pub fn new() -> Self {
        Self {
            inner: PhosphorHighlighter::new(),
        }
    }

    pub fn highlight(&self, path: &str, source: &str) -> Result<Vec<DiffTokenSpan>> {
        let language = self.resolve_language(path);
        self.highlight_resolved(language, source)
    }

    pub fn resolve_language(&self, path: &str) -> Option<PhosphorLanguageId> {
        self.inner.guess_language(Path::new(path))
    }

    pub fn highlight_resolved(
        &self,
        language: Option<PhosphorLanguageId>,
        source: &str,
    ) -> Result<Vec<DiffTokenSpan>> {
        let Some(language) = language else {
            return Ok(Vec::new());
        };
        if !self.inner.is_parser_available(language) {
            return Ok(Vec::new());
        }
        self.inner
            .highlight_language(language, source)
            .map(|spans| spans.into_iter().map(map_span).collect())
            .map_err(|error| DiffyError::Syntax(error.to_string()))
    }

    pub fn highlight_text_store_resolved(
        &self,
        language: Option<PhosphorLanguageId>,
        text: &TextStore,
    ) -> Result<Vec<DiffTokenSpan>> {
        let Some(source) = text.as_str() else {
            return Err(DiffyError::Syntax(
                "syntax source is not valid UTF-8".to_owned(),
            ));
        };
        self.highlight_resolved(language, source)
    }

    pub fn highlight_resolved_ranges(
        &self,
        language: Option<PhosphorLanguageId>,
        source: &str,
        byte_ranges: &[TextByteRange],
    ) -> Result<Vec<DiffTokenSpan>> {
        let Some(language) = language else {
            return Ok(Vec::new());
        };
        if !self.inner.is_parser_available(language) {
            return Ok(Vec::new());
        }
        self.inner
            .highlight_language_ranges(language, source, byte_ranges)
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
