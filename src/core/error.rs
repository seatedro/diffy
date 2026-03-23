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

pub type Result<T> = std::result::Result<T, DiffyError>;
