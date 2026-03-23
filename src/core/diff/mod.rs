pub mod types;
pub mod unified_parser;
pub mod word_diff;

pub use types::{DiffDocument, DiffLine, FileDiff, Hunk, LineKind};
pub use unified_parser::{parse, parse_into};
pub use word_diff::compute_word_diff;
