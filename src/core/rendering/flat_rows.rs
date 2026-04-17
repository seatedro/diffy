use serde::Serialize;

use crate::core::diff::types::{FileDiff, LineKind};

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
    let mut rows = vec![FlatDiffRow {
        row_type: DiffRowType::FileHeader,
        file_index: file_index as i32,
        hunk_index: -1,
        line_index: -1,
        old_line_index: -1,
        new_line_index: -1,
    }];

    for (hunk_index, hunk) in file.hunks.iter().enumerate() {
        rows.push(FlatDiffRow {
            row_type: DiffRowType::HunkSeparator,
            file_index: file_index as i32,
            hunk_index: hunk_index as i32,
            line_index: -1,
            old_line_index: -1,
            new_line_index: -1,
        });

        let mut index = 0;
        while index < hunk.lines.len() {
            let line = &hunk.lines[index];
            match line.kind {
                LineKind::Context => {
                    rows.push(FlatDiffRow {
                        row_type: DiffRowType::Context,
                        file_index: file_index as i32,
                        hunk_index: hunk_index as i32,
                        line_index: index as i32,
                        old_line_index: index as i32,
                        new_line_index: index as i32,
                    });
                    index += 1;
                }
                _ => {
                    let mut removed = Vec::new();
                    let mut added = Vec::new();

                    while index < hunk.lines.len() {
                        match hunk.lines[index].kind {
                            LineKind::Removed => removed.push(index),
                            LineKind::Added => added.push(index),
                            LineKind::Context => break,
                        }
                        index += 1;
                    }

                    for &line_index in &removed {
                        rows.push(FlatDiffRow {
                            row_type: DiffRowType::Removed,
                            file_index: file_index as i32,
                            hunk_index: hunk_index as i32,
                            line_index: line_index as i32,
                            old_line_index: line_index as i32,
                            new_line_index: -1,
                        });
                    }

                    for &line_index in &added {
                        rows.push(FlatDiffRow {
                            row_type: DiffRowType::Added,
                            file_index: file_index as i32,
                            hunk_index: hunk_index as i32,
                            line_index: line_index as i32,
                            old_line_index: -1,
                            new_line_index: line_index as i32,
                        });
                    }
                }
            }
        }
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::{DiffRowType, flatten_file_diff};
    use crate::core::diff::types::{DiffLine, FileDiff, Hunk, LineKind};

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
}
