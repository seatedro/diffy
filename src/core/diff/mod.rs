pub mod types;
pub mod word_diff;

pub use types::{DeferredHunkSource, DiffDocument, DiffLine, FileDiff, Hunk, LineKind};
pub use word_diff::compute_word_diff;
