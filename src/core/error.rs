use std::fmt;
use std::path::PathBuf;

use thiserror::Error;

/// VCS backend that produced a [`DiffyError::Vcs`] failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsBackendKind {
    Git,
    Jj,
}

impl fmt::Display for VcsBackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Git => "git",
            Self::Jj => "jj",
        })
    }
}

#[derive(Error, Debug)]
pub enum DiffyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// A VCS operation failed. `recoverable` is true when retrying after
    /// fixing repository state can succeed, false for operations the backend
    /// cannot perform at all (or invariant violations).
    #[error("{backend} {op} failed: {details}")]
    Vcs {
        backend: VcsBackendKind,
        op: String,
        details: String,
        recoverable: bool,
    },
    /// A network request failed. `retryable` marks transient transport or
    /// server failures where retrying the same request can succeed.
    #[error("{details}")]
    Network { details: String, retryable: bool },
    /// An operation required authentication that is missing or was rejected.
    #[error("{details}")]
    Auth { details: String },
    /// The OS denied access to a path.
    #[error("permission denied: cannot {op} {}", path.display())]
    Permission { path: PathBuf, op: String },
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Syntax error: {0}")]
    Syntax(String),
    /// Fallback for failures that have not been classified yet.
    #[error("{0}")]
    General(String),
}

impl DiffyError {
    /// VCS failure that retrying after fixing repository state may resolve.
    pub fn vcs(backend: VcsBackendKind, op: impl Into<String>, details: impl Into<String>) -> Self {
        Self::Vcs {
            backend,
            op: op.into(),
            details: details.into(),
            recoverable: true,
        }
    }

    /// VCS failure that retrying will not fix (unsupported operation or
    /// broken invariant).
    pub fn vcs_fatal(
        backend: VcsBackendKind,
        op: impl Into<String>,
        details: impl Into<String>,
    ) -> Self {
        Self::Vcs {
            backend,
            op: op.into(),
            details: details.into(),
            recoverable: false,
        }
    }

    /// Transient network failure worth retrying (connection, timeout, 5xx).
    pub fn network(details: impl Into<String>) -> Self {
        Self::Network {
            details: details.into(),
            retryable: true,
        }
    }

    /// Network failure that retrying alone will not fix (4xx, protocol).
    pub fn network_fatal(details: impl Into<String>) -> Self {
        Self::Network {
            details: details.into(),
            retryable: false,
        }
    }

    pub fn auth(details: impl Into<String>) -> Self {
        Self::Auth {
            details: details.into(),
        }
    }

    /// Classify an IO error against the path/op it touched so permission
    /// failures surface distinctly from generic IO failures.
    pub fn io(path: impl Into<PathBuf>, op: &str, error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            Self::Permission {
                path: path.into(),
                op: op.to_owned(),
            }
        } else {
            Self::Io(error)
        }
    }

    /// True when retrying the same operation unchanged can plausibly succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Network {
                retryable: true,
                ..
            }
        )
    }

    /// Message for user-facing surfaces (toasts, error states). Appends a
    /// retry hint for transient failures.
    pub fn user_message(&self) -> String {
        let base = self.to_string();
        if self.is_retryable() {
            format!("{base} Check your connection and retry.")
        } else {
            base
        }
    }
}

pub type Result<T> = std::result::Result<T, DiffyError>;
