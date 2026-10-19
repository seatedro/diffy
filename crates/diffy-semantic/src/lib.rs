mod align;
mod config;
mod matcher;
mod syntax;

use std::fmt;

use diffy_diff::SemanticSpan;
use tree_sitter as ts;

pub use config::{Config, config};
pub use tree_sitter::Language;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub spans: [Vec<SemanticSpan>; 2],
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_errors: usize,
    pub max_work: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 1_000_000,
            max_errors: 100,
            max_work: 20_000_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    TooLarge(usize),
    ParseErrors(usize),
    Unparsed,
    Budget,
}

impl fmt::Display for Fallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(bytes) => write!(f, "{bytes} bytes exceeds the semantic byte limit"),
            Self::ParseErrors(count) => write!(f, "{count} parse errors"),
            Self::Unparsed => f.write_str("the parser gave up"),
            Self::Budget => f.write_str("exceeded the semantic work budget"),
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
) -> Result<Vec<Vec<Row>>, Fallback> {
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
    let matching = matcher::run(&old, &new, limits.max_work)?;
    Ok(align::rows(&old, &new, &matching))
}
