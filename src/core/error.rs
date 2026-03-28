use thiserror::Error;

#[derive(Error, Debug)]
pub enum DiffyError {
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Syntax error: {0}")]
    Syntax(String),
    #[error("{0}")]
    General(String),
}

impl From<ureq::Error> for DiffyError {
    fn from(value: ureq::Error) -> Self {
        Self::Http(value.to_string())
    }
}

impl From<tree_sitter::LanguageError> for DiffyError {
    fn from(value: tree_sitter::LanguageError) -> Self {
        Self::Syntax(value.to_string())
    }
}

impl From<tree_sitter::QueryError> for DiffyError {
    fn from(value: tree_sitter::QueryError) -> Self {
        Self::Syntax(value.to_string())
    }
}

pub type Result<T> = std::result::Result<T, DiffyError>;
