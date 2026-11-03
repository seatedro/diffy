//! Headless file comparison: the chain from a changed file's bytes to a
//! [`DiffDocument`], with no UI, Git, or network dependencies.
//!
//! For each file the caller hands over its status, old and new paths, and
//! old and new bytes ([`FileInput`]). [`diff_files`] picks a language from
//! the path, loads its grammar from the caller's [`GrammarStore`], and runs
//! the semantic diff, falling back to a line diff (and saying why, see
//! [`Fallback`]) when no grammar exists or the semantic diff gives up.
//! Files are diffed on a few worker threads.
//!
//! Budgets: each file's semantic diff gets [`Options::file_budget`]; a
//! request may also set a [`Options::deadline`], after which remaining
//! files get line diffs only. [`Options::cancel`] stops the request
//! between files with [`Error::Cancelled`].
//!
//! With the `serde` feature, [`wire`] projects results into a versioned
//! display format that non-Rust clients render from.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

pub use diffy_diff::{DiffDocument, FileMeta, FileStatus};
pub use diffy_syntax::{GrammarStore, LanguageId};

mod document;
mod file;
mod grammar;
mod line;
mod run;
#[cfg(test)]
mod tests;
#[cfg(feature = "serde")]
pub mod wire;

#[doc(hidden)]
pub use document::{Semantics, clean_spans, document_from_rows};
#[doc(hidden)]
pub use file::{Prepared, prepare};
pub use grammar::{SemanticGrammar, semantic_grammar};
pub use line::{CONTEXT_LINES, binary_document, line_diff};

/// The semantic diff's default time budget per file.
pub const FILE_BUDGET: Duration = Duration::from_millis(400);

/// One changed file, as the caller read it.
#[derive(Debug, Clone, Default)]
pub struct FileInput {
    pub status: FileStatus,
    /// `None` for an added file.
    pub old_path: Option<String>,
    /// `None` for a deleted file.
    pub new_path: Option<String>,
    pub old: Vec<u8>,
    pub new: Vec<u8>,
}

impl FileInput {
    /// The new path, else the old one.
    pub fn display_path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or_default()
    }

    /// The document metadata for this file: paths and status.
    pub fn meta(&self) -> FileMeta {
        FileMeta {
            old_path: self.old_path.as_deref().map(Arc::from),
            new_path: self.new_path.as_deref().map(Arc::from),
            status: self.status,
            ..FileMeta::default()
        }
    }
}

/// Why a file got a line diff instead of a semantic one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    /// No language is known for the path.
    NoGrammar,
    /// The path names a language (this tag) whose grammar the store
    /// cannot load.
    GrammarUnavailable(String),
    /// The semantic diff gave up: size, parse errors, work or time budget.
    Semantic(diffy_semantic::Fallback),
    /// The request's deadline passed before this file was diffed.
    RequestBudget,
}

impl std::fmt::Display for Fallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoGrammar => f.write_str("no grammar for this file"),
            Self::GrammarUnavailable(tag) => write!(f, "{tag} grammar unavailable"),
            Self::Semantic(fallback) => fallback.fmt(f),
            Self::RequestBudget => f.write_str("request time budget exhausted"),
        }
    }
}

/// One file's result. Binary files, added and deleted files, and unchanged
/// files never run the semantic diff and have no fallback.
#[derive(Debug, Clone)]
pub struct FileDiff {
    /// Holds exactly one file.
    pub document: DiffDocument,
    pub fallback: Option<Fallback>,
}

/// Hooks for callers that track progress or account for work per file.
/// Called from worker threads.
pub trait Observer: Sync {
    /// Runs `work`, which diffs the file at `path`; wrap it to time or
    /// attribute the work. The default just runs it.
    fn file(&self, path: &str, work: &mut dyn FnMut()) {
        let _ = path;
        work();
    }

    /// `seen` of `total` files are done.
    fn progress(&self, seen: u32, total: u32) {
        let _ = (seen, total);
    }
}

pub struct Options<'a> {
    /// Grammars for the semantic diff; [`GrammarStore::none`] gives line
    /// diffs only.
    pub grammars: &'a GrammarStore,
    /// Time budget for one file's semantic diff.
    pub file_budget: Duration,
    /// When the whole request must be done. Files started after it get
    /// line diffs ([`Fallback::RequestBudget`]); a file's semantic budget
    /// never runs past it.
    pub deadline: Option<Instant>,
    /// Set to stop the request; checked between files.
    pub cancel: Option<&'a AtomicBool>,
    /// Confirms candidate renames of identifiers, see
    /// [`diffy_semantic::Vet`].
    pub vet: Option<&'a diffy_semantic::Vet<'a>>,
    /// Worker threads; 0 picks from the file count and the machine.
    pub workers: usize,
    pub observer: Option<&'a dyn Observer>,
}

impl<'a> Options<'a> {
    pub fn new(grammars: &'a GrammarStore) -> Self {
        Self {
            grammars,
            file_budget: FILE_BUDGET,
            deadline: None,
            cancel: None,
            vet: None,
            workers: 0,
            observer: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// [`Options::cancel`] was set.
    Cancelled,
    /// A document failed to build; a bug, never expected.
    Build(String),
    /// A worker thread could not start or panicked.
    Worker(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("compare cancelled"),
            Self::Build(error) => write!(f, "diff document failed to build: {error}"),
            Self::Worker(error) => write!(f, "compare worker failed: {error}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Diffs one file on the calling thread.
pub fn diff_file(input: &FileInput, options: &Options<'_>) -> Result<FileDiff> {
    diff_file_with(input, options, &|tag| {
        semantic_grammar(options.grammars, tag)
    })
}

/// [`diff_file`] with grammars from `grammars` instead of
/// [`Options::grammars`], for tools that wait on or swap grammars.
#[doc(hidden)]
pub fn diff_file_with(
    input: &FileInput,
    options: &Options<'_>,
    grammars: &dyn Fn(&LanguageId) -> Option<SemanticGrammar>,
) -> Result<FileDiff> {
    if run::cancelled(options) {
        return Err(Error::Cancelled);
    }
    let diff = run::observed(input, options, grammars)?;
    if let Some(observer) = options.observer {
        observer.progress(1, 1);
    }
    Ok(diff)
}

/// Diffs every file, in parallel when there are several, and returns the
/// results in input order.
pub fn diff_files(inputs: &[FileInput], options: &Options<'_>) -> Result<Vec<FileDiff>> {
    run::diff_all(inputs, options, &|tag| {
        semantic_grammar(options.grammars, tag)
    })
}

/// The language tag the semantic diff uses for `path`: JavaScript is
/// parsed with the TypeScript grammar, JSX with TSX.
pub fn semantic_tag(path: &str) -> Option<LanguageId> {
    let id = LanguageId::from_path(path)?;
    match id.as_str() {
        "js" | "mjs" | "cjs" => LanguageId::from_fence("typescript"),
        "jsx" => LanguageId::from_fence("tsx"),
        _ => Some(id),
    }
}

/// Whether bytes look binary: a NUL in the first 8 KiB, as Git decides.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}
