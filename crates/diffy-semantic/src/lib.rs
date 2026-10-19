mod align;
mod config;
mod matcher;
mod syntax;

use std::fmt;
use std::time::{Duration, Instant};

use diffy_diff::SemanticSpan;
use tree_sitter as ts;

pub use config::{Config, config};
pub use diffy_diff::{ItemChange, OutlineItem, Rename};
pub use tree_sitter::Language;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub spans: [Vec<SemanticSpan>; 2],
}

#[derive(Debug, Clone, Default)]
pub struct SemanticDiff {
    pub chunks: Vec<Vec<Row>>,
    pub outline: Vec<OutlineItem>,
    pub renames: Vec<Rename>,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_errors: usize,
    pub max_work: u64,
    pub max_time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 1_000_000,
            max_errors: 100,
            max_work: 20_000_000,
            max_time: Duration::from_secs(1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    TooLarge(usize),
    ParseErrors(usize),
    Unparsed,
    Budget,
    Timeout,
}

impl fmt::Display for Fallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(bytes) => write!(f, "{bytes} bytes exceeds the semantic byte limit"),
            Self::ParseErrors(count) => write!(f, "{count} parse errors"),
            Self::Unparsed => f.write_str("the parser gave up"),
            Self::Budget => f.write_str("exceeded the semantic work budget"),
            Self::Timeout => f.write_str("exceeded the semantic time budget"),
        }
    }
}

impl std::error::Error for Fallback {}

pub fn diff(
    language: &ts::Language,
    config: &Config,
    old: &str,
    new: &str,
    limits: &Limits,
) -> Result<SemanticDiff, Fallback> {
    let deadline = Instant::now() + limits.max_time;
    let bytes = old.len().max(new.len());
    if bytes > limits.max_bytes {
        return Err(Fallback::TooLarge(bytes));
    }
    let mut parser = ts::Parser::new();
    parser
        .set_language(language)
        .map_err(|_| Fallback::Unparsed)?;
    let old_tree = parser.parse(old, None).ok_or(Fallback::Unparsed)?;
    let new_tree = parser.parse(new, None).ok_or(Fallback::Unparsed)?;
    let old = syntax::Tree::lower(&old_tree, old, config);
    let new = syntax::Tree::lower(&new_tree, new, config);
    let errors = old.errors + new.errors;
    if errors > limits.max_errors {
        return Err(Fallback::ParseErrors(errors));
    }
    let matching = matcher::run(&old, &new, limits.max_work, deadline)?;
    let (chunks, outline, renames) = align::rows(&old, &new, &matching);
    Ok(SemanticDiff {
        chunks,
        outline,
        renames,
    })
}
