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
    #[serde(skip)]
    pub pair_id: Option<u32>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeferredHunkSource {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_oid: Option<String>,
    pub new_oid: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub status: String,
    pub is_binary: bool,
    pub additions: i32,
    pub deletions: i32,
    pub hunks: Vec<Hunk>,
    #[serde(skip)]
    pub hunks_deferred: bool,
    #[serde(skip)]
    pub stats_deferred: bool,
    #[serde(skip)]
    pub deferred_hunk_source: Option<DeferredHunkSource>,
    #[serde(skip)]
    pub syntax_annotated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiffDocument {
    pub files: Vec<FileDiff>,
}
