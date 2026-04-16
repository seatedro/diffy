use crate::core::diff::types::{FileDiff, Hunk, LineKind};
use crate::core::text::buffer::TextBuffer;

pub fn format_hunk_patch(
    file: &FileDiff,
    hunk_index: usize,
    text_buffer: &TextBuffer,
) -> Option<String> {
    let hunk = file.hunks.get(hunk_index)?;
    let mut patch = format_file_header(file);
    append_hunk(&mut patch, hunk, text_buffer);
    Some(patch)
}

pub fn format_lines_patch(
    file: &FileDiff,
    hunk_index: usize,
    selected_lines: &[usize],
    text_buffer: &TextBuffer,
    reverse: bool,
) -> Option<String> {
    let hunk = file.hunks.get(hunk_index)?;
    let mut patch = format_file_header(file);
    let rewritten = rewrite_hunk_for_lines(hunk, selected_lines, text_buffer, reverse)?;
    patch.push_str(&rewritten);
    Some(patch)
}

pub fn format_reverse_hunk_patch(
    file: &FileDiff,
    hunk_index: usize,
    text_buffer: &TextBuffer,
) -> Option<String> {
    let hunk = file.hunks.get(hunk_index)?;
    let mut patch = format_file_header(file);
    append_reversed_hunk(&mut patch, hunk, text_buffer);
    Some(patch)
}

fn format_file_header(file: &FileDiff) -> String {
    let path = &file.path;
    match file.status.as_str() {
        "A" => format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n"
        ),
        "D" => format!(
            "diff --git a/{path} b/{path}\ndeleted file mode 100644\n--- a/{path}\n+++ /dev/null\n"
        ),
        _ => format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n"),
    }
}

fn append_hunk(patch: &mut String, hunk: &Hunk, text_buffer: &TextBuffer) {
    use std::fmt::Write;
    let _ = write!(
        patch,
        "@@ -{},{} +{},{} @@\n",
        hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
    );
    for line in &hunk.lines {
        let text = text_buffer.view(line.text_range);
        let prefix = match line.kind {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
        };
        patch.push(prefix);
        patch.push_str(text);
        patch.push('\n');
    }
}

fn append_reversed_hunk(patch: &mut String, hunk: &Hunk, text_buffer: &TextBuffer) {
    use std::fmt::Write;
    let _ = write!(
        patch,
        "@@ -{},{} +{},{} @@\n",
        hunk.new_start, hunk.new_count, hunk.old_start, hunk.old_count
    );
    for line in &hunk.lines {
        let text = text_buffer.view(line.text_range);
        let prefix = match line.kind {
            LineKind::Context => ' ',
            LineKind::Added => '-',
            LineKind::Removed => '+',
        };
        patch.push(prefix);
        patch.push_str(text);
        patch.push('\n');
    }
}

fn rewrite_hunk_for_lines(
    hunk: &Hunk,
    selected_lines: &[usize],
    text_buffer: &TextBuffer,
    reverse: bool,
) -> Option<String> {
    use std::fmt::Write;

    let selected: std::collections::HashSet<usize> = selected_lines.iter().copied().collect();
    let mut old_count: i32 = 0;
    let mut new_count: i32 = 0;
    let mut body = String::new();
    let mut has_change = false;

    for (i, line) in hunk.lines.iter().enumerate() {
        let text = text_buffer.view(line.text_range);
        let is_selected = selected.contains(&i);

        match line.kind {
            LineKind::Context => {
                old_count += 1;
                new_count += 1;
                body.push(' ');
                body.push_str(text);
                body.push('\n');
            }
            LineKind::Removed => {
                if is_selected {
                    has_change = true;
                    if reverse {
                        new_count += 1;
                        body.push('+');
                    } else {
                        old_count += 1;
                        body.push('-');
                    }
                    body.push_str(text);
                    body.push('\n');
                } else if !reverse {
                    old_count += 1;
                    new_count += 1;
                    body.push(' ');
                    body.push_str(text);
                    body.push('\n');
                }
            }
            LineKind::Added => {
                if is_selected {
                    has_change = true;
                    if reverse {
                        old_count += 1;
                        body.push('-');
                    } else {
                        new_count += 1;
                        body.push('+');
                    }
                    body.push_str(text);
                    body.push('\n');
                } else if reverse {
                    old_count += 1;
                    new_count += 1;
                    body.push(' ');
                    body.push_str(text);
                    body.push('\n');
                }
            }
        }
    }

    if !has_change {
        return None;
    }

    let (old_start, new_start) = if reverse {
        (hunk.new_start, hunk.old_start)
    } else {
        (hunk.old_start, hunk.new_start)
    };

    let mut result = String::new();
    let _ = write!(
        result,
        "@@ -{},{} +{},{} @@\n",
        old_start, old_count, new_start, new_count
    );
    result.push_str(&body);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::diff::types::{DiffLine, FileDiff, Hunk, LineKind};
    use crate::core::text::buffer::TextBuffer;

    fn make_file(hunks: Vec<Hunk>) -> FileDiff {
        FileDiff {
            path: "src/lib.rs".to_owned(),
            status: "M".to_owned(),
            hunks,
            ..FileDiff::default()
        }
    }

    fn make_hunk(
        text_buffer: &mut TextBuffer,
        old_start: i32,
        old_count: i32,
        new_start: i32,
        new_count: i32,
        lines: &[(&str, LineKind)],
    ) -> Hunk {
        let mut old_line = old_start;
        let mut new_line = new_start;
        let diff_lines: Vec<DiffLine> = lines
            .iter()
            .map(|(text, kind)| {
                let text_range = text_buffer.append(text);
                let (old_no, new_no) = match kind {
                    LineKind::Context => {
                        let o = old_line;
                        let n = new_line;
                        old_line += 1;
                        new_line += 1;
                        (Some(o), Some(n))
                    }
                    LineKind::Removed => {
                        let o = old_line;
                        old_line += 1;
                        (Some(o), None)
                    }
                    LineKind::Added => {
                        let n = new_line;
                        new_line += 1;
                        (None, Some(n))
                    }
                };
                DiffLine {
                    kind: *kind,
                    old_line_number: old_no,
                    new_line_number: new_no,
                    text_range,
                    ..DiffLine::default()
                }
            })
            .collect();

        Hunk {
            old_start,
            old_count,
            new_start,
            new_count,
            header: format!(
                "@@ -{},{} +{},{} @@",
                old_start, old_count, new_start, new_count
            ),
            lines: diff_lines,
        }
    }

    #[test]
    fn format_hunk_patch_produces_valid_unified_diff() {
        let mut tb = TextBuffer::default();
        let hunk = make_hunk(
            &mut tb,
            1,
            3,
            1,
            4,
            &[
                ("int a = 1;", LineKind::Context),
                ("int b = 2;", LineKind::Removed),
                ("int b = 3;", LineKind::Added),
                ("int c = 4;", LineKind::Added),
                ("return a + b;", LineKind::Context),
            ],
        );
        let file = make_file(vec![hunk]);
        let patch = format_hunk_patch(&file, 0, &tb).unwrap();

        assert!(patch.starts_with("diff --git a/src/lib.rs b/src/lib.rs\n"));
        assert!(patch.contains("--- a/src/lib.rs\n"));
        assert!(patch.contains("+++ b/src/lib.rs\n"));
        assert!(patch.contains("@@ -1,3 +1,4 @@\n"));
        assert!(patch.contains(" int a = 1;\n"));
        assert!(patch.contains("-int b = 2;\n"));
        assert!(patch.contains("+int b = 3;\n"));
        assert!(patch.contains("+int c = 4;\n"));
        assert!(patch.contains(" return a + b;\n"));
    }

    #[test]
    fn format_reverse_hunk_patch_swaps_add_remove() {
        let mut tb = TextBuffer::default();
        let hunk = make_hunk(
            &mut tb,
            1,
            2,
            1,
            3,
            &[
                ("old line", LineKind::Removed),
                ("new line", LineKind::Added),
                ("extra", LineKind::Added),
                ("ctx", LineKind::Context),
            ],
        );
        let file = make_file(vec![hunk]);
        let patch = format_reverse_hunk_patch(&file, 0, &tb).unwrap();

        assert!(patch.contains("@@ -1,3 +1,2 @@\n"));
        assert!(patch.contains("+old line\n"));
        assert!(patch.contains("-new line\n"));
        assert!(patch.contains("-extra\n"));
        assert!(patch.contains(" ctx\n"));
    }

    #[test]
    fn format_hunk_patch_new_file() {
        let mut tb = TextBuffer::default();
        let hunk = make_hunk(&mut tb, 0, 0, 1, 1, &[("hello", LineKind::Added)]);
        let file = FileDiff {
            path: "new.txt".to_owned(),
            status: "A".to_owned(),
            hunks: vec![hunk],
            ..FileDiff::default()
        };
        let patch = format_hunk_patch(&file, 0, &tb).unwrap();

        assert!(patch.contains("new file mode 100644"));
        assert!(patch.contains("--- /dev/null\n"));
        assert!(patch.contains("+++ b/new.txt\n"));
    }

    #[test]
    fn format_hunk_patch_deleted_file() {
        let mut tb = TextBuffer::default();
        let hunk = make_hunk(&mut tb, 1, 1, 0, 0, &[("goodbye", LineKind::Removed)]);
        let file = FileDiff {
            path: "old.txt".to_owned(),
            status: "D".to_owned(),
            hunks: vec![hunk],
            ..FileDiff::default()
        };
        let patch = format_hunk_patch(&file, 0, &tb).unwrap();

        assert!(patch.contains("deleted file mode 100644"));
        assert!(patch.contains("--- a/old.txt\n"));
        assert!(patch.contains("+++ /dev/null\n"));
    }

    #[test]
    fn format_hunk_patch_invalid_index_returns_none() {
        let file = make_file(vec![]);
        let tb = TextBuffer::default();
        assert!(format_hunk_patch(&file, 0, &tb).is_none());
    }

    #[test]
    fn format_lines_patch_stages_selected_added_lines() {
        let mut tb = TextBuffer::default();
        let hunk = make_hunk(
            &mut tb,
            1,
            2,
            1,
            4,
            &[
                ("ctx", LineKind::Context),
                ("add1", LineKind::Added),
                ("add2", LineKind::Added),
                ("ctx2", LineKind::Context),
            ],
        );
        let file = make_file(vec![hunk]);
        let patch = format_lines_patch(&file, 0, &[1], &tb, false).unwrap();

        assert!(patch.contains("+add1\n"));
        assert!(!patch.contains("add2"));
        assert!(patch.contains("@@ -1,2 +1,3 @@\n"));
    }

    #[test]
    fn format_lines_patch_unselected_removed_becomes_context() {
        let mut tb = TextBuffer::default();
        let hunk = make_hunk(
            &mut tb,
            1,
            3,
            1,
            2,
            &[
                ("keep", LineKind::Removed),
                ("also_remove", LineKind::Removed),
                ("ctx", LineKind::Context),
            ],
        );
        let file = make_file(vec![hunk]);
        let patch = format_lines_patch(&file, 0, &[1], &tb, false).unwrap();

        assert!(patch.contains(" keep\n"));
        assert!(patch.contains("-also_remove\n"));
        assert!(patch.contains(" ctx\n"));
        assert!(patch.contains("@@ -1,3 +1,2 @@\n"));
    }

    #[test]
    fn format_lines_patch_no_selected_changes_returns_none() {
        let mut tb = TextBuffer::default();
        let hunk = make_hunk(&mut tb, 1, 1, 1, 1, &[("ctx", LineKind::Context)]);
        let file = make_file(vec![hunk]);
        assert!(format_lines_patch(&file, 0, &[], &tb, false).is_none());
    }

    #[test]
    fn format_lines_patch_reverse_swaps_directions() {
        let mut tb = TextBuffer::default();
        let hunk = make_hunk(
            &mut tb,
            1,
            2,
            1,
            2,
            &[
                ("ctx", LineKind::Context),
                ("removed", LineKind::Removed),
                ("added", LineKind::Added),
            ],
        );
        let file = make_file(vec![hunk]);
        let patch = format_lines_patch(&file, 0, &[1, 2], &tb, true).unwrap();

        assert!(patch.contains("+removed\n"));
        assert!(patch.contains("-added\n"));
        assert!(patch.contains("@@ -1,2 +1,2 @@\n"));
    }

    #[test]
    fn format_lines_patch_mixed_selection() {
        let mut tb = TextBuffer::default();
        let hunk = make_hunk(
            &mut tb,
            10,
            3,
            10,
            4,
            &[
                ("ctx_before", LineKind::Context),
                ("old_line", LineKind::Removed),
                ("new_line", LineKind::Added),
                ("extra_add", LineKind::Added),
                ("ctx_after", LineKind::Context),
            ],
        );
        let file = make_file(vec![hunk]);
        let patch = format_lines_patch(&file, 0, &[1, 2], &tb, false).unwrap();

        assert!(patch.contains("-old_line\n"));
        assert!(patch.contains("+new_line\n"));
        assert!(!patch.contains("extra_add"));
        assert!(patch.contains("@@ -10,3 +10,3 @@\n"));
    }

    #[test]
    fn format_lines_patch_reverse_keeps_unselected_added_as_context() {
        let mut tb = TextBuffer::default();
        let hunk = make_hunk(
            &mut tb,
            74,
            6,
            74,
            15,
            &[
                ("UnstageFile(usize),", LineKind::Context),
                ("StageAllFiles,", LineKind::Context),
                ("UnstageAllFiles,", LineKind::Context),
                ("StageHunk,", LineKind::Added),
                ("UnstageHunk,", LineKind::Added),
                ("DiscardHunk,", LineKind::Added),
                ("ToggleLineSelection(usize),", LineKind::Added),
                ("ToggleLineSelectionRange(usize, usize),", LineKind::Added),
                ("StageSelectedLines,", LineKind::Added),
                ("UnstageSelectedLines,", LineKind::Added),
                ("DiscardSelectedLines,", LineKind::Added),
                ("ClearLineSelection,", LineKind::Added),
                ("SelectFile(usize),", LineKind::Context),
                ("SelectFilePath(String),", LineKind::Context),
                ("SelectNextFile,", LineKind::Context),
            ],
        );
        let file = make_file(vec![hunk]);
        let patch = format_lines_patch(&file, 0, &[3], &tb, true).unwrap();

        assert!(patch.contains("-StageHunk,\n"));
        assert!(patch.contains(" UnstageHunk,\n"));
        assert!(patch.contains(" ClearLineSelection,\n"));
        assert!(patch.contains(" SelectFile(usize),\n"));
        assert!(patch.contains("@@ -74,15 +74,14 @@\n"));
    }

    #[test]
    fn format_lines_patch_forward_skips_unselected_removed() {
        let mut tb = TextBuffer::default();
        let hunk = make_hunk(
            &mut tb,
            1,
            3,
            1,
            1,
            &[
                ("ctx", LineKind::Context),
                ("remove_a", LineKind::Removed),
                ("remove_b", LineKind::Removed),
            ],
        );
        let file = make_file(vec![hunk]);
        let patch = format_lines_patch(&file, 0, &[1], &tb, true).unwrap();

        assert!(patch.contains(" ctx\n"));
        assert!(patch.contains("+remove_a\n"));
        assert!(!patch.contains("remove_b"));
        assert!(patch.contains("@@ -1,1 +1,2 @@\n"));
    }
}
