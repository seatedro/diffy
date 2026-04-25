//! Phosphor is Diffy's tree-sitter-backed syntax analysis crate.

mod error;
mod language;
pub mod pack;
mod types;

use std::ops::Range;
use std::path::Path;

use carbon::{TextByteRange, TextStore};

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

    pub fn highlight_text_store_language(
        &self,
        language: LanguageId,
        text: &TextStore,
    ) -> Result<Vec<HighlightSpan>> {
        let source = text.as_str().ok_or(PhosphorError::InvalidUtf8)?;
        self.highlight_language(language, source)
    }

    pub fn highlight_text_store_language_ranges(
        &self,
        language: LanguageId,
        text: &TextStore,
        byte_ranges: &[TextByteRange],
    ) -> Result<Vec<HighlightSpan>> {
        let source = text.as_str().ok_or(PhosphorError::InvalidUtf8)?;
        language::highlight_text_ranges(language, source, byte_ranges)
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

    pub fn highlight_text_store_path(
        &self,
        path: &Path,
        text: &TextStore,
    ) -> Result<Vec<HighlightSpan>> {
        let Some(language) = self.guess_language(path) else {
            return Ok(Vec::new());
        };
        if !self.is_parser_available(language) {
            return Ok(Vec::new());
        }
        self.highlight_text_store_language(language, text)
    }

    pub fn highlight_text_store_path_ranges(
        &self,
        path: &Path,
        text: &TextStore,
        byte_ranges: &[TextByteRange],
    ) -> Result<Vec<HighlightSpan>> {
        let Some(language) = self.guess_language(path) else {
            return Ok(Vec::new());
        };
        if !self.is_parser_available(language) {
            return Ok(Vec::new());
        }
        self.highlight_text_store_language_ranges(language, text, byte_ranges)
    }
}

#[cfg(test)]
mod tests {
    use super::Highlighter;
    use carbon::{
        DiffSide, ExpansionState, LineId, ProjectionMode, ProjectionOptions, ProjectionRowKind,
        TextByteRange, TextStore, parse_unified_patch, project_file, projected_row_byte_range,
    };

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

    #[test]
    fn text_store_path_highlighting_uses_carbon_text_coordinates() {
        let highlighter = Highlighter::new();
        let text = TextStore::from_text("export const greeting = \"hello\";\n");
        let range = text.line_range(LineId(0)).unwrap();
        let spans = highlighter
            .highlight_text_store_path_ranges(std::path::Path::new("src/app.ts"), &text, &[range])
            .unwrap();

        assert!(spans.is_empty());
    }

    #[test]
    fn text_store_language_highlighting_rejects_non_utf8() {
        let highlighter = Highlighter::new();
        let text = TextStore::from_bytes([0xff]);
        let error = highlighter
            .highlight_text_store_language(super::LanguageId::Rust, &text)
            .unwrap_err();

        assert!(matches!(error, super::PhosphorError::InvalidUtf8));
    }

    #[test]
    fn text_store_ranges_are_clamped_by_existing_range_highlighter() {
        let highlighter = Highlighter::new();
        let text = TextStore::from_text("plain text\n");
        let spans = highlighter
            .highlight_text_store_path_ranges(
                std::path::Path::new("README.unknown"),
                &text,
                &[TextByteRange {
                    start: 0,
                    len: u32::MAX,
                }],
            )
            .unwrap();

        assert!(spans.is_empty());
    }

    #[test]
    fn carbon_projected_rows_feed_text_store_range_highlighting() {
        let highlighter = Highlighter::new();
        let patch = "\
diff --git a/src/app.rs b/src/app.rs
index 1111111..2222222 100644
--- a/src/app.rs
+++ b/src/app.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
 }
";
        let document = parse_unified_patch(patch).unwrap();
        let file = &document.files[0];
        let new_text = file.side_text(DiffSide::New).unwrap();
        let mut ranges = Vec::new();

        project_file(
            file,
            ProjectionOptions {
                mode: ProjectionMode::Unified,
                include_hunk_headers: false,
                collapsed_context_threshold: u32::MAX,
            },
            &ExpansionState::default(),
            |row| {
                if matches!(
                    row.kind,
                    ProjectionRowKind::Context | ProjectionRowKind::Added
                ) && let Some(range) = projected_row_byte_range(file, &row, DiffSide::New)
                {
                    ranges.push(range);
                }
            },
        );

        assert_eq!(ranges.len(), 3);
        assert_eq!(
            new_text.as_bytes().get(ranges[1].as_usize_range()).unwrap(),
            b"    println!(\"new\");"
        );

        let spans = highlighter
            .highlight_text_store_path_ranges(std::path::Path::new("src/app.rs"), new_text, &ranges)
            .unwrap();
        assert!(spans.is_empty());
    }
}
