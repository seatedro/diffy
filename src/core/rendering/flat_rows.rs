use std::collections::HashSet;

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
        let mut emitted_pairs = HashSet::new();
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
                    if let Some(pair_id) = line.pair_id {
                        if emitted_pairs.insert(pair_id) {
                            let (old_line_index, new_line_index) =
                                paired_line_indexes(hunk, pair_id, index);
                            rows.push(FlatDiffRow {
                                row_type: if old_line_index >= 0 && new_line_index >= 0 {
                                    DiffRowType::Modified
                                } else if old_line_index >= 0 {
                                    DiffRowType::Removed
                                } else {
                                    DiffRowType::Added
                                },
                                file_index: file_index as i32,
                                hunk_index: hunk_index as i32,
                                line_index: index as i32,
                                old_line_index,
                                new_line_index,
                            });
                        }
                        index += 1;
                        continue;
                    }

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

fn paired_line_indexes(
    hunk: &crate::core::diff::types::Hunk,
    pair_id: u32,
    start_index: usize,
) -> (i32, i32) {
    let mut old_line_index = -1;
    let mut new_line_index = -1;
    let mut index = start_index;
    while index < hunk.lines.len() {
        let line = &hunk.lines[index];
        if line.kind == LineKind::Context {
            break;
        }
        if line.pair_id == Some(pair_id) {
            match line.kind {
                LineKind::Removed => old_line_index = index as i32,
                LineKind::Added => new_line_index = index as i32,
                LineKind::Context => {}
            }
        }
        index += 1;
    }
    (old_line_index, new_line_index)
}

#[cfg(test)]
mod tests {
    use super::{DiffRowType, flatten_file_diff};
    use crate::core::diff::types::{DiffLine, FileDiff, Hunk, LineKind};

    #[test]
    fn flatten_file_diff_preserves_explicit_semantic_pairing() {
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
        assert_eq!(rows[2].row_type, DiffRowType::Modified);
        assert_eq!(rows[2].old_line_index, 0);
        assert_eq!(rows[2].new_line_index, 1);
        assert_eq!(rows[3].row_type, DiffRowType::Added);
        assert_eq!(rows[3].new_line_index, 2);
    }
}
