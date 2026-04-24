//! Phosphor is Diffy's tree-sitter-backed syntax analysis crate.

mod error;
mod language;
pub mod pack;
mod types;

use std::ops::Range;
use std::path::Path;

pub use error::{PhosphorError, Result};
pub use pack::PackInstaller;
pub use types::{HighlightKind, HighlightSpan, LanguageId, LanguageMetadata};

#[derive(Debug, Default, Clone, Copy)]
pub struct Highlighter;

impl Highlighter {
    pub const fn new() -> Self {
        Self
    }

    pub fn guess_language(&self, path: &Path) -> Option<LanguageId> {
        language::guess_language(path)
    }

    pub fn languages(&self) -> &'static [LanguageMetadata] {
        language::languages()
    }

    pub fn common_languages(&self) -> impl Iterator<Item = LanguageId> + 'static {
        language::common_languages()
    }

    pub fn is_parser_available(&self, language: LanguageId) -> bool {
        language::is_parser_available(language)
    }

    pub fn highlight_language(
        &self,
        language: LanguageId,
        source: &str,
    ) -> Result<Vec<HighlightSpan>> {
        language::highlight(language, source)
    }

    pub fn highlight_language_ranges(
        &self,
        language: LanguageId,
        source: &str,
        byte_ranges: &[Range<usize>],
    ) -> Result<Vec<HighlightSpan>> {
        language::highlight_ranges(language, source, byte_ranges)
    }

    pub fn highlight_path(&self, path: &Path, source: &str) -> Result<Vec<HighlightSpan>> {
        let Some(language) = self.guess_language(path) else {
            return Ok(Vec::new());
        };
        if !self.is_parser_available(language) {
            return Ok(Vec::new());
        }
        self.highlight_language(language, source)
    }
}

#[cfg(test)]
mod tests {
    use super::Highlighter;

    #[test]
    fn known_paths_without_installed_packs_return_no_tokens() {
        let highlighter = Highlighter::new();
        let spans = highlighter
            .highlight_path(
                std::path::Path::new("data.json"),
                "{ \"name\": \"Diffy\", \"fast\": true }\n",
            )
            .unwrap();

        assert!(spans.is_empty());
    }

    #[test]
    fn common_registry_includes_unbundled_languages() {
        let highlighter = Highlighter::new();

        assert_eq!(
            highlighter.guess_language(std::path::Path::new("src/app.ts")),
            Some(super::LanguageId::TypeScript)
        );
        assert!(
            highlighter
                .common_languages()
                .any(|language| language == super::LanguageId::TypeScript)
        );
        assert!(!highlighter.is_parser_available(super::LanguageId::TypeScript));
    }

    #[test]
    fn missing_packs_return_no_tokens() {
        let highlighter = Highlighter::new();
        let spans = highlighter
            .highlight_path(
                std::path::Path::new("src/app.ts"),
                "export const greeting = \"hello\";\n",
            )
            .unwrap();

        assert!(spans.is_empty());
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
