pub fn format_carbon_hunk_patch(
    carbon_file: &carbon::FileDiff,
    hunk_index: usize,
    reverse: bool,
) -> Option<String> {
    let hunk = carbon_file.hunks.get(hunk_index)?;
    let mut patch = format_carbon_file_header(carbon_file);
    if reverse {
        append_reversed_carbon_hunk(&mut patch, hunk, carbon_file);
    } else {
        append_carbon_hunk(&mut patch, hunk, carbon_file);
    }
    Some(patch)
}

pub fn format_carbon_lines_patch(
    carbon_file: &carbon::FileDiff,
    hunk_index: usize,
    selected_lines: &[CarbonLineSelection],
    reverse: bool,
) -> Option<String> {
    let hunk = carbon_file.hunks.get(hunk_index)?;
    let mut patch = format_carbon_file_header(carbon_file);
    let rewritten = rewrite_carbon_hunk_for_lines(hunk, selected_lines, carbon_file, reverse)?;
    patch.push_str(&rewritten);
    Some(patch)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CarbonLineSelection {
    pub side: carbon::DiffSide,
    pub source_index: u32,
}

fn format_carbon_file_header(file: &carbon::FileDiff) -> String {
    let path = file.path();
    match file.status {
        carbon::FileStatus::Added => format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n"
        ),
        carbon::FileStatus::Deleted => format!(
            "diff --git a/{path} b/{path}\ndeleted file mode 100644\n--- a/{path}\n+++ /dev/null\n"
        ),
        _ => format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n"),
    }
}

fn append_carbon_hunk(patch: &mut String, hunk: &carbon::Hunk, carbon_file: &carbon::FileDiff) {
    use std::fmt::Write;

    let _ = write!(
        patch,
        "@@ -{},{} +{},{} @@\n",
        hunk.old_start, hunk.old_count, hunk.old_start, hunk.new_count
    );
    for block in carbon_file.hunk_blocks(hunk) {
        match block.kind {
            carbon::BlockKind::Context => {
                for offset in 0..block.old.len {
                    push_carbon_patch_line(
                        patch,
                        ' ',
                        carbon_file,
                        carbon::DiffSide::Old,
                        block.old.start.saturating_add(offset),
                    );
                }
            }
            carbon::BlockKind::Change => {
                for offset in 0..block.old.len {
                    push_carbon_patch_line(
                        patch,
                        '-',
                        carbon_file,
                        carbon::DiffSide::Old,
                        block.old.start.saturating_add(offset),
                    );
                }
                for offset in 0..block.new.len {
                    push_carbon_patch_line(
                        patch,
                        '+',
                        carbon_file,
                        carbon::DiffSide::New,
                        block.new.start.saturating_add(offset),
                    );
                }
            }
        }
    }
}

fn append_reversed_carbon_hunk(
    patch: &mut String,
    hunk: &carbon::Hunk,
    carbon_file: &carbon::FileDiff,
) {
    use std::fmt::Write;

    let _ = write!(
        patch,
        "@@ -{},{} +{},{} @@\n",
        hunk.new_start, hunk.new_count, hunk.new_start, hunk.old_count
    );
    for block in carbon_file.hunk_blocks(hunk) {
        match block.kind {
            carbon::BlockKind::Context => {
                for offset in 0..block.new.len {
                    push_carbon_patch_line(
                        patch,
                        ' ',
                        carbon_file,
                        carbon::DiffSide::New,
                        block.new.start.saturating_add(offset),
                    );
                }
            }
            carbon::BlockKind::Change => {
                for offset in 0..block.old.len {
                    push_carbon_patch_line(
                        patch,
                        '+',
                        carbon_file,
                        carbon::DiffSide::Old,
                        block.old.start.saturating_add(offset),
                    );
                }
                for offset in 0..block.new.len {
                    push_carbon_patch_line(
                        patch,
                        '-',
                        carbon_file,
                        carbon::DiffSide::New,
                        block.new.start.saturating_add(offset),
                    );
                }
            }
        }
    }
}

fn rewrite_carbon_hunk_for_lines(
    hunk: &carbon::Hunk,
    selected_lines: &[CarbonLineSelection],
    carbon_file: &carbon::FileDiff,
    reverse: bool,
) -> Option<String> {
    use std::fmt::Write;

    let selected: std::collections::HashSet<CarbonLineSelection> =
        selected_lines.iter().copied().collect();
    let mut old_count: i32 = 0;
    let mut new_count: i32 = 0;
    let mut body = String::new();
    let mut has_change = false;

    for block in carbon_file.hunk_blocks(hunk) {
        match block.kind {
            carbon::BlockKind::Context => {
                let count = block.old.len.min(block.new.len);
                for offset in 0..count {
                    old_count += 1;
                    new_count += 1;
                    push_carbon_patch_line(
                        &mut body,
                        ' ',
                        carbon_file,
                        if reverse {
                            carbon::DiffSide::New
                        } else {
                            carbon::DiffSide::Old
                        },
                        if reverse {
                            block.new.start.saturating_add(offset)
                        } else {
                            block.old.start.saturating_add(offset)
                        },
                    );
                }
            }
            carbon::BlockKind::Change => {
                for offset in 0..block.old.len {
                    let source_index = block.old.start.saturating_add(offset);
                    let is_selected = selected.contains(&CarbonLineSelection {
                        side: carbon::DiffSide::Old,
                        source_index,
                    });
                    if is_selected {
                        has_change = true;
                        if reverse {
                            new_count += 1;
                            push_carbon_patch_line(
                                &mut body,
                                '+',
                                carbon_file,
                                carbon::DiffSide::Old,
                                source_index,
                            );
                        } else {
                            old_count += 1;
                            push_carbon_patch_line(
                                &mut body,
                                '-',
                                carbon_file,
                                carbon::DiffSide::Old,
                                source_index,
                            );
                        }
                    } else if !reverse {
                        old_count += 1;
                        new_count += 1;
                        push_carbon_patch_line(
                            &mut body,
                            ' ',
                            carbon_file,
                            carbon::DiffSide::Old,
                            source_index,
                        );
                    }
                }
                for offset in 0..block.new.len {
                    let source_index = block.new.start.saturating_add(offset);
                    let is_selected = selected.contains(&CarbonLineSelection {
                        side: carbon::DiffSide::New,
                        source_index,
                    });
                    if is_selected {
                        has_change = true;
                        if reverse {
                            old_count += 1;
                            push_carbon_patch_line(
                                &mut body,
                                '-',
                                carbon_file,
                                carbon::DiffSide::New,
                                source_index,
                            );
                        } else {
                            new_count += 1;
                            push_carbon_patch_line(
                                &mut body,
                                '+',
                                carbon_file,
                                carbon::DiffSide::New,
                                source_index,
                            );
                        }
                    } else if reverse {
                        old_count += 1;
                        new_count += 1;
                        push_carbon_patch_line(
                            &mut body,
                            ' ',
                            carbon_file,
                            carbon::DiffSide::New,
                            source_index,
                        );
                    }
                }
            }
        }
    }

    if !has_change {
        return None;
    }

    let anchor = if reverse {
        hunk.new_start
    } else {
        hunk.old_start
    };

    let mut result = String::new();
    let _ = write!(
        result,
        "@@ -{},{} +{},{} @@\n",
        anchor, old_count, anchor, new_count
    );
    result.push_str(&body);
    Some(result)
}

fn push_carbon_patch_line(
    patch: &mut String,
    prefix: char,
    file: &carbon::FileDiff,
    side: carbon::DiffSide,
    source_index: u32,
) {
    patch.push(prefix);
    if let Some(text) = file
        .side_text(side)
        .and_then(|text| text.line_str(carbon::LineId(source_index)))
    {
        patch.push_str(text);
    }
    patch.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carbon_file() -> carbon::FileDiff {
        carbon::parse_unified_patch(
            "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 ctx
-old
+new
+extra
 tail
",
        )
        .unwrap()
        .files
        .into_iter()
        .next()
        .unwrap()
    }

    #[test]
    fn carbon_hunk_patch_reads_from_text_store() {
        let file = carbon_file();
        let patch = format_carbon_hunk_patch(&file, 0, false).unwrap();

        assert!(patch.starts_with("diff --git a/src/lib.rs b/src/lib.rs\n"));
        assert!(patch.contains("--- a/src/lib.rs\n"));
        assert!(patch.contains("+++ b/src/lib.rs\n"));
        assert!(patch.contains("@@ -1,3 +1,4 @@\n"));
        assert!(patch.contains(" ctx\n"));
        assert!(patch.contains("-old\n"));
        assert!(patch.contains("+new\n"));
        assert!(patch.contains("+extra\n"));
        assert!(patch.contains(" tail\n"));
    }

    #[test]
    fn carbon_reverse_hunk_patch_swaps_add_remove() {
        let file = carbon_file();
        let patch = format_carbon_hunk_patch(&file, 0, true).unwrap();

        assert!(patch.contains("@@ -1,4 +1,3 @@\n"));
        assert!(patch.contains("+old\n"));
        assert!(patch.contains("-new\n"));
        assert!(patch.contains("-extra\n"));
        assert!(patch.contains(" tail\n"));
    }

    #[test]
    fn carbon_line_patch_stages_selected_added_line() {
        let file = carbon_file();
        let patch = format_carbon_lines_patch(
            &file,
            0,
            &[CarbonLineSelection {
                side: carbon::DiffSide::New,
                source_index: 1,
            }],
            false,
        )
        .unwrap();

        assert!(patch.contains("+new\n"));
        assert!(!patch.contains("extra"));
        assert!(patch.contains("@@ -1,3 +1,4 @@\n"));
    }

    #[test]
    fn carbon_line_patch_returns_none_without_selected_changes() {
        let file = carbon_file();
        assert!(format_carbon_lines_patch(&file, 0, &[], false).is_none());
    }

    #[test]
    fn carbon_file_header_handles_added_and_deleted_files() {
        let added = carbon::parse_unified_patch(
            "\
diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hello
",
        )
        .unwrap();
        let deleted = carbon::parse_unified_patch(
            "\
diff --git a/old.txt b/old.txt
deleted file mode 100644
--- a/old.txt
+++ /dev/null
@@ -1 +0,0 @@
-goodbye
",
        )
        .unwrap();

        let added_patch = format_carbon_hunk_patch(&added.files[0], 0, false).unwrap();
        let deleted_patch = format_carbon_hunk_patch(&deleted.files[0], 0, false).unwrap();

        assert!(added_patch.contains("new file mode 100644"));
        assert!(added_patch.contains("--- /dev/null\n"));
        assert!(deleted_patch.contains("deleted file mode 100644"));
        assert!(deleted_patch.contains("+++ /dev/null\n"));
    }
}
