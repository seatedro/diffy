use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;

use crate::core::diff::types::{DiffLine, FileDiff, Hunk};
use crate::core::rendering::{DiffRowType, FlatDiffRow, flatten_file_diff};
use crate::core::text::{DiffTokenSpan, SyntaxTokenKind, TextBuffer, TextRange, TokenBuffer};

pub const INVALID_U32: u32 = u32::MAX;
pub const STYLE_FLAG_CHANGE: u16 = 0x1;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u32,
    pub len: u32,
}

impl ByteRange {
    pub const fn invalid() -> Self {
        Self {
            start: INVALID_U32,
            len: 0,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.start != INVALID_U32
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunRange {
    pub start: u32,
    pub len: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StyleRun {
    pub byte_start: u32,
    pub byte_len: u32,
    pub style_id: u16,
    pub flags: u16,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RenderRowKind {
    #[default]
    FileHeader = 0,
    HunkSeparator = 1,
    Context = 2,
    Added = 3,
    Removed = 4,
    Modified = 5,
}

impl RenderRowKind {
    pub const fn from_diff_row(row_type: DiffRowType) -> Self {
        match row_type {
            DiffRowType::FileHeader => Self::FileHeader,
            DiffRowType::HunkSeparator => Self::HunkSeparator,
            DiffRowType::Context => Self::Context,
            DiffRowType::Added => Self::Added,
            DiffRowType::Removed => Self::Removed,
            DiffRowType::Modified => Self::Modified,
        }
    }

    pub const fn is_body(self) -> bool {
        matches!(
            self,
            Self::Context | Self::Added | Self::Removed | Self::Modified
        )
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderLine {
    pub kind: u8,
    pub flags: u8,
    pub reserved: u16,
    pub old_line_no: u32,
    pub new_line_no: u32,
    pub left_cols: u32,
    pub right_cols: u32,
    pub left_text: ByteRange,
    pub right_text: ByteRange,
    pub left_runs: RunRange,
    pub right_runs: RunRange,
}

impl RenderLine {
    pub fn row_kind(&self) -> RenderRowKind {
        match self.kind {
            0 => RenderRowKind::FileHeader,
            1 => RenderRowKind::HunkSeparator,
            2 => RenderRowKind::Context,
            3 => RenderRowKind::Added,
            4 => RenderRowKind::Removed,
            5 => RenderRowKind::Modified,
            _ => RenderRowKind::Context,
        }
    }

    pub fn primary_cols(&self) -> u32 {
        if self.right_text.is_valid() {
            self.right_cols
        } else {
            self.left_cols
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayRow {
    pub line_index: u32,
    pub y_px: u32,
    pub h_px: u16,
    pub wrap_left: u16,
    pub wrap_right: u16,
    pub kind: u8,
    pub reserved0: u8,
    pub reserved1: u8,
    pub reserved2: u8,
}

impl DisplayRow {
    pub fn bottom_px(&self) -> u32 {
        self.y_px.saturating_add(u32::from(self.h_px))
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderDoc {
    pub text_bytes: Vec<u8>,
    pub style_runs: Vec<StyleRun>,
    pub lines: Vec<RenderLine>,
}

impl RenderDoc {
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line_text(&self, range: ByteRange) -> &str {
        if !range.is_valid() {
            return "";
        }
        let start = range.start as usize;
        let end = start.saturating_add(range.len as usize);
        std::str::from_utf8(self.text_bytes.get(start..end).unwrap_or_default()).unwrap_or("")
    }

    pub fn line_runs(&self, range: RunRange) -> &[StyleRun] {
        let start = range.start as usize;
        let end = start.saturating_add(range.len as usize);
        self.style_runs.get(start..end).unwrap_or(&[])
    }
}

#[derive(Default)]
struct ActiveRenderDoc {
    key: u64,
    doc: Option<Arc<RenderDoc>>,
}

static ACTIVE_RENDER_DOC: OnceLock<RwLock<ActiveRenderDoc>> = OnceLock::new();
static NEXT_RENDER_KEY: AtomicU64 = AtomicU64::new(1);

fn active_registry() -> &'static RwLock<ActiveRenderDoc> {
    ACTIVE_RENDER_DOC.get_or_init(|| RwLock::new(ActiveRenderDoc::default()))
}

pub fn publish_render_doc(doc: RenderDoc) -> i64 {
    let key = NEXT_RENDER_KEY.fetch_add(1, Ordering::Relaxed);
    let mut registry = active_registry().write();
    registry.key = key;
    registry.doc = Some(Arc::new(doc));
    key as i64
}

pub fn clear_render_doc() -> i64 {
    let key = NEXT_RENDER_KEY.fetch_add(1, Ordering::Relaxed);
    let mut registry = active_registry().write();
    registry.key = key;
    registry.doc = None;
    key as i64
}

pub fn clone_render_doc(key: i64) -> Option<Arc<RenderDoc>> {
    let registry = active_registry().read();
    if registry.key == key as u64 {
        registry.doc.clone()
    } else {
        None
    }
}

pub fn build_render_doc(
    file: &FileDiff,
    file_index: usize,
    text_buffer: &TextBuffer,
    token_buffer: &TokenBuffer,
) -> RenderDoc {
    let rows = flatten_file_diff(file, file_index);
    let mut doc = RenderDoc {
        text_bytes: Vec::with_capacity(
            file.path.len()
                + file
                    .hunks
                    .iter()
                    .map(|hunk| hunk.header.len())
                    .sum::<usize>()
                + text_buffer.size().min(32 * 1024),
        ),
        style_runs: Vec::with_capacity(token_buffer.len().saturating_mul(2).max(16)),
        lines: Vec::with_capacity(rows.len()),
    };

    for row in rows {
        doc.lines.push(build_render_line(
            file,
            &row,
            &mut doc.text_bytes,
            &mut doc.style_runs,
            text_buffer,
            token_buffer,
        ));
    }

    doc
}

fn build_render_line(
    file: &FileDiff,
    row: &FlatDiffRow,
    text_bytes: &mut Vec<u8>,
    style_runs: &mut Vec<StyleRun>,
    text_buffer: &TextBuffer,
    token_buffer: &TokenBuffer,
) -> RenderLine {
    let kind = RenderRowKind::from_diff_row(row.row_type);
    match row.row_type {
        DiffRowType::FileHeader => {
            let left_text = append_text(text_bytes, &file.path);
            RenderLine {
                kind: kind as u8,
                left_cols: file.path.chars().count() as u32,
                left_text,
                right_text: ByteRange::invalid(),
                left_runs: append_style_runs(style_runs, &file.path, &[], &[]),
                right_runs: RunRange::default(),
                old_line_no: INVALID_U32,
                new_line_no: INVALID_U32,
                ..RenderLine::default()
            }
        }
        DiffRowType::HunkSeparator => {
            let header = hunk_for_row(file, row)
                .map(|hunk| hunk.header.as_str())
                .unwrap_or("");
            let left_text = append_text(text_bytes, header);
            RenderLine {
                kind: kind as u8,
                left_cols: header.chars().count() as u32,
                left_text,
                right_text: ByteRange::invalid(),
                left_runs: append_style_runs(style_runs, header, &[], &[]),
                right_runs: RunRange::default(),
                old_line_no: INVALID_U32,
                new_line_no: INVALID_U32,
                ..RenderLine::default()
            }
        }
        DiffRowType::Context => {
            let line = main_line_for_row(file, row);
            build_dual_sided_line(
                kind,
                line,
                line,
                text_bytes,
                style_runs,
                text_buffer,
                token_buffer,
            )
        }
        DiffRowType::Added => {
            let line = new_line_for_row(file, row);
            build_dual_sided_line(
                kind,
                None,
                line,
                text_bytes,
                style_runs,
                text_buffer,
                token_buffer,
            )
        }
        DiffRowType::Removed => {
            let line = old_line_for_row(file, row);
            build_dual_sided_line(
                kind,
                line,
                None,
                text_bytes,
                style_runs,
                text_buffer,
                token_buffer,
            )
        }
        DiffRowType::Modified => {
            let old_line = old_line_for_row(file, row);
            let new_line = new_line_for_row(file, row);
            build_dual_sided_line(
                kind,
                old_line,
                new_line,
                text_bytes,
                style_runs,
                text_buffer,
                token_buffer,
            )
        }
    }
}

fn build_dual_sided_line(
    kind: RenderRowKind,
    left_line: Option<&DiffLine>,
    right_line: Option<&DiffLine>,
    text_bytes: &mut Vec<u8>,
    style_runs: &mut Vec<StyleRun>,
    text_buffer: &TextBuffer,
    token_buffer: &TokenBuffer,
) -> RenderLine {
    let (left_text, left_runs, left_cols, old_line_no) = build_line_side(
        left_line,
        text_bytes,
        style_runs,
        text_buffer,
        token_buffer,
        true,
    );
    let (right_text, right_runs, right_cols, new_line_no) = build_line_side(
        right_line,
        text_bytes,
        style_runs,
        text_buffer,
        token_buffer,
        false,
    );

    RenderLine {
        kind: kind as u8,
        old_line_no,
        new_line_no,
        left_cols,
        right_cols,
        left_text,
        right_text,
        left_runs,
        right_runs,
        ..RenderLine::default()
    }
}

fn build_line_side(
    line: Option<&DiffLine>,
    text_bytes: &mut Vec<u8>,
    style_runs: &mut Vec<StyleRun>,
    text_buffer: &TextBuffer,
    token_buffer: &TokenBuffer,
    use_old_number: bool,
) -> (ByteRange, RunRange, u32, u32) {
    let Some(line) = line else {
        return (ByteRange::invalid(), RunRange::default(), 0, INVALID_U32);
    };
    let text = text_buffer.view(line.text_range);
    let range = append_text(text_bytes, text);
    let syntax = token_buffer.view(line.syntax_tokens);
    let change = token_buffer.view(line.change_tokens);
    let runs = append_style_runs(style_runs, text, syntax, change);
    let line_no = if use_old_number {
        line.old_line_number.unwrap_or_default()
    } else {
        line.new_line_number.unwrap_or_default()
    };
    (
        range,
        runs,
        text.chars().count() as u32,
        if line_no > 0 {
            line_no as u32
        } else {
            INVALID_U32
        },
    )
}

fn append_text(storage: &mut Vec<u8>, text: &str) -> ByteRange {
    let start = storage.len() as u32;
    storage.extend_from_slice(text.as_bytes());
    ByteRange {
        start,
        len: text.len() as u32,
    }
}

fn append_style_runs(
    storage: &mut Vec<StyleRun>,
    text: &str,
    syntax_tokens: &[DiffTokenSpan],
    change_tokens: &[DiffTokenSpan],
) -> RunRange {
    let start = storage.len() as u32;
    if text.is_empty() {
        return RunRange { start, len: 0 };
    }

    let mut boundaries = Vec::with_capacity(
        2 + syntax_tokens.len().saturating_mul(2) + change_tokens.len().saturating_mul(2),
    );
    boundaries.push(0_u32);
    boundaries.push(text.len() as u32);
    collect_boundaries(&mut boundaries, syntax_tokens, text.len() as u32);
    collect_boundaries(&mut boundaries, change_tokens, text.len() as u32);
    boundaries.sort_unstable();
    boundaries.dedup();

    for window in boundaries.windows(2) {
        let start_byte = window[0];
        let end_byte = window[1];
        if end_byte <= start_byte {
            continue;
        }
        let syntax_kind = token_kind_at(syntax_tokens, start_byte);
        let flags = if token_contains(change_tokens, start_byte) {
            STYLE_FLAG_CHANGE
        } else {
            0
        };
        storage.push(StyleRun {
            byte_start: start_byte,
            byte_len: end_byte - start_byte,
            style_id: syntax_kind as u16,
            flags,
        });
    }

    RunRange {
        start,
        len: (storage.len() as u32).saturating_sub(start),
    }
}

fn collect_boundaries(boundaries: &mut Vec<u32>, tokens: &[DiffTokenSpan], text_len: u32) {
    for token in tokens {
        let start = token.offset.min(text_len);
        let end = token.offset.saturating_add(token.length).min(text_len);
        boundaries.push(start);
        boundaries.push(end);
    }
}

fn token_kind_at(tokens: &[DiffTokenSpan], offset: u32) -> SyntaxTokenKind {
    for token in tokens {
        let end = token.offset.saturating_add(token.length);
        if offset >= token.offset && offset < end {
            return token.kind;
        }
    }
    SyntaxTokenKind::Normal
}

fn token_contains(tokens: &[DiffTokenSpan], offset: u32) -> bool {
    tokens.iter().any(|token| {
        let end = token.offset.saturating_add(token.length);
        offset >= token.offset && offset < end
    })
}

fn hunk_for_row<'a>(file: &'a FileDiff, row: &FlatDiffRow) -> Option<&'a Hunk> {
    usize::try_from(row.hunk_index)
        .ok()
        .and_then(|index| file.hunks.get(index))
}

fn main_line_for_row<'a>(file: &'a FileDiff, row: &FlatDiffRow) -> Option<&'a DiffLine> {
    line_for_range(file, row.hunk_index, row.line_index)
}

fn old_line_for_row<'a>(file: &'a FileDiff, row: &FlatDiffRow) -> Option<&'a DiffLine> {
    line_for_range(file, row.hunk_index, row.old_line_index)
}

fn new_line_for_row<'a>(file: &'a FileDiff, row: &FlatDiffRow) -> Option<&'a DiffLine> {
    line_for_range(file, row.hunk_index, row.new_line_index)
}

fn line_for_range(file: &FileDiff, hunk_index: i32, line_index: i32) -> Option<&DiffLine> {
    let hunk = usize::try_from(hunk_index)
        .ok()
        .and_then(|idx| file.hunks.get(idx))?;
    usize::try_from(line_index)
        .ok()
        .and_then(|idx| hunk.lines.get(idx))
}

pub fn range_len(text: TextRange) -> usize {
    text.len
}

#[cfg(test)]
mod tests {
    use super::{INVALID_U32, RenderRowKind, STYLE_FLAG_CHANGE, build_render_doc};
    use crate::core::diff::types::{DiffLine, FileDiff, Hunk, LineKind};
    use crate::core::text::{DiffTokenSpan, SyntaxTokenKind, TextBuffer, TokenBuffer};

    #[test]
    fn render_doc_keeps_headers_and_dual_sided_modified_lines() {
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        let removed_text = text_buffer.append("old value");
        let added_text = text_buffer.append("new value");
        let removed_change = token_buffer.append(&[DiffTokenSpan {
            offset: 4,
            length: 5,
            kind: SyntaxTokenKind::Normal,
        }]);
        let added_syntax = token_buffer.append(&[DiffTokenSpan {
            offset: 0,
            length: 3,
            kind: SyntaxTokenKind::Keyword,
        }]);

        let file = FileDiff {
            path: "src/app/controller.rs".to_owned(),
            hunks: vec![Hunk {
                header: "@@ -1 +1 @@".to_owned(),
                lines: vec![
                    DiffLine {
                        kind: LineKind::Removed,
                        old_line_number: Some(1),
                        text_range: removed_text,
                        change_tokens: removed_change,
                        ..DiffLine::default()
                    },
                    DiffLine {
                        kind: LineKind::Added,
                        new_line_number: Some(1),
                        text_range: added_text,
                        syntax_tokens: added_syntax,
                        ..DiffLine::default()
                    },
                ],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        };

        let doc = build_render_doc(&file, 0, &text_buffer, &token_buffer);

        assert_eq!(doc.lines.len(), 3);
        assert_eq!(doc.lines[0].row_kind(), RenderRowKind::FileHeader);
        assert_eq!(
            doc.line_text(doc.lines[0].left_text),
            "src/app/controller.rs"
        );
        assert_eq!(doc.lines[1].row_kind(), RenderRowKind::HunkSeparator);
        assert_eq!(doc.lines[2].row_kind(), RenderRowKind::Modified);
        assert_eq!(doc.line_text(doc.lines[2].left_text), "old value");
        assert_eq!(doc.line_text(doc.lines[2].right_text), "new value");
        assert_eq!(doc.lines[2].old_line_no, 1);
        assert_eq!(doc.lines[2].new_line_no, 1);
        assert_eq!(
            doc.line_runs(doc.lines[2].left_runs)[1].flags,
            STYLE_FLAG_CHANGE
        );
        assert_eq!(
            doc.line_runs(doc.lines[2].right_runs)[0].style_id,
            SyntaxTokenKind::Keyword as u16
        );
    }

    #[test]
    fn missing_side_uses_invalid_sentinel() {
        let mut text_buffer = TextBuffer::default();
        let token_buffer = TokenBuffer::default();
        let line_text = text_buffer.append("only added");

        let file = FileDiff {
            path: "src/lib.rs".to_owned(),
            hunks: vec![Hunk {
                header: "@@ -0,0 +1 @@".to_owned(),
                lines: vec![DiffLine {
                    kind: LineKind::Added,
                    new_line_number: Some(1),
                    text_range: line_text,
                    ..DiffLine::default()
                }],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        };

        let doc = build_render_doc(&file, 0, &text_buffer, &token_buffer);
        let line = &doc.lines[2];
        assert_eq!(line.row_kind(), RenderRowKind::Added);
        assert_eq!(line.old_line_no, INVALID_U32);
        assert!(!line.left_text.is_valid());
        assert!(line.right_text.is_valid());
    }
}
