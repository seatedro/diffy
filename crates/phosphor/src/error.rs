use thiserror::Error;

use crate::LanguageId;

#[derive(Debug, Error)]
pub enum PhosphorError {
    #[error("Failed to initialize parser for {language}: {message}")]
    InitParser {
        language: LanguageId,
        message: String,
    },
    #[error("Tree-sitter parse failed for {language}")]
    ParseFailed { language: LanguageId },
}

pub type Result<T> = std::result::Result<T, PhosphorError>;
