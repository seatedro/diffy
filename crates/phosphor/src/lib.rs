//! Phosphor is Diffy's tree-sitter-backed syntax analysis crate.

mod error;
mod language;
mod types;

use std::path::Path;

pub use error::{PhosphorError, Result};
pub use types::{HighlightKind, HighlightSpan, LanguageId};

#[derive(Debug, Default, Clone, Copy)]
pub struct Highlighter;

impl Highlighter {
    pub const fn new() -> Self {
        Self
    }

    pub fn guess_language(&self, path: &Path) -> Option<LanguageId> {
        language::guess_language(path)
    }

    pub fn highlight_language(
        &self,
        language: LanguageId,
        source: &str,
    ) -> Result<Vec<HighlightSpan>> {
        language::highlight(language, source)
    }

    pub fn highlight_path(&self, path: &Path, source: &str) -> Result<Vec<HighlightSpan>> {
        let Some(language) = self.guess_language(path) else {
            return Ok(Vec::new());
        };
        self.highlight_language(language, source)
    }
}

#[cfg(test)]
mod tests {
    use super::{HighlightKind, Highlighter};

    #[test]
    fn rust_highlighting_returns_semantic_tokens() {
        let highlighter = Highlighter::new();
        let spans = highlighter
            .highlight_path(
                std::path::Path::new("src/lib.rs"),
                "pub fn greet(name: &str) -> usize { name.len() }\n",
            )
            .unwrap();

        assert!(spans.iter().any(|span| span.kind == HighlightKind::Keyword));
        assert!(
            spans
                .iter()
                .any(|span| span.kind == HighlightKind::Function)
        );
    }

    #[test]
    fn typescript_highlighting_returns_string_tokens() {
        let highlighter = Highlighter::new();
        let spans = highlighter
            .highlight_path(
                std::path::Path::new("src/app.ts"),
                "export const greeting = \"hello\";\n",
            )
            .unwrap();

        assert!(spans.iter().any(|span| span.kind == HighlightKind::Keyword));
        assert!(spans.iter().any(|span| span.kind == HighlightKind::String));
    }

    #[test]
    fn unsupported_extensions_return_no_tokens() {
        let highlighter = Highlighter::new();
        let spans = highlighter
            .highlight_path(std::path::Path::new("README.unknown"), "plain text\n")
            .unwrap();

        assert!(spans.is_empty());
    }
}
