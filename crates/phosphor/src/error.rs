use thiserror::Error;

use crate::LanguageId;

#[derive(Debug, Error)]
pub enum PhosphorError {
    #[error("Failed to initialize parser for {language}: {message}")]
    InitParser {
        language: LanguageId,
        message: String,
    },
    #[error("Failed to load parser pack for {language}: {message}")]
    LoadParserPack {
        language: LanguageId,
        message: String,
    },
    #[error("Invalid highlight query for {language}: {message}")]
    InvalidHighlightQuery {
        language: LanguageId,
        message: String,
    },
    #[error("No installed tree-sitter parser for {language}")]
    MissingParser { language: LanguageId },
    #[error("Tree-sitter parse failed for {language}")]
    ParseFailed { language: LanguageId },
    #[error("Carbon text store is not valid UTF-8")]
    InvalidUtf8,
}

pub type Result<T> = std::result::Result<T, PhosphorError>;
