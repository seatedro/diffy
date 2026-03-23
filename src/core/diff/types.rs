use serde::Serialize;

use crate::core::text::buffer::TextRange;
use crate::core::text::token::TokenRange;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum LineKind {
    #[default]
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_line_number: Option<i32>,
    pub new_line_number: Option<i32>,
    pub text_range: TextRange,
    pub syntax_tokens: TokenRange,
    pub change_tokens: TokenRange,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Hunk {
    pub old_start: i32,
    pub old_count: i32,
    pub new_start: i32,
    pub new_count: i32,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub status: String,
    pub additions: i32,
    pub deletions: i32,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiffDocument {
    pub files: Vec<FileDiff>,
}
