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
            match hunk.lines[index].kind {
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
                    let block_start = index;
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

                    let paired = removed.len().min(added.len());
                    for pair_index in 0..paired {
                        rows.push(FlatDiffRow {
                            row_type: DiffRowType::Modified,
                            file_index: file_index as i32,
                            hunk_index: hunk_index as i32,
                            line_index: block_start as i32,
                            old_line_index: removed[pair_index] as i32,
                            new_line_index: added[pair_index] as i32,
                        });
                    }

                    for &line_index in &removed[paired..] {
                        rows.push(FlatDiffRow {
                            row_type: DiffRowType::Removed,
                            file_index: file_index as i32,
                            hunk_index: hunk_index as i32,
                            line_index: line_index as i32,
                            old_line_index: line_index as i32,
                            new_line_index: -1,
                        });
                    }

                    for &line_index in &added[paired..] {
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
