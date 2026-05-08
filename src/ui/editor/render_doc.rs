use std::collections::HashMap;

use crate::core::text::{ChangeIntensity, DiffTokenSpan, SyntaxTokenKind, TokenBuffer, TokenRange};

pub const INVALID_U32: u32 = u32::MAX;
pub const STYLE_FLAG_CHANGE: u16 = 0x1;
pub const STYLE_FLAG_NOVEL_WORD: u16 = 0x2;
pub const STYLE_FLAG_UNCHANGED_CTX: u16 = 0x4;
pub const DIFF_TAB_WIDTH: u16 = 8;

pub(crate) fn advance_display_col(col: u32, ch: char) -> u32 {
    if ch == '\t' {
        let tab_width = u32::from(DIFF_TAB_WIDTH.max(1));
        let remainder = col % tab_width;
        let advance = if remainder == 0 {
            tab_width
        } else {
            tab_width - remainder
        };
        col.saturating_add(advance)
    } else {
        col.saturating_add(1)
    }
}

pub(crate) fn display_cols(text: &str) -> u32 {
    text.chars().fold(0, advance_display_col)
}

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
    Block = 6,
}

impl RenderRowKind {
    pub const fn is_body(self) -> bool {
        matches!(
            self,
            Self::Context | Self::Added | Self::Removed | Self::Modified
        )
    }

    pub const fn is_block(self) -> bool {
        matches!(self, Self::Block)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderLine {
    pub kind: u8,
    pub flags: u8,
    pub hunk_index: i16,
    pub old_line_no: u32,
    pub new_line_no: u32,
    pub left_cols: u32,
    pub right_cols: u32,
    pub left_text: ByteRange,
    pub right_text: ByteRange,
    pub left_runs: RunRange,
    pub right_runs: RunRange,
    pub line_index: i32,
    pub old_line_index: i32,
    pub new_line_index: i32,
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
            6 => RenderRowKind::Block,
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
    pub block_index: u16,
}

impl DisplayRow {
    pub fn bottom_px(&self) -> u32 {
        self.y_px.saturating_add(u32::from(self.h_px))
    }

    pub fn is_block(&self) -> bool {
        self.kind == RenderRowKind::Block as u8
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderDoc {
    pub text_bytes: Vec<u8>,
    pub style_runs: Vec<StyleRun>,
    pub lines: Vec<RenderLine>,
    pub file_metadata: Vec<FileHeaderMeta>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileHeaderMeta {
    pub path: String,
    pub old_path: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    pub is_binary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CarbonLineKey {
    pub hunk_id: u32,
    pub side: carbon::DiffSide,
    pub source_index: u32,
}

#[derive(Debug, Clone, Default)]
pub struct CarbonStyleOverlays {
    syntax: HashMap<CarbonLineKey, TokenRange>,
    change: HashMap<CarbonLineKey, TokenRange>,
}

impl CarbonStyleOverlays {
    pub fn clear(&mut self) {
        self.syntax.clear();
        self.change.clear();
    }

    pub fn clear_syntax(&mut self) {
        self.syntax.clear();
    }

    pub fn has_change_tokens(&self) -> bool {
        !self.change.is_empty()
    }

    pub fn insert_syntax(
        &mut self,
        hunk_id: u32,
        side: carbon::DiffSide,
        source_index: u32,
        tokens: TokenRange,
    ) {
        self.syntax.insert(
            CarbonLineKey {
                hunk_id,
                side,
                source_index,
            },
            tokens,
        );
    }

    pub fn insert_change(
        &mut self,
        hunk_id: u32,
        side: carbon::DiffSide,
        source_index: u32,
        tokens: TokenRange,
    ) {
        self.change.insert(
            CarbonLineKey {
                hunk_id,
                side,
                source_index,
            },
            tokens,
        );
    }

    fn syntax_tokens<'a>(
        &self,
        token_buffer: &'a TokenBuffer,
        hunk_id: u32,
        side: carbon::DiffSide,
        source_index: u32,
    ) -> &'a [DiffTokenSpan] {
        self.syntax
            .get(&CarbonLineKey {
                hunk_id,
                side,
                source_index,
            })
            .map(|range| token_buffer.view(*range))
            .unwrap_or(&[])
    }

    fn change_tokens<'a>(
        &self,
        token_buffer: &'a TokenBuffer,
        hunk_id: u32,
        side: carbon::DiffSide,
        source_index: u32,
    ) -> &'a [DiffTokenSpan] {
        self.change
            .get(&CarbonLineKey {
                hunk_id,
                side,
                source_index,
            })
            .map(|range| token_buffer.view(*range))
            .unwrap_or(&[])
    }
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

    pub fn append_doc(&mut self, other: &RenderDoc) {
        let byte_offset = self.text_bytes.len() as u32;
        let run_offset = self.style_runs.len() as u32;
        let meta_offset = self.file_metadata.len();
        self.text_bytes.extend_from_slice(&other.text_bytes);
        self.style_runs.extend_from_slice(&other.style_runs);
        self.file_metadata
            .extend(other.file_metadata.iter().cloned());
        self.lines
            .extend(other.lines.iter().copied().map(|mut line| {
                line.left_text = offset_byte_range(line.left_text, byte_offset);
                line.right_text = offset_byte_range(line.right_text, byte_offset);
                line.left_runs = offset_run_range(line.left_runs, run_offset);
                line.right_runs = offset_run_range(line.right_runs, run_offset);
                if line.row_kind() == RenderRowKind::FileHeader && line.flags != 0 {
                    let original_index = (line.flags as usize).saturating_sub(1);
                    let new_index = original_index.saturating_add(meta_offset);
                    line.flags = u8::try_from(new_index.saturating_add(1)).unwrap_or(0);
                }
                line
            }));
    }

    pub fn file_meta(&self, line: &RenderLine) -> Option<&FileHeaderMeta> {
        if line.row_kind() != RenderRowKind::FileHeader || line.flags == 0 {
            return None;
        }
        let index = (line.flags as usize).saturating_sub(1);
        self.file_metadata.get(index)
    }
}

fn offset_byte_range(range: ByteRange, offset: u32) -> ByteRange {
    if range.is_valid() {
        ByteRange {
            start: range.start.saturating_add(offset),
            len: range.len,
        }
    } else {
        range
    }
}

fn offset_run_range(range: RunRange, offset: u32) -> RunRange {
    RunRange {
        start: range.start.saturating_add(offset),
        len: range.len,
    }
}

pub fn build_placeholder_render_doc(path: &str, message: &str) -> RenderDoc {
    let mut doc = RenderDoc::default();
    let left_text = append_text(&mut doc.text_bytes, path);
    doc.file_metadata.push(FileHeaderMeta {
        path: path.to_owned(),
        ..FileHeaderMeta::default()
    });
    let flags = u8::try_from(doc.file_metadata.len()).unwrap_or(0);
    doc.lines.push(RenderLine {
        kind: RenderRowKind::FileHeader as u8,
        flags,
        left_cols: display_cols(path),
        left_text,
        right_text: ByteRange::invalid(),
        left_runs: append_style_runs(&mut doc.style_runs, path, &[], &[]),
        right_runs: RunRange::default(),
        old_line_no: INVALID_U32,
        new_line_no: INVALID_U32,
        ..RenderLine::default()
    });

    let left_text = append_text(&mut doc.text_bytes, message);
    doc.lines.push(RenderLine {
        kind: RenderRowKind::HunkSeparator as u8,
        hunk_index: -1,
        left_cols: display_cols(message),
        left_text,
        right_text: ByteRange::invalid(),
        left_runs: append_style_runs(&mut doc.style_runs, message, &[], &[]),
        right_runs: RunRange::default(),
        old_line_no: INVALID_U32,
        new_line_no: INVALID_U32,
        ..RenderLine::default()
    });
    doc
}

pub fn build_render_doc_from_carbon(
    carbon_file: &carbon::FileDiff,
    file_index: usize,
    expansion: &carbon::ExpansionState,
    overlays: &CarbonStyleOverlays,
    token_buffer: &TokenBuffer,
) -> RenderDoc {
    build_render_doc_from_carbon_rows(carbon_file, file_index, expansion, overlays, token_buffer)
}

fn build_render_doc_from_carbon_rows(
    carbon_file: &carbon::FileDiff,
    file_index: usize,
    expansion: &carbon::ExpansionState,
    overlays: &CarbonStyleOverlays,
    token_buffer: &TokenBuffer,
) -> RenderDoc {
    let text_capacity = carbon_file.path().len()
        + carbon_file
            .hunks
            .iter()
            .map(|hunk| hunk.header.len())
            .sum::<usize>()
        + carbon_file
            .old_text
            .as_ref()
            .map(|text| carbon::u32_to_usize_saturating(text.len()))
            .unwrap_or_default()
            .min(16 * 1024)
        + carbon_file
            .new_text
            .as_ref()
            .map(|text| carbon::u32_to_usize_saturating(text.len()))
            .unwrap_or_default()
            .min(16 * 1024);
    let mut doc = RenderDoc {
        text_bytes: Vec::with_capacity(text_capacity),
        style_runs: Vec::with_capacity(token_buffer.len().saturating_mul(2).max(16)),
        lines: Vec::with_capacity(carbon_projection_capacity(carbon_file)),
        file_metadata: Vec::with_capacity(1),
    };

    doc.lines.push(carbon_file_header_line(
        carbon_file,
        &mut doc.text_bytes,
        &mut doc.style_runs,
        &mut doc.file_metadata,
    ));
    carbon::project_file(
        carbon_file,
        carbon::ProjectionOptions {
            mode: carbon::ProjectionMode::Unified,
            collapsed_context_threshold: 0,
            include_hunk_headers: true,
        },
        expansion,
        |row| {
            if row.kind == carbon::ProjectionRowKind::ContextGap {
                return;
            }
            doc.lines.push(build_render_line_from_carbon(
                carbon_file,
                file_index,
                row,
                overlays,
                &mut doc.text_bytes,
                &mut doc.style_runs,
                token_buffer,
            ));
        },
    );

    doc
}

fn carbon_file_header_line(
    carbon_file: &carbon::FileDiff,
    text_bytes: &mut Vec<u8>,
    style_runs: &mut Vec<StyleRun>,
    file_metadata: &mut Vec<FileHeaderMeta>,
) -> RenderLine {
    let path = carbon_file.path();
    let left_text = append_text(text_bytes, path);
    let path_string = path.to_owned();
    let old_path = match carbon_file.status {
        carbon::FileStatus::Renamed | carbon::FileStatus::RenamedModified => carbon_file
            .old_path
            .as_deref()
            .filter(|old| *old != path)
            .map(str::to_owned),
        _ => None,
    };
    file_metadata.push(FileHeaderMeta {
        path: path_string,
        old_path,
        additions: carbon_file.additions,
        deletions: carbon_file.deletions,
        is_binary: carbon_file.is_binary,
    });
    let flags = u8::try_from(file_metadata.len()).unwrap_or(0);
    RenderLine {
        kind: RenderRowKind::FileHeader as u8,
        flags,
        left_cols: display_cols(path),
        left_text,
        right_text: ByteRange::invalid(),
        left_runs: append_style_runs(style_runs, path, &[], &[]),
        right_runs: RunRange::default(),
        old_line_no: INVALID_U32,
        new_line_no: INVALID_U32,
        ..RenderLine::default()
    }
}

fn build_render_line_from_carbon(
    carbon_file: &carbon::FileDiff,
    file_index: usize,
    row: carbon::ProjectionRow,
    overlays: &CarbonStyleOverlays,
    text_bytes: &mut Vec<u8>,
    style_runs: &mut Vec<StyleRun>,
    token_buffer: &TokenBuffer,
) -> RenderLine {
    let source = SourceIndices::from_carbon_row(row);
    match row.kind {
        carbon::ProjectionRowKind::HunkHeader => {
            let header = row
                .hunk_id
                .and_then(|hunk_id| carbon_file.hunk(hunk_id))
                .map(|hunk| hunk.header.as_str())
                .unwrap_or("");
            let left_text = append_text(text_bytes, header);
            RenderLine {
                kind: RenderRowKind::HunkSeparator as u8,
                hunk_index: source.hunk_index,
                left_cols: display_cols(header),
                left_text,
                right_text: ByteRange::invalid(),
                left_runs: append_style_runs(style_runs, header, &[], &[]),
                right_runs: RunRange::default(),
                old_line_no: INVALID_U32,
                new_line_no: INVALID_U32,
                ..RenderLine::default()
            }
        }
        carbon::ProjectionRowKind::Context | carbon::ProjectionRowKind::ContextExpanded => {
            let mut rl = build_dual_sided_line_with_text(
                RenderRowKind::Context,
                carbon_line_source_from_row(
                    carbon_file,
                    row,
                    carbon::DiffSide::Old,
                    overlays,
                    token_buffer,
                ),
                carbon_line_source_from_row(
                    carbon_file,
                    row,
                    carbon::DiffSide::New,
                    overlays,
                    token_buffer,
                ),
                text_bytes,
                style_runs,
            );
            source.apply(&mut rl);
            rl
        }
        carbon::ProjectionRowKind::Added => {
            let mut rl = build_dual_sided_line_with_text(
                RenderRowKind::Added,
                None,
                carbon_line_source_from_row(
                    carbon_file,
                    row,
                    carbon::DiffSide::New,
                    overlays,
                    token_buffer,
                ),
                text_bytes,
                style_runs,
            );
            source.apply(&mut rl);
            rl
        }
        carbon::ProjectionRowKind::Removed => {
            let mut rl = build_dual_sided_line_with_text(
                RenderRowKind::Removed,
                carbon_line_source_from_row(
                    carbon_file,
                    row,
                    carbon::DiffSide::Old,
                    overlays,
                    token_buffer,
                ),
                None,
                text_bytes,
                style_runs,
            );
            source.apply(&mut rl);
            rl
        }
        carbon::ProjectionRowKind::Modified => {
            let mut rl = build_dual_sided_line_with_text(
                RenderRowKind::Modified,
                carbon_line_source_from_row(
                    carbon_file,
                    row,
                    carbon::DiffSide::Old,
                    overlays,
                    token_buffer,
                ),
                carbon_line_source_from_row(
                    carbon_file,
                    row,
                    carbon::DiffSide::New,
                    overlays,
                    token_buffer,
                ),
                text_bytes,
                style_runs,
            );
            source.apply(&mut rl);
            rl
        }
        carbon::ProjectionRowKind::ContextGap => RenderLine {
            kind: RenderRowKind::Context as u8,
            hunk_index: source.hunk_index,
            line_index: i32::try_from(file_index).unwrap_or(i32::MAX),
            old_line_no: INVALID_U32,
            new_line_no: INVALID_U32,
            ..RenderLine::default()
        },
    }
}

struct SourceIndices {
    hunk_index: i16,
    line_index: i32,
    old_line_index: i32,
    new_line_index: i32,
}

impl SourceIndices {
    fn from_carbon_row(row: carbon::ProjectionRow) -> Self {
        let hunk_index = row
            .hunk_id
            .map(|hunk_id| i16::try_from(hunk_id.0).unwrap_or(i16::MAX))
            .unwrap_or(-1);
        let old_line_index = row
            .old_index
            .map(|index| i32::try_from(index).unwrap_or(i32::MAX))
            .unwrap_or(-1);
        let new_line_index = row
            .new_index
            .map(|index| i32::try_from(index).unwrap_or(i32::MAX))
            .unwrap_or(-1);
        Self {
            hunk_index,
            line_index: if old_line_index >= 0 {
                old_line_index
            } else {
                new_line_index
            },
            old_line_index,
            new_line_index,
        }
    }

    fn apply(&self, line: &mut RenderLine) {
        line.hunk_index = self.hunk_index;
        line.line_index = self.line_index;
        line.old_line_index = self.old_line_index;
        line.new_line_index = self.new_line_index;
    }
}

struct LineSideSource<'a> {
    text: &'a str,
    syntax: &'a [DiffTokenSpan],
    core_change: &'a [DiffTokenSpan],
    carbon_change: &'a [carbon::InlineSpan],
    line_no: Option<u32>,
}

fn build_dual_sided_line_with_text(
    kind: RenderRowKind,
    left_line: Option<LineSideSource<'_>>,
    right_line: Option<LineSideSource<'_>>,
    text_bytes: &mut Vec<u8>,
    style_runs: &mut Vec<StyleRun>,
) -> RenderLine {
    let (left_text, left_runs, left_cols, old_line_no) =
        build_line_side(left_line, text_bytes, style_runs);
    let (right_text, right_runs, right_cols, new_line_no) =
        build_line_side(right_line, text_bytes, style_runs);

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
    line: Option<LineSideSource<'_>>,
    text_bytes: &mut Vec<u8>,
    style_runs: &mut Vec<StyleRun>,
) -> (ByteRange, RunRange, u32, u32) {
    let Some(line) = line else {
        return (ByteRange::invalid(), RunRange::default(), 0, INVALID_U32);
    };
    let text = line.text;
    let range = append_text(text_bytes, text);
    let runs = append_style_runs_with_carbon(
        style_runs,
        text,
        line.syntax,
        line.core_change,
        line.carbon_change,
    );
    (
        range,
        runs,
        display_cols(text),
        line.line_no.unwrap_or(INVALID_U32),
    )
}

fn carbon_line_source_from_row<'a>(
    file: &'a carbon::FileDiff,
    row: carbon::ProjectionRow,
    side: carbon::DiffSide,
    overlays: &'a CarbonStyleOverlays,
    token_buffer: &'a TokenBuffer,
) -> Option<LineSideSource<'a>> {
    let index = match side {
        carbon::DiffSide::Old => row.old_index?,
        carbon::DiffSide::New => row.new_index?,
    };
    let line_no = match side {
        carbon::DiffSide::Old => row.old_line,
        carbon::DiffSide::New => row.new_line,
    };
    let text = file.side_text(side)?.line_str(carbon::LineId(index))?;
    let hunk_id = row.hunk_id.map(|id| id.0).unwrap_or(u32::MAX);
    Some(LineSideSource {
        text,
        syntax: overlays.syntax_tokens(token_buffer, hunk_id, side, index),
        core_change: overlays.change_tokens(token_buffer, hunk_id, side, index),
        carbon_change: carbon_inline_for_row(file, row, side),
        line_no,
    })
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
    append_style_runs_with_carbon(storage, text, syntax_tokens, change_tokens, &[])
}

fn append_style_runs_with_carbon(
    storage: &mut Vec<StyleRun>,
    text: &str,
    syntax_tokens: &[DiffTokenSpan],
    change_tokens: &[DiffTokenSpan],
    carbon_change: &[carbon::InlineSpan],
) -> RunRange {
    let start = storage.len() as u32;
    if text.is_empty() {
        return RunRange { start, len: 0 };
    }

    let mut boundaries = Vec::with_capacity(
        2 + syntax_tokens.len().saturating_mul(2)
            + change_tokens.len().saturating_mul(2)
            + carbon_change.len().saturating_mul(2),
    );
    boundaries.push(0_u32);
    boundaries.push(text.len() as u32);
    collect_boundaries(&mut boundaries, syntax_tokens, text.len() as u32);
    collect_boundaries(&mut boundaries, change_tokens, text.len() as u32);
    collect_carbon_boundaries(&mut boundaries, carbon_change, text.len() as u32);
    boundaries.sort_unstable();
    boundaries.dedup();

    for window in boundaries.windows(2) {
        let start_byte = window[0];
        let end_byte = window[1];
        if end_byte <= start_byte {
            continue;
        }
        let syntax_kind = match token_kind_at(syntax_tokens, start_byte) {
            SyntaxTokenKind::Normal => token_kind_at(change_tokens, start_byte),
            kind => kind,
        };
        let flags = change_flags_at(change_tokens, start_byte)
            | carbon_change_flags_at(carbon_change, start_byte);
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

fn collect_carbon_boundaries(
    boundaries: &mut Vec<u32>,
    tokens: &[carbon::InlineSpan],
    text_len: u32,
) {
    for token in tokens {
        let start = token.offset.min(text_len);
        let end = token.offset.saturating_add(token.len).min(text_len);
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

fn change_flags_at(tokens: &[DiffTokenSpan], offset: u32) -> u16 {
    for token in tokens {
        let end = token.offset.saturating_add(token.length);
        if offset >= token.offset && offset < end {
            return match token.intensity {
                ChangeIntensity::NovelWord => STYLE_FLAG_CHANGE | STYLE_FLAG_NOVEL_WORD,
                ChangeIntensity::UnchangedContext => STYLE_FLAG_CHANGE | STYLE_FLAG_UNCHANGED_CTX,
                ChangeIntensity::Novel => STYLE_FLAG_CHANGE,
            };
        }
    }
    0
}

fn carbon_change_flags_at(tokens: &[carbon::InlineSpan], offset: u32) -> u16 {
    for token in tokens {
        let end = token.offset.saturating_add(token.len);
        if offset >= token.offset && offset < end {
            return match token.intensity {
                carbon::ChangeIntensity::NovelWord => STYLE_FLAG_CHANGE | STYLE_FLAG_NOVEL_WORD,
                carbon::ChangeIntensity::UnchangedContext => {
                    STYLE_FLAG_CHANGE | STYLE_FLAG_UNCHANGED_CTX
                }
                carbon::ChangeIntensity::Novel => STYLE_FLAG_CHANGE,
            };
        }
    }
    0
}

fn carbon_inline_for_row(
    file: &carbon::FileDiff,
    row: carbon::ProjectionRow,
    side: carbon::DiffSide,
) -> &[carbon::InlineSpan] {
    let Some(block_id) = row.block_id else {
        return &[];
    };
    let Some(block) = file.block(block_id) else {
        return &[];
    };
    match side {
        carbon::DiffSide::Old => &block.old_inline,
        carbon::DiffSide::New => &block.new_inline,
    }
}

fn carbon_projection_capacity(file: &carbon::FileDiff) -> usize {
    file.hunks
        .iter()
        .fold(1usize.saturating_add(file.hunks.len()), |acc, hunk| {
            acc.saturating_add(carbon::u32_to_usize_saturating(
                hunk.old_count.max(hunk.new_count),
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::{
        CarbonStyleOverlays, INVALID_U32, RenderDoc, RenderRowKind, STYLE_FLAG_CHANGE,
        build_render_doc_from_carbon,
    };
    use crate::core::text::{DiffTokenSpan, SyntaxTokenKind, TokenBuffer};

    fn carbon_doc(
        file: &carbon::FileDiff,
        overlays: &CarbonStyleOverlays,
        tokens: &TokenBuffer,
    ) -> RenderDoc {
        build_render_doc_from_carbon(
            file,
            0,
            &carbon::ExpansionState::default(),
            overlays,
            tokens,
        )
    }

    #[test]
    fn render_doc_keeps_headers_and_emits_block_style_changes() {
        let mut token_buffer = TokenBuffer::default();
        let removed_change = token_buffer.append(&[DiffTokenSpan {
            offset: 4,
            length: 5,
            kind: SyntaxTokenKind::Normal,
            ..DiffTokenSpan::default()
        }]);
        let added_syntax = token_buffer.append(&[DiffTokenSpan {
            offset: 0,
            length: 3,
            kind: SyntaxTokenKind::Keyword,
            ..DiffTokenSpan::default()
        }]);
        let file = carbon::parse_unified_patch(
            "\
diff --git a/src/app/controller.rs b/src/app/controller.rs
--- a/src/app/controller.rs
+++ b/src/app/controller.rs
@@ -1 +1 @@
-old value
+new value
",
        )
        .unwrap()
        .files
        .into_iter()
        .next()
        .unwrap();
        let mut overlays = CarbonStyleOverlays::default();
        overlays.insert_change(0, carbon::DiffSide::Old, 0, removed_change);
        overlays.insert_syntax(0, carbon::DiffSide::New, 0, added_syntax);

        let doc = carbon_doc(&file, &overlays, &token_buffer);

        assert_eq!(doc.lines.len(), 4);
        assert_eq!(doc.lines[0].row_kind(), RenderRowKind::FileHeader);
        assert_eq!(
            doc.line_text(doc.lines[0].left_text),
            "src/app/controller.rs"
        );
        assert_eq!(doc.lines[1].row_kind(), RenderRowKind::HunkSeparator);
        assert_eq!(doc.lines[2].row_kind(), RenderRowKind::Removed);
        assert_eq!(doc.line_text(doc.lines[2].left_text), "old value");
        assert!(!doc.lines[2].right_text.is_valid());
        assert_eq!(doc.lines[2].old_line_no, 1);
        assert_eq!(doc.lines[2].new_line_no, INVALID_U32);
        assert_eq!(
            doc.line_runs(doc.lines[2].left_runs)[1].flags,
            STYLE_FLAG_CHANGE
        );
        assert_eq!(doc.lines[3].row_kind(), RenderRowKind::Added);
        assert!(!doc.lines[3].left_text.is_valid());
        assert_eq!(doc.line_text(doc.lines[3].right_text), "new value");
        assert_eq!(doc.lines[3].old_line_no, INVALID_U32);
        assert_eq!(doc.lines[3].new_line_no, 1);
        assert_eq!(
            doc.line_runs(doc.lines[3].right_runs)[0].style_id,
            SyntaxTokenKind::Keyword as u16
        );
    }

    #[test]
    fn missing_side_uses_invalid_sentinel() {
        let token_buffer = TokenBuffer::default();
        let file = carbon::parse_unified_patch(
            "\
diff --git a/src/lib.rs b/src/lib.rs
--- /dev/null
+++ b/src/lib.rs
@@ -0,0 +1 @@
+only added
",
        )
        .unwrap()
        .files
        .into_iter()
        .next()
        .unwrap();

        let doc = carbon_doc(&file, &CarbonStyleOverlays::default(), &token_buffer);
        let line = &doc.lines[2];
        assert_eq!(line.row_kind(), RenderRowKind::Added);
        assert_eq!(line.old_line_no, INVALID_U32);
        assert!(!line.left_text.is_valid());
        assert!(line.right_text.is_valid());
    }

    #[test]
    fn change_tokens_can_supply_semantic_style_when_syntax_tokens_are_absent() {
        let mut token_buffer = TokenBuffer::default();
        let semantic_change = token_buffer.append(&[DiffTokenSpan {
            offset: 3,
            length: 3,
            kind: SyntaxTokenKind::Keyword,
            ..DiffTokenSpan::default()
        }]);
        let file = carbon::parse_unified_patch(
            "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +0,0 @@
-fn old_call();
",
        )
        .unwrap()
        .files
        .into_iter()
        .next()
        .unwrap();
        let mut overlays = CarbonStyleOverlays::default();
        overlays.insert_change(0, carbon::DiffSide::Old, 0, semantic_change);

        let doc = carbon_doc(&file, &overlays, &token_buffer);
        let runs = doc.line_runs(doc.lines[2].left_runs);
        assert_eq!(runs[1].style_id, SyntaxTokenKind::Keyword as u16);
        assert_eq!(runs[1].flags, STYLE_FLAG_CHANGE);
    }

    #[test]
    fn render_doc_counts_tabs_as_visual_columns() {
        let token_buffer = TokenBuffer::default();
        let file = carbon::parse_unified_patch(
            "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +0,0 @@
-\tab
",
        )
        .unwrap()
        .files
        .into_iter()
        .next()
        .unwrap();

        let doc = carbon_doc(&file, &CarbonStyleOverlays::default(), &token_buffer);
        assert_eq!(doc.lines[2].left_cols, 10);
    }

    #[test]
    fn carbon_render_doc_reads_text_from_text_store() {
        let carbon = carbon::parse_unified_patch(
            "\
diff --git a/src/lib.rs b/src/lib.rs
@@ -1 +1 @@
-old text
+new text
",
        )
        .unwrap();
        let token_buffer = TokenBuffer::default();

        let doc = build_render_doc_from_carbon(
            &carbon.files[0],
            0,
            &carbon::ExpansionState::default(),
            &CarbonStyleOverlays::default(),
            &token_buffer,
        );

        assert_eq!(doc.line_text(doc.lines[2].left_text), "old text");
        assert_eq!(doc.line_text(doc.lines[3].right_text), "new text");
    }

    #[test]
    fn expanded_context_rows_show_full_file_text() {
        let mut file = carbon::parse_unified_patch(
            "\
diff --git a/src/lib.rs b/src/lib.rs
@@ -3 +3 @@
-old text
+new text
",
        )
        .unwrap()
        .files
        .into_iter()
        .next()
        .unwrap();
        file.old_text = Some(carbon::TextStore::from_text("one\ntwo\nold text\nfour\n"));
        file.new_text = Some(carbon::TextStore::from_text("one\ntwo\nnew text\nfour\n"));
        for block in &mut file.blocks {
            block.old.start = block.old_line_start.saturating_sub(1);
            block.new.start = block.new_line_start.saturating_sub(1);
        }
        file.is_partial = false;

        let mut expansion = carbon::ExpansionState::default();
        carbon::expand_context(
            &file,
            &mut expansion,
            file.hunks[0].id,
            carbon::ExpansionDirection::Above,
            1,
        );
        let token_buffer = TokenBuffer::default();

        let doc = build_render_doc_from_carbon(
            &file,
            0,
            &expansion,
            &CarbonStyleOverlays::default(),
            &token_buffer,
        );
        let expanded = doc
            .lines
            .iter()
            .find(|line| line.old_line_no == 2 && line.new_line_no == 2)
            .expect("expanded context line");

        assert_eq!(expanded.row_kind(), RenderRowKind::Context);
        assert_eq!(doc.line_text(expanded.left_text), "two");
        assert_eq!(doc.line_text(expanded.right_text), "two");
        assert!(!doc.lines.iter().any(|line| {
            line.row_kind() == RenderRowKind::Context
                && line.old_line_no == INVALID_U32
                && line.new_line_no == INVALID_U32
                && !line.left_text.is_valid()
                && !line.right_text.is_valid()
        }));
    }
}
