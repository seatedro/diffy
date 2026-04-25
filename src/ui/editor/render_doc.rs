use std::collections::HashMap;

use crate::core::diff::types::{DiffLine, FileDiff, Hunk};
use crate::core::rendering::{DiffRowType, FlatDiffRow, flatten_file_diff};
use crate::core::text::{
    ChangeIntensity, DiffTokenSpan, SyntaxTokenKind, TextBuffer, TextRange, TokenBuffer, TokenRange,
};

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
}

pub fn build_render_doc(
    file: &FileDiff,
    file_index: usize,
    text_buffer: &TextBuffer,
    token_buffer: &TokenBuffer,
) -> RenderDoc {
    let rows = flatten_file_diff(file, file_index);
    build_render_doc_from_rows(file, rows, text_buffer, token_buffer)
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

fn build_render_doc_from_rows(
    file: &FileDiff,
    rows: Vec<FlatDiffRow>,
    text_buffer: &TextBuffer,
    token_buffer: &TokenBuffer,
) -> RenderDoc {
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
    };

    doc.lines.push(carbon_file_header_line(
        carbon_file,
        &mut doc.text_bytes,
        &mut doc.style_runs,
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

fn build_render_line(
    file: &FileDiff,
    row: &FlatDiffRow,
    text_bytes: &mut Vec<u8>,
    style_runs: &mut Vec<StyleRun>,
    text_buffer: &TextBuffer,
    token_buffer: &TokenBuffer,
) -> RenderLine {
    let kind = RenderRowKind::from_diff_row(row.row_type);
    let source = SourceIndices::from_row(row);
    match row.row_type {
        DiffRowType::FileHeader => {
            let left_text = append_text(text_bytes, &file.path);
            RenderLine {
                kind: kind as u8,
                left_cols: display_cols(&file.path),
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
        DiffRowType::Context => {
            let line = main_line_for_row(file, row);
            let mut rl = build_dual_sided_line(
                kind,
                line,
                line,
                text_bytes,
                style_runs,
                text_buffer,
                token_buffer,
            );
            source.apply(&mut rl);
            rl
        }
        DiffRowType::Added => {
            let line = new_line_for_row(file, row);
            let mut rl = build_dual_sided_line(
                kind,
                None,
                line,
                text_bytes,
                style_runs,
                text_buffer,
                token_buffer,
            );
            source.apply(&mut rl);
            rl
        }
        DiffRowType::Removed => {
            let line = old_line_for_row(file, row);
            let mut rl = build_dual_sided_line(
                kind,
                line,
                None,
                text_bytes,
                style_runs,
                text_buffer,
                token_buffer,
            );
            source.apply(&mut rl);
            rl
        }
        DiffRowType::Modified => {
            let old_line = old_line_for_row(file, row);
            let new_line = new_line_for_row(file, row);
            let mut rl = build_dual_sided_line(
                kind,
                old_line,
                new_line,
                text_bytes,
                style_runs,
                text_buffer,
                token_buffer,
            );
            source.apply(&mut rl);
            rl
        }
    }
}

fn carbon_file_header_line(
    carbon_file: &carbon::FileDiff,
    text_bytes: &mut Vec<u8>,
    style_runs: &mut Vec<StyleRun>,
) -> RenderLine {
    let path = carbon_file.path();
    let left_text = append_text(text_bytes, path);
    RenderLine {
        kind: RenderRowKind::FileHeader as u8,
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
        carbon::ProjectionRowKind::Context => {
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
        carbon::ProjectionRowKind::ContextExpanded | carbon::ProjectionRowKind::ContextGap => {
            RenderLine {
                kind: RenderRowKind::Context as u8,
                hunk_index: source.hunk_index,
                line_index: i32::try_from(file_index).unwrap_or(i32::MAX),
                old_line_no: INVALID_U32,
                new_line_no: INVALID_U32,
                ..RenderLine::default()
            }
        }
    }
}

struct SourceIndices {
    hunk_index: i16,
    line_index: i32,
    old_line_index: i32,
    new_line_index: i32,
}

impl SourceIndices {
    fn from_row(row: &FlatDiffRow) -> Self {
        Self {
            hunk_index: row.hunk_index as i16,
            line_index: row.line_index,
            old_line_index: row.old_line_index,
            new_line_index: row.new_line_index,
        }
    }

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

fn build_dual_sided_line(
    kind: RenderRowKind,
    left_line: Option<&DiffLine>,
    right_line: Option<&DiffLine>,
    text_bytes: &mut Vec<u8>,
    style_runs: &mut Vec<StyleRun>,
    text_buffer: &TextBuffer,
    token_buffer: &TokenBuffer,
) -> RenderLine {
    let left = left_line.map(|line| LineSideSource {
        text: text_buffer.view(line.text_range),
        syntax: token_buffer.view(line.syntax_tokens),
        core_change: token_buffer.view(line.change_tokens),
        carbon_change: &[],
        line_no: line.old_line_number.and_then(i32_to_u32_positive),
    });
    let right = right_line.map(|line| LineSideSource {
        text: text_buffer.view(line.text_range),
        syntax: token_buffer.view(line.syntax_tokens),
        core_change: token_buffer.view(line.change_tokens),
        carbon_change: &[],
        line_no: line.new_line_number.and_then(i32_to_u32_positive),
    });
    build_dual_sided_line_with_text(kind, left, right, text_bytes, style_runs)
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

fn carbon_projection_capacity(file: &carbon::FileDiff) -> usize {
    file.hunks
        .iter()
        .fold(1usize.saturating_add(file.hunks.len()), |acc, hunk| {
            acc.saturating_add(carbon::u32_to_usize_saturating(
                hunk.old_count.max(hunk.new_count),
            ))
        })
}

fn i32_to_u32_positive(value: i32) -> Option<u32> {
    (value > 0).then(|| u32::try_from(value).ok()).flatten()
}

pub fn range_len(text: TextRange) -> usize {
    text.len
}

#[cfg(test)]
mod tests {
    use super::{
        CarbonStyleOverlays, INVALID_U32, RenderRowKind, STYLE_FLAG_CHANGE, build_render_doc,
        build_render_doc_from_carbon,
    };
    use crate::core::diff::types::{DiffLine, FileDiff, Hunk, LineKind};
    use crate::core::text::{DiffTokenSpan, SyntaxTokenKind, TextBuffer, TokenBuffer};

    #[test]
    fn render_doc_keeps_headers_and_emits_block_style_changes() {
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        let removed_text = text_buffer.append("old value");
        let added_text = text_buffer.append("new value");
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

    #[test]
    fn change_tokens_can_supply_semantic_style_when_syntax_tokens_are_absent() {
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();
        let line_text = text_buffer.append("fn old_call();");
        let semantic_change = token_buffer.append(&[DiffTokenSpan {
            offset: 3,
            length: 3,
            kind: SyntaxTokenKind::Keyword,
            ..DiffTokenSpan::default()
        }]);

        let file = FileDiff {
            path: "src/lib.rs".to_owned(),
            hunks: vec![Hunk {
                header: "@@ -1 +1 @@".to_owned(),
                lines: vec![DiffLine {
                    kind: LineKind::Removed,
                    old_line_number: Some(1),
                    text_range: line_text,
                    change_tokens: semantic_change,
                    ..DiffLine::default()
                }],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        };

        let doc = build_render_doc(&file, 0, &text_buffer, &token_buffer);
        let runs = doc.line_runs(doc.lines[2].left_runs);
        assert_eq!(runs[1].style_id, SyntaxTokenKind::Keyword as u16);
        assert_eq!(runs[1].flags, STYLE_FLAG_CHANGE);
    }

    #[test]
    fn render_doc_counts_tabs_as_visual_columns() {
        let mut text_buffer = TextBuffer::default();
        let token_buffer = TokenBuffer::default();
        let line_text = text_buffer.append("\tab");

        let file = FileDiff {
            path: "src/lib.rs".to_owned(),
            hunks: vec![Hunk {
                header: "@@ -1 +1 @@".to_owned(),
                lines: vec![DiffLine {
                    kind: LineKind::Removed,
                    old_line_number: Some(1),
                    text_range: line_text,
                    ..DiffLine::default()
                }],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        };

        let doc = build_render_doc(&file, 0, &text_buffer, &token_buffer);
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
}
