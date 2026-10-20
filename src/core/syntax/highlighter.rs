use std::ops::Range;

use diffy_syntax::{GrammarStore, HighlightKind, HighlightSpan, LanguageId};

use crate::core::text::{DiffTokenSpan, SyntaxTokenKind};

#[derive(Debug, Clone, Default)]
pub struct Highlighter {
    store: GrammarStore,
}

impl Highlighter {
    pub fn new(store: GrammarStore) -> Self {
        Self { store }
    }

    pub fn resolve_language(path: &str) -> Option<LanguageId> {
        LanguageId::from_path(path)
    }

    pub fn highlight(&self, path: &str, source: &str) -> Vec<DiffTokenSpan> {
        match Self::resolve_language(path) {
            Some(language) => self.highlight_language(&language, source),
            None => Vec::new(),
        }
    }

    pub fn highlight_language(&self, language: &LanguageId, source: &str) -> Vec<DiffTokenSpan> {
        diffy_syntax::highlight(&self.store, language, source)
            .into_iter()
            .map(token_span)
            .collect()
    }

    pub fn highlight_fragments(
        &self,
        language: &LanguageId,
        source: &str,
        fragments: &[Range<u32>],
    ) -> Vec<DiffTokenSpan> {
        diffy_syntax::highlight_fragments(&self.store, language, source, fragments)
            .into_iter()
            .map(token_span)
            .collect()
    }
}

pub fn token_span(span: HighlightSpan) -> DiffTokenSpan {
    DiffTokenSpan {
        offset: span.offset,
        length: span.length,
        kind: token_kind(span.kind),
        ..DiffTokenSpan::default()
    }
}

pub fn token_kind(kind: HighlightKind) -> SyntaxTokenKind {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::syntax::grammar::Grammars;

    #[test]
    fn typescript_imports_map_to_product_token_kinds() {
        let Some(store) = diffy_syntax::testing::store_with("typescript") else {
            return;
        };
        let highlighter = Highlighter::new(store);
        let source = "import { x } from \"y\";\n";
        let spans = highlighter.highlight("src/app.ts", source);
        let text = |kind| {
            spans
                .iter()
                .filter(|span| span.kind == kind)
                .map(|span| &source[span.offset as usize..(span.offset + span.length) as usize])
                .collect::<Vec<_>>()
        };

        assert!(text(SyntaxTokenKind::Keyword).contains(&"import"));
        assert_eq!(text(SyntaxTokenKind::String), vec!["\"y\""]);
    }

    #[test]
    fn shared_workers_answer_on_their_own_handles() {
        let Some(store) = diffy_syntax::testing::store_with("rust") else {
            return;
        };
        let grammars = Grammars::with_store(store);
        let (viewport, review) = (grammars.worker(), grammars.worker());
        let source: std::sync::Arc<str> = "fn main() {}\n".into();

        viewport.request(1, 1, LanguageId::from_fence("rs").unwrap(), source.clone());
        review.request(1, 7, LanguageId::from_fence("rs").unwrap(), source);
        let (first, second) = (viewport.recv().unwrap(), review.recv().unwrap());

        assert_eq!((first.generation, second.generation), (1, 7));
        assert!(
            first
                .spans
                .iter()
                .any(|span| span.kind == HighlightKind::Keyword)
        );
    }
}
