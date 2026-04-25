use std::collections::HashMap;

use crate::model::{Block, BlockId, BlockKind, DiffSide, FileDiff, FileId, Hunk, HunkId};
use crate::review::Anchor;
use crate::text::{LineId, TextByteRange};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ProjectionMode {
    #[default]
    Unified,
    Split,
    Both,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ProjectionRowKind {
    #[default]
    HunkHeader,
    Context,
    ContextExpanded,
    ContextGap,
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectionWindow {
    pub start: u32,
    pub len: u32,
}

impl ProjectionWindow {
    pub const fn contains(self, row_index: u32) -> bool {
        row_index >= self.start && row_index < self.start.saturating_add(self.len)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionOptions {
    pub mode: ProjectionMode,
    pub collapsed_context_threshold: u32,
    pub include_hunk_headers: bool,
}

impl Default for ProjectionOptions {
    fn default() -> Self {
        Self {
            mode: ProjectionMode::Unified,
            collapsed_context_threshold: 2,
            include_hunk_headers: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectionRow {
    pub file_id: FileId,
    pub kind: ProjectionRowKind,
    pub hunk_id: Option<HunkId>,
    pub block_id: Option<BlockId>,
    /// One-based line number on the old side.
    pub old_line: Option<u32>,
    /// One-based line number on the new side.
    pub new_line: Option<u32>,
    /// Zero-based line index on the old side.
    pub old_index: Option<u32>,
    /// Zero-based line index on the new side.
    pub new_index: Option<u32>,
    pub collapsed_count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectionBuffer {
    rows: Vec<ProjectionRow>,
}

impl ProjectionBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            rows: Vec::with_capacity(capacity),
        }
    }

    pub fn rows(&self) -> &[ProjectionRow] {
        &self.rows
    }

    pub fn clear(&mut self) {
        self.rows.clear();
    }

    pub fn reserve(&mut self, additional: usize) {
        self.rows.reserve(additional);
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn capacity(&self) -> usize {
        self.rows.capacity()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn rebuild_file(
        &mut self,
        file: &FileDiff,
        options: ProjectionOptions,
        expansion: &ExpansionState,
    ) {
        self.rows.clear();
        self.append_file(file, options, expansion);
    }

    pub fn append_file(
        &mut self,
        file: &FileDiff,
        options: ProjectionOptions,
        expansion: &ExpansionState,
    ) {
        project_file(file, options, expansion, |row| self.rows.push(row));
    }

    pub fn rebuild_window(
        &mut self,
        file: &FileDiff,
        options: ProjectionOptions,
        expansion: &ExpansionState,
        window: ProjectionWindow,
    ) {
        self.rows.clear();
        self.append_window(file, options, expansion, window);
    }

    pub fn append_window(
        &mut self,
        file: &FileDiff,
        options: ProjectionOptions,
        expansion: &ExpansionState,
        window: ProjectionWindow,
    ) {
        project_window(file, options, expansion, window, |row| self.rows.push(row));
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HunkExpansion {
    pub above: u32,
    pub below: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExpansionState {
    hunks: HashMap<HunkId, HunkExpansion>,
}

impl ExpansionState {
    pub fn hunk(&self, hunk_id: HunkId) -> HunkExpansion {
        self.hunks.get(&hunk_id).copied().unwrap_or_default()
    }

    pub fn set_hunk(&mut self, hunk_id: HunkId, expansion: HunkExpansion) {
        self.hunks.insert(hunk_id, expansion);
    }

    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpansionDirection {
    Above,
    Below,
}

pub fn expand_context(
    file: &FileDiff,
    state: &mut ExpansionState,
    hunk_id: HunkId,
    direction: ExpansionDirection,
    amount: u32,
) {
    let caps = expansion_caps(file, hunk_id);
    let mut expansion = state.hunk(hunk_id);
    match direction {
        ExpansionDirection::Above => {
            expansion.above = expansion.above.saturating_add(amount).min(caps.above);
        }
        ExpansionDirection::Below => {
            expansion.below = expansion.below.saturating_add(amount).min(caps.below);
        }
    }
    state.set_hunk(hunk_id, expansion);
}

pub fn expansion_caps(file: &FileDiff, hunk_id: HunkId) -> HunkExpansion {
    let Some(index) = file.hunks.iter().position(|hunk| hunk.id == hunk_id) else {
        return HunkExpansion::default();
    };
    let hunk = &file.hunks[index];
    let above = hunk.new_start_index();
    let below = file
        .hunks
        .get(index + 1)
        .map(Hunk::new_start_index)
        .or_else(|| file.new_text.as_ref().map(|text| text.line_count()))
        .unwrap_or(hunk.new_end_index())
        .saturating_sub(hunk.new_end_index());
    HunkExpansion { above, below }
}

pub fn project_file(
    file: &FileDiff,
    options: ProjectionOptions,
    expansion: &ExpansionState,
    mut emit: impl FnMut(ProjectionRow),
) {
    project_file_inner(file, options, expansion, None, &mut emit);
}

pub fn project_window(
    file: &FileDiff,
    options: ProjectionOptions,
    expansion: &ExpansionState,
    window: ProjectionWindow,
    mut emit: impl FnMut(ProjectionRow),
) {
    project_file_inner(file, options, expansion, Some(window), &mut emit);
}

pub fn map_anchor_to_projection<'a>(
    anchor: &Anchor,
    rows: impl IntoIterator<Item = &'a ProjectionRow>,
) -> Vec<ProjectionRow> {
    rows.into_iter()
        .copied()
        .filter(|row| anchor.touches_row(row))
        .collect()
}

pub fn projected_row_byte_range(
    file: &FileDiff,
    row: &ProjectionRow,
    side: DiffSide,
) -> Option<TextByteRange> {
    let line_index = match side {
        DiffSide::Old => row.old_index,
        DiffSide::New => row.new_index,
    }?;
    file.side_text(side)?.line_range(LineId(line_index))
}

fn project_file_inner(
    file: &FileDiff,
    options: ProjectionOptions,
    expansion: &ExpansionState,
    window: Option<ProjectionWindow>,
    emit: &mut impl FnMut(ProjectionRow),
) {
    let mut index = 0_u32;
    let mut windowed_emit = |row: ProjectionRow| {
        let should_emit = window.is_none_or(|window| window.contains(index));
        index = index.saturating_add(1);
        if should_emit {
            emit(row);
        }
    };

    for (hunk_index, hunk) in file.hunks.iter().enumerate() {
        if hunk_index == 0 {
            let old_gap_start = 0;
            let new_gap_start = 0;
            let gap_len = hunk.old_start_index().min(hunk.new_start_index());
            emit_gap(
                file,
                hunk.id,
                old_gap_start,
                new_gap_start,
                gap_len,
                expansion.hunk(hunk.id).above,
                0,
                options.collapsed_context_threshold,
                &mut windowed_emit,
            );
        }

        if options.include_hunk_headers {
            windowed_emit(ProjectionRow {
                file_id: file.id,
                kind: ProjectionRowKind::HunkHeader,
                hunk_id: Some(hunk.id),
                ..ProjectionRow::default()
            });
        }

        for block in file.hunk_blocks(hunk) {
            match block.kind {
                BlockKind::Context => emit_context(file.id, hunk, block, &mut windowed_emit),
                BlockKind::Change => {
                    emit_change(file.id, hunk, block, options.mode, &mut windowed_emit)
                }
            }
        }

        let next = file.hunks.get(hunk_index + 1);
        let old_gap_start = hunk.old_end_index();
        let new_gap_start = hunk.new_end_index();
        let old_gap_end = next
            .map(Hunk::old_start_index)
            .or_else(|| file.old_text.as_ref().map(|text| text.line_count()))
            .unwrap_or(old_gap_start);
        let new_gap_end = next
            .map(Hunk::new_start_index)
            .or_else(|| file.new_text.as_ref().map(|text| text.line_count()))
            .unwrap_or(new_gap_start);
        let gap_len = old_gap_end
            .saturating_sub(old_gap_start)
            .min(new_gap_end.saturating_sub(new_gap_start));
        let this_expansion = expansion.hunk(hunk.id);
        let next_above = next.map(|hunk| expansion.hunk(hunk.id).above).unwrap_or(0);
        emit_gap(
            file,
            hunk.id,
            old_gap_start,
            new_gap_start,
            gap_len,
            next_above,
            this_expansion.below,
            options.collapsed_context_threshold,
            &mut windowed_emit,
        );
    }
}

fn emit_context(file_id: FileId, hunk: &Hunk, block: &Block, emit: &mut impl FnMut(ProjectionRow)) {
    let count = block.old.len.min(block.new.len);
    for offset in 0..count {
        emit(ProjectionRow {
            file_id,
            kind: ProjectionRowKind::Context,
            hunk_id: Some(hunk.id),
            block_id: Some(block.id),
            old_line: Some(block.old_line_start + offset),
            new_line: Some(block.new_line_start + offset),
            old_index: Some(block.old.start + offset),
            new_index: Some(block.new.start + offset),
            collapsed_count: 0,
        });
    }
}

fn emit_change(
    file_id: FileId,
    hunk: &Hunk,
    block: &Block,
    mode: ProjectionMode,
    emit: &mut impl FnMut(ProjectionRow),
) {
    match mode {
        ProjectionMode::Unified => {
            for offset in 0..block.old.len {
                emit(ProjectionRow {
                    file_id,
                    kind: ProjectionRowKind::Removed,
                    hunk_id: Some(hunk.id),
                    block_id: Some(block.id),
                    old_line: Some(block.old_line_start + offset),
                    old_index: Some(block.old.start + offset),
                    ..ProjectionRow::default()
                });
            }
            for offset in 0..block.new.len {
                emit(ProjectionRow {
                    file_id,
                    kind: ProjectionRowKind::Added,
                    hunk_id: Some(hunk.id),
                    block_id: Some(block.id),
                    new_line: Some(block.new_line_start + offset),
                    new_index: Some(block.new.start + offset),
                    ..ProjectionRow::default()
                });
            }
        }
        ProjectionMode::Split | ProjectionMode::Both => {
            for offset in 0..block.old.len.max(block.new.len) {
                let has_old = offset < block.old.len;
                let has_new = offset < block.new.len;
                emit(ProjectionRow {
                    file_id,
                    kind: match (has_old, has_new) {
                        (true, true) => ProjectionRowKind::Modified,
                        (true, false) => ProjectionRowKind::Removed,
                        (false, true) => ProjectionRowKind::Added,
                        (false, false) => ProjectionRowKind::Modified,
                    },
                    hunk_id: Some(hunk.id),
                    block_id: Some(block.id),
                    old_line: has_old.then_some(block.old_line_start + offset),
                    new_line: has_new.then_some(block.new_line_start + offset),
                    old_index: has_old.then_some(block.old.start + offset),
                    new_index: has_new.then_some(block.new.start + offset),
                    collapsed_count: 0,
                });
            }
        }
    }
}

fn emit_gap(
    file: &FileDiff,
    hunk_id: HunkId,
    old_gap_start: u32,
    new_gap_start: u32,
    gap_len: u32,
    expand_from_end: u32,
    expand_from_start: u32,
    collapsed_context_threshold: u32,
    emit: &mut impl FnMut(ProjectionRow),
) {
    if gap_len == 0 || file.is_partial {
        return;
    }
    if gap_len <= collapsed_context_threshold {
        for offset in 0..gap_len {
            emit_expanded_gap_line(
                file.id,
                hunk_id,
                old_gap_start + offset,
                new_gap_start + offset,
                emit,
            );
        }
        return;
    }
    let from_start = expand_from_start.min(gap_len);
    let remaining = gap_len.saturating_sub(from_start);
    let from_end = expand_from_end.min(remaining);
    let collapsed = gap_len.saturating_sub(from_start + from_end);

    for offset in 0..from_start {
        emit_expanded_gap_line(
            file.id,
            hunk_id,
            old_gap_start + offset,
            new_gap_start + offset,
            emit,
        );
    }
    if collapsed <= collapsed_context_threshold {
        for offset in from_start..gap_len.saturating_sub(from_end) {
            emit_expanded_gap_line(
                file.id,
                hunk_id,
                old_gap_start + offset,
                new_gap_start + offset,
                emit,
            );
        }
    } else if collapsed > 0 {
        emit(ProjectionRow {
            file_id: file.id,
            kind: ProjectionRowKind::ContextGap,
            hunk_id: Some(hunk_id),
            collapsed_count: collapsed,
            ..ProjectionRow::default()
        });
    }
    let old_end_start = old_gap_start + gap_len - from_end;
    let new_end_start = new_gap_start + gap_len - from_end;
    for offset in 0..from_end {
        emit_expanded_gap_line(
            file.id,
            hunk_id,
            old_end_start + offset,
            new_end_start + offset,
            emit,
        );
    }
}

fn emit_expanded_gap_line(
    file_id: FileId,
    hunk_id: HunkId,
    old_index: u32,
    new_index: u32,
    emit: &mut impl FnMut(ProjectionRow),
) {
    emit(ProjectionRow {
        file_id,
        kind: ProjectionRowKind::ContextExpanded,
        hunk_id: Some(hunk_id),
        old_line: Some(old_index + 1),
        new_line: Some(new_index + 1),
        old_index: Some(old_index),
        new_index: Some(new_index),
        collapsed_count: 0,
        block_id: None,
    });
}

#[cfg(test)]
mod tests {
    use super::{
        ExpansionDirection, ExpansionState, ProjectionBuffer, ProjectionMode, ProjectionOptions,
        ProjectionRowKind, ProjectionWindow, expand_context, project_file, project_window,
        projected_row_byte_range,
    };
    use crate::model::{
        Block, BlockId, BlockRange, DiffSide, FileDiff, FileId, Hunk, HunkId, SourceRange,
    };
    use crate::text::TextStore;

    fn sample_file() -> FileDiff {
        let mut file = FileDiff {
            id: FileId(1),
            old_text: Some(TextStore::from_text("a\nold\nc\nd\ne\n")),
            new_text: Some(TextStore::from_text("a\nnew\nc\nd\ne\n")),
            ..FileDiff::default()
        };
        file.add_hunk(
            Hunk::new(HunkId(0), 2, 1, 2, 1, BlockRange::default()),
            [Block::change(
                BlockId(0),
                SourceRange::new(1, 1),
                SourceRange::new(1, 1),
            )],
        );
        file
    }

    #[test]
    fn unified_projection_separates_removed_and_added_rows() {
        let file = sample_file();
        let mut rows = Vec::new();
        project_file(
            &file,
            ProjectionOptions {
                collapsed_context_threshold: 0,
                ..ProjectionOptions::default()
            },
            &ExpansionState::default(),
            |row| rows.push(row),
        );

        assert_eq!(rows[0].kind, ProjectionRowKind::ContextGap);
        assert_eq!(rows[1].kind, ProjectionRowKind::HunkHeader);
        assert_eq!(rows[2].kind, ProjectionRowKind::Removed);
        assert_eq!(rows[3].kind, ProjectionRowKind::Added);
        assert_eq!(rows[4].kind, ProjectionRowKind::ContextGap);
    }

    #[test]
    fn split_projection_pairs_change_rows() {
        let file = sample_file();
        let mut rows = Vec::new();
        project_file(
            &file,
            ProjectionOptions {
                mode: ProjectionMode::Split,
                ..ProjectionOptions::default()
            },
            &ExpansionState::default(),
            |row| rows.push(row),
        );

        let modified = rows
            .iter()
            .find(|row| row.kind == ProjectionRowKind::Modified)
            .unwrap();
        assert_eq!(modified.old_line, Some(2));
        assert_eq!(modified.new_line, Some(2));
    }

    #[test]
    fn expansion_and_windowing_do_not_mutate_hunks() {
        let file = sample_file();
        let mut expansion = ExpansionState::default();
        expand_context(
            &file,
            &mut expansion,
            HunkId(0),
            ExpansionDirection::Below,
            1,
        );
        let mut rows = Vec::new();
        project_window(
            &file,
            ProjectionOptions::default(),
            &expansion,
            ProjectionWindow { start: 4, len: 2 },
            |row| rows.push(row),
        );

        assert_eq!(rows[0].kind, ProjectionRowKind::ContextExpanded);
        assert_eq!(rows[0].new_line, Some(3));
        assert_eq!(file.hunks[0].new_count, 1);
    }

    #[test]
    fn projection_buffer_reuses_capacity_for_materialized_rows() {
        let file = sample_file();
        let mut buffer = ProjectionBuffer::with_capacity(8);
        buffer.rebuild_file(
            &file,
            ProjectionOptions::default(),
            &ExpansionState::default(),
        );
        let capacity = buffer.capacity();

        buffer.rebuild_window(
            &file,
            ProjectionOptions::default(),
            &ExpansionState::default(),
            ProjectionWindow { start: 1, len: 2 },
        );

        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.capacity(), capacity);
    }

    #[test]
    fn projected_rows_map_to_side_specific_text_ranges() {
        let file = sample_file();
        let mut rows = Vec::new();
        project_file(
            &file,
            ProjectionOptions {
                collapsed_context_threshold: 0,
                ..ProjectionOptions::default()
            },
            &ExpansionState::default(),
            |row| rows.push(row),
        );

        let removed = rows
            .iter()
            .find(|row| row.kind == ProjectionRowKind::Removed)
            .unwrap();
        let added = rows
            .iter()
            .find(|row| row.kind == ProjectionRowKind::Added)
            .unwrap();
        let old_range = projected_row_byte_range(&file, removed, DiffSide::Old).unwrap();
        let new_range = projected_row_byte_range(&file, added, DiffSide::New).unwrap();

        assert_eq!(
            file.old_text.as_ref().unwrap().bytes_in_range(old_range),
            Some(&b"old"[..])
        );
        assert_eq!(
            file.new_text.as_ref().unwrap().bytes_in_range(new_range),
            Some(&b"new"[..])
        );
        assert!(projected_row_byte_range(&file, removed, DiffSide::New).is_none());
        assert!(projected_row_byte_range(&file, added, DiffSide::Old).is_none());
    }
}
