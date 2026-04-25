use serde::Serialize;

use crate::core::diff::types::{FileDiff, Hunk, LineKind};
use carbon::{
    Block, BlockId, BlockRange, DiffSide, ExpansionState, FileId, HunkId, ProjectionMode,
    ProjectionOptions, ProjectionRow, ProjectionRowKind, SourceRange, project_file,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum DiffRowType {
    #[default]
    FileHeader,
    HunkSeparator,
    Context,
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FlatDiffRow {
    pub row_type: DiffRowType,
    pub file_index: i32,
    pub hunk_index: i32,
    pub line_index: i32,
    pub old_line_index: i32,
    pub new_line_index: i32,
}

pub fn flatten_file_diff(file: &FileDiff, file_index: usize) -> Vec<FlatDiffRow> {
    let projected = ProjectedFileRows::build(file, file_index);
    let mut rows = Vec::with_capacity(projected.capacity());
    rows.push(FlatDiffRow {
        row_type: DiffRowType::FileHeader,
        file_index: usize_to_i32_saturating(file_index),
        hunk_index: -1,
        line_index: -1,
        old_line_index: -1,
        new_line_index: -1,
    });
    projected.project_into(&mut rows);

    rows
}

pub fn flatten_carbon_file_diff(
    file: &FileDiff,
    carbon_file: &carbon::FileDiff,
    file_index: usize,
) -> Vec<FlatDiffRow> {
    let projected = ProjectedFileRows::from_carbon(file, carbon_file, file_index);
    let mut rows = Vec::with_capacity(projected.capacity());
    rows.push(FlatDiffRow {
        row_type: DiffRowType::FileHeader,
        file_index: usize_to_i32_saturating(file_index),
        hunk_index: -1,
        line_index: -1,
        old_line_index: -1,
        new_line_index: -1,
    });
    projected.project_into(&mut rows);
    rows
}

#[derive(Debug)]
struct ProjectedFileRows<'a> {
    file: ProjectedCarbonFile<'a>,
    file_index: i32,
    old_lines: Vec<i32>,
    new_lines: Vec<i32>,
    capacity: usize,
}

#[derive(Debug)]
enum ProjectedCarbonFile<'a> {
    Borrowed(&'a carbon::FileDiff),
    Owned(carbon::FileDiff),
}

impl ProjectedCarbonFile<'_> {
    fn as_ref(&self) -> &carbon::FileDiff {
        match self {
            Self::Borrowed(file) => file,
            Self::Owned(file) => file,
        }
    }
}

impl ProjectedFileRows<'_> {
    fn build(file: &FileDiff, file_index: usize) -> ProjectedFileRows<'static> {
        let capacity = file
            .hunks
            .iter()
            .fold(1usize.saturating_add(file.hunks.len()), |acc, hunk| {
                acc.saturating_add(hunk.lines.len())
            });
        let mut projected = ProjectedFileRows {
            file: ProjectedCarbonFile::Owned(carbon::FileDiff {
                id: FileId(usize_to_u32_saturating(file_index)),
                old_path: Some(file.path.clone()),
                new_path: Some(file.path.clone()),
                status: carbon_status(&file.status, file.is_binary),
                is_binary: file.is_binary,
                is_partial: true,
                ..carbon::FileDiff::default()
            }),
            file_index: usize_to_i32_saturating(file_index),
            old_lines: Vec::new(),
            new_lines: Vec::new(),
            capacity,
        };

        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            projected.push_hunk(hunk_index, hunk);
        }

        projected
    }

    fn from_carbon<'a>(
        file: &FileDiff,
        carbon_file: &'a carbon::FileDiff,
        file_index: usize,
    ) -> ProjectedFileRows<'a> {
        let mut projected = ProjectedFileRows {
            file: ProjectedCarbonFile::Borrowed(carbon_file),
            file_index: usize_to_i32_saturating(file_index),
            old_lines: Vec::new(),
            new_lines: Vec::new(),
            capacity: file
                .hunks
                .iter()
                .fold(1usize.saturating_add(file.hunks.len()), |acc, hunk| {
                    acc.saturating_add(hunk.lines.len())
                }),
        };

        for hunk in &file.hunks {
            for (line_index, line) in hunk.lines.iter().enumerate() {
                match line.kind {
                    LineKind::Context => {
                        projected
                            .old_lines
                            .push(usize_to_i32_saturating(line_index));
                        projected
                            .new_lines
                            .push(usize_to_i32_saturating(line_index));
                    }
                    LineKind::Removed => {
                        projected
                            .old_lines
                            .push(usize_to_i32_saturating(line_index));
                    }
                    LineKind::Added => {
                        projected
                            .new_lines
                            .push(usize_to_i32_saturating(line_index));
                    }
                }
            }
        }

        projected
    }

    fn capacity(&self) -> usize {
        self.capacity
    }

    fn project_into(&self, rows: &mut Vec<FlatDiffRow>) {
        project_file(
            self.file.as_ref(),
            ProjectionOptions {
                mode: ProjectionMode::Unified,
                collapsed_context_threshold: 0,
                include_hunk_headers: true,
            },
            &ExpansionState::default(),
            |row| {
                if let Some(flat) = self.flat_row(row) {
                    rows.push(flat);
                }
            },
        );
    }

    fn push_hunk(&mut self, hunk_index: usize, hunk: &Hunk) {
        let hunk_id = HunkId(usize_to_u32_saturating(hunk_index));
        let old_start = i32_to_u32_nonnegative(hunk.old_start);
        let old_count = i32_to_u32_nonnegative(hunk.old_count);
        let new_start = i32_to_u32_nonnegative(hunk.new_start);
        let new_count = i32_to_u32_nonnegative(hunk.new_count);
        let mut blocks = Vec::new();
        let mut line_index = 0usize;
        let mut old_line_cursor = old_start.max(1);
        let mut new_line_cursor = new_start.max(1);

        while line_index < hunk.lines.len() {
            match hunk.lines[line_index].kind {
                LineKind::Context => {
                    let block_id = BlockId(usize_to_u32_saturating(
                        self.file.as_ref().blocks.len().saturating_add(blocks.len()),
                    ));
                    let old_start_index = usize_to_u32_saturating(self.old_lines.len());
                    let new_start_index = usize_to_u32_saturating(self.new_lines.len());
                    let old_line_start = line_old_number(hunk, line_index, old_line_cursor);
                    let new_line_start = line_new_number(hunk, line_index, new_line_cursor);
                    let mut count = 0u32;

                    while line_index < hunk.lines.len()
                        && hunk.lines[line_index].kind == LineKind::Context
                    {
                        self.old_lines.push(usize_to_i32_saturating(line_index));
                        self.new_lines.push(usize_to_i32_saturating(line_index));
                        old_line_cursor =
                            line_old_number(hunk, line_index, old_line_cursor).saturating_add(1);
                        new_line_cursor =
                            line_new_number(hunk, line_index, new_line_cursor).saturating_add(1);
                        count = count.saturating_add(1);
                        line_index += 1;
                    }

                    blocks.push(
                        Block::context(
                            block_id,
                            SourceRange::new(old_start_index, count),
                            SourceRange::new(new_start_index, count),
                        )
                        .with_source_lines(old_line_start, new_line_start),
                    );
                }
                LineKind::Added | LineKind::Removed => {
                    let block_id = BlockId(usize_to_u32_saturating(
                        self.file.as_ref().blocks.len().saturating_add(blocks.len()),
                    ));
                    let old_start_index = usize_to_u32_saturating(self.old_lines.len());
                    let new_start_index = usize_to_u32_saturating(self.new_lines.len());
                    let mut old_line_start = old_line_cursor;
                    let mut new_line_start = new_line_cursor;
                    let mut old_count = 0u32;
                    let mut new_count = 0u32;
                    let mut saw_old = false;
                    let mut saw_new = false;

                    while line_index < hunk.lines.len()
                        && hunk.lines[line_index].kind != LineKind::Context
                    {
                        match hunk.lines[line_index].kind {
                            LineKind::Removed => {
                                let line_number =
                                    line_old_number(hunk, line_index, old_line_cursor);
                                if !saw_old {
                                    old_line_start = line_number;
                                    saw_old = true;
                                }
                                self.old_lines.push(usize_to_i32_saturating(line_index));
                                old_line_cursor = line_number.saturating_add(1);
                                old_count = old_count.saturating_add(1);
                            }
                            LineKind::Added => {
                                let line_number =
                                    line_new_number(hunk, line_index, new_line_cursor);
                                if !saw_new {
                                    new_line_start = line_number;
                                    saw_new = true;
                                }
                                self.new_lines.push(usize_to_i32_saturating(line_index));
                                new_line_cursor = line_number.saturating_add(1);
                                new_count = new_count.saturating_add(1);
                            }
                            LineKind::Context => {}
                        }
                        line_index += 1;
                    }

                    blocks.push(
                        Block::change(
                            block_id,
                            SourceRange::new(old_start_index, old_count),
                            SourceRange::new(new_start_index, new_count),
                        )
                        .with_source_lines(old_line_start, new_line_start),
                    );
                }
            }
        }

        let mut carbon_hunk = carbon::Hunk::new(
            hunk_id,
            old_start,
            old_count,
            new_start,
            new_count,
            BlockRange::default(),
        );
        carbon_hunk.header.clone_from(&hunk.header);
        match &mut self.file {
            ProjectedCarbonFile::Owned(file) => file.add_hunk(carbon_hunk, blocks),
            ProjectedCarbonFile::Borrowed(_) => {}
        }
    }

    fn flat_row(&self, row: ProjectionRow) -> Option<FlatDiffRow> {
        let hunk_index = row
            .hunk_id
            .map(|id| u32_to_i32_saturating(id.0))
            .unwrap_or(-1);
        let base = FlatDiffRow {
            file_index: self.file_index,
            hunk_index,
            ..FlatDiffRow::default()
        };

        match row.kind {
            ProjectionRowKind::HunkHeader => Some(FlatDiffRow {
                row_type: DiffRowType::HunkSeparator,
                line_index: -1,
                old_line_index: -1,
                new_line_index: -1,
                ..base
            }),
            ProjectionRowKind::Context => {
                let old_line_index = self.side_line_index(DiffSide::Old, row.old_index);
                let new_line_index = self.side_line_index(DiffSide::New, row.new_index);
                Some(FlatDiffRow {
                    row_type: DiffRowType::Context,
                    line_index: first_nonnegative(old_line_index, new_line_index),
                    old_line_index,
                    new_line_index,
                    ..base
                })
            }
            ProjectionRowKind::Removed => {
                let old_line_index = self.side_line_index(DiffSide::Old, row.old_index);
                Some(FlatDiffRow {
                    row_type: DiffRowType::Removed,
                    line_index: old_line_index,
                    old_line_index,
                    new_line_index: -1,
                    ..base
                })
            }
            ProjectionRowKind::Added => {
                let new_line_index = self.side_line_index(DiffSide::New, row.new_index);
                Some(FlatDiffRow {
                    row_type: DiffRowType::Added,
                    line_index: new_line_index,
                    old_line_index: -1,
                    new_line_index,
                    ..base
                })
            }
            ProjectionRowKind::Modified => {
                let old_line_index = self.side_line_index(DiffSide::Old, row.old_index);
                let new_line_index = self.side_line_index(DiffSide::New, row.new_index);
                Some(FlatDiffRow {
                    row_type: DiffRowType::Modified,
                    line_index: first_nonnegative(old_line_index, new_line_index),
                    old_line_index,
                    new_line_index,
                    ..base
                })
            }
            ProjectionRowKind::ContextExpanded | ProjectionRowKind::ContextGap => None,
        }
    }

    fn side_line_index(&self, side: DiffSide, source_index: Option<u32>) -> i32 {
        let Some(source_index) = source_index else {
            return -1;
        };
        let lines = match side {
            DiffSide::Old => &self.old_lines,
            DiffSide::New => &self.new_lines,
        };
        lines
            .get(u32_to_usize_saturating(source_index))
            .copied()
            .unwrap_or(-1)
    }
}

fn carbon_status(status: &str, is_binary: bool) -> carbon::FileStatus {
    if is_binary {
        carbon::FileStatus::Binary
    } else {
        match status {
            "added" | "A" => carbon::FileStatus::Added,
            "deleted" | "D" => carbon::FileStatus::Deleted,
            "renamed" | "R" => carbon::FileStatus::Renamed,
            "mode" => carbon::FileStatus::ModeChanged,
            _ => carbon::FileStatus::Modified,
        }
    }
}

fn line_old_number(hunk: &Hunk, line_index: usize, fallback: u32) -> u32 {
    hunk.lines[line_index]
        .old_line_number
        .map(i32_to_u32_nonnegative)
        .unwrap_or(fallback)
}

fn line_new_number(hunk: &Hunk, line_index: usize, fallback: u32) -> u32 {
    hunk.lines[line_index]
        .new_line_number
        .map(i32_to_u32_nonnegative)
        .unwrap_or(fallback)
}

fn first_nonnegative(left: i32, right: i32) -> i32 {
    if left >= 0 { left } else { right }
}

fn i32_to_u32_nonnegative(value: i32) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

fn u32_to_usize_saturating(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn usize_to_u32_saturating(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn u32_to_i32_saturating(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn usize_to_i32_saturating(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{DiffRowType, FlatDiffRow, flatten_carbon_file_diff, flatten_file_diff};
    use crate::core::diff::types::{DiffLine, FileDiff, Hunk, LineKind};
    use crate::core::diff::unified_parser::lower_carbon_document;
    use crate::core::text::TextBuffer;

    fn row_types(rows: &[FlatDiffRow]) -> Vec<DiffRowType> {
        rows.iter().map(|row| row.row_type).collect()
    }

    #[test]
    fn flatten_file_diff_groups_changes_into_block_form() {
        let file = FileDiff {
            hunks: vec![Hunk {
                lines: vec![
                    DiffLine {
                        kind: LineKind::Removed,
                        old_line_number: Some(3),
                        pair_id: Some(7),
                        ..DiffLine::default()
                    },
                    DiffLine {
                        kind: LineKind::Added,
                        new_line_number: Some(3),
                        pair_id: Some(7),
                        ..DiffLine::default()
                    },
                    DiffLine {
                        kind: LineKind::Added,
                        new_line_number: Some(4),
                        ..DiffLine::default()
                    },
                ],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        };

        let rows = flatten_file_diff(&file, 0);
        assert_eq!(rows[2].row_type, DiffRowType::Removed);
        assert_eq!(rows[2].old_line_index, 0);
        assert_eq!(rows[2].new_line_index, -1);
        assert_eq!(rows[3].row_type, DiffRowType::Added);
        assert_eq!(rows[3].new_line_index, 1);
        assert_eq!(rows[4].row_type, DiffRowType::Added);
        assert_eq!(rows[4].new_line_index, 2);
    }

    #[test]
    fn flatten_file_diff_projects_multiple_hunks_through_carbon() {
        let file = FileDiff {
            hunks: vec![
                Hunk {
                    old_start: 3,
                    old_count: 2,
                    new_start: 3,
                    new_count: 2,
                    lines: vec![
                        DiffLine {
                            kind: LineKind::Context,
                            old_line_number: Some(3),
                            new_line_number: Some(3),
                            ..DiffLine::default()
                        },
                        DiffLine {
                            kind: LineKind::Removed,
                            old_line_number: Some(4),
                            ..DiffLine::default()
                        },
                        DiffLine {
                            kind: LineKind::Added,
                            new_line_number: Some(4),
                            ..DiffLine::default()
                        },
                    ],
                    ..Hunk::default()
                },
                Hunk {
                    old_start: 20,
                    old_count: 1,
                    new_start: 20,
                    new_count: 2,
                    lines: vec![
                        DiffLine {
                            kind: LineKind::Added,
                            new_line_number: Some(20),
                            ..DiffLine::default()
                        },
                        DiffLine {
                            kind: LineKind::Context,
                            old_line_number: Some(20),
                            new_line_number: Some(21),
                            ..DiffLine::default()
                        },
                    ],
                    ..Hunk::default()
                },
            ],
            ..FileDiff::default()
        };

        let rows = flatten_file_diff(&file, 2);

        assert_eq!(
            row_types(&rows),
            vec![
                DiffRowType::FileHeader,
                DiffRowType::HunkSeparator,
                DiffRowType::Context,
                DiffRowType::Removed,
                DiffRowType::Added,
                DiffRowType::HunkSeparator,
                DiffRowType::Added,
                DiffRowType::Context,
            ]
        );
        assert_eq!(rows[0].file_index, 2);
        assert_eq!(rows[5].hunk_index, 1);
        assert_eq!(rows[6].new_line_index, 0);
        assert_eq!(rows[7].line_index, 1);
        assert_eq!(rows[7].old_line_index, 1);
        assert_eq!(rows[7].new_line_index, 1);
    }

    #[test]
    fn flatten_file_diff_preserves_side_order_for_split_change_blocks() {
        let file = FileDiff {
            hunks: vec![Hunk {
                lines: vec![
                    DiffLine {
                        kind: LineKind::Removed,
                        old_line_number: Some(8),
                        ..DiffLine::default()
                    },
                    DiffLine {
                        kind: LineKind::Added,
                        new_line_number: Some(8),
                        ..DiffLine::default()
                    },
                    DiffLine {
                        kind: LineKind::Removed,
                        old_line_number: Some(9),
                        ..DiffLine::default()
                    },
                ],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        };

        let rows = flatten_file_diff(&file, 0);

        assert_eq!(
            row_types(&rows),
            vec![
                DiffRowType::FileHeader,
                DiffRowType::HunkSeparator,
                DiffRowType::Removed,
                DiffRowType::Removed,
                DiffRowType::Added,
            ]
        );
        assert_eq!(rows[2].old_line_index, 0);
        assert_eq!(rows[3].old_line_index, 2);
        assert_eq!(rows[4].new_line_index, 1);
    }

    #[test]
    fn flatten_carbon_file_diff_uses_existing_carbon_projection() {
        let carbon = carbon::parse_unified_patch(
            "\
diff --git a/src/lib.rs b/src/lib.rs
@@ -1,2 +1,3 @@
 context
-old
+new
+added
",
        )
        .unwrap();
        let mut text_buffer = TextBuffer::default();
        let legacy = lower_carbon_document(&carbon, &mut text_buffer, None);

        let carbon_rows = flatten_carbon_file_diff(&legacy.files[0], &carbon.files[0], 0);
        let legacy_rows = flatten_file_diff(&legacy.files[0], 0);

        assert_eq!(carbon_rows, legacy_rows);
    }
}
