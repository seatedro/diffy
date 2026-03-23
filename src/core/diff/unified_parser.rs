use crate::core::diff::types::{DiffDocument, DiffLine, FileDiff, Hunk, LineKind};
use crate::core::text::buffer::TextBuffer;

pub fn parse(input: &str) -> DiffDocument {
    let mut text_buffer = TextBuffer::default();
    parse_into(input, &mut text_buffer)
}

pub fn parse_into(input: &str, text_buffer: &mut TextBuffer) -> DiffDocument {
    let mut document = DiffDocument::default();
    let mut current_file_index: Option<usize> = None;
    let mut current_hunk_index: Option<usize> = None;
    let mut old_line = 0_i32;
    let mut new_line = 0_i32;

    for raw_line in input.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

        if let Some(path) = parse_diff_git_header(line) {
            document.files.push(FileDiff {
                path,
                status: "M".to_owned(),
                ..FileDiff::default()
            });
            current_file_index = Some(document.files.len() - 1);
            current_hunk_index = None;
            continue;
        }

        let Some(file_index) = current_file_index else {
            continue;
        };
        let file = &mut document.files[file_index];

        if line.starts_with("new file mode ") {
            file.status = "A".to_owned();
            continue;
        }
        if line.starts_with("deleted file mode ") {
            file.status = "D".to_owned();
            continue;
        }
        if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            file.is_binary = true;
            continue;
        }
        if line.starts_with("rename from ") || line.starts_with("rename to ") {
            file.status = "R".to_owned();
            continue;
        }
        if let Some(path) = parse_file_marker(line) {
            if !path.is_empty() {
                file.path = path;
            }
            continue;
        }

        if let Some((old_start, old_count, new_start, new_count)) = parse_hunk_header(line) {
            file.hunks.push(Hunk {
                old_start,
                old_count,
                new_start,
                new_count,
                header: line.to_owned(),
                ..Hunk::default()
            });
            current_hunk_index = Some(file.hunks.len() - 1);
            old_line = old_start;
            new_line = new_start;
            continue;
        }

        if line.starts_with(r"\ No newline at end of file") {
            continue;
        }

        let Some(hunk_index) = current_hunk_index else {
            continue;
        };
        let hunk = &mut file.hunks[hunk_index];

        let (kind, text, old_number, new_number) = match line.as_bytes().first().copied() {
            Some(b'+') => {
                let text = &line[1..];
                let line_no = new_line;
                new_line += 1;
                file.additions += 1;
                (LineKind::Added, text, None, Some(line_no))
            }
            Some(b'-') => {
                let text = &line[1..];
                let line_no = old_line;
                old_line += 1;
                file.deletions += 1;
                (LineKind::Removed, text, Some(line_no), None)
            }
            Some(b' ') => {
                let text = &line[1..];
                let old_number = old_line;
                let new_number = new_line;
                old_line += 1;
                new_line += 1;
                (LineKind::Context, text, Some(old_number), Some(new_number))
            }
            _ => {
                let old_number = old_line;
                let new_number = new_line;
                old_line += 1;
                new_line += 1;
                (LineKind::Context, line, Some(old_number), Some(new_number))
            }
        };

        let text_range = text_buffer.append(text);
        hunk.lines.push(DiffLine {
            kind,
            old_line_number: old_number,
            new_line_number: new_number,
            text_range,
            ..DiffLine::default()
        });
    }

    document
}

fn parse_diff_git_header(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "diff" || parts.next()? != "--git" {
        return None;
    }
    let _old_path = parts.next()?;
    let new_path = parts.next()?;
    Some(strip_diff_path_prefix(new_path).to_owned())
}

fn parse_file_marker(line: &str) -> Option<String> {
    let marker = if line.starts_with("--- ") || line.starts_with("+++ ") {
        &line[4..]
    } else {
        return None;
    };
    if marker == "/dev/null" {
        return Some(String::new());
    }
    Some(strip_diff_path_prefix(marker).to_owned())
}

fn strip_diff_path_prefix(path: &str) -> &str {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
}

fn parse_hunk_header(line: &str) -> Option<(i32, i32, i32, i32)> {
    if !line.starts_with("@@ ") {
        return None;
    }
    let mut parts = line.split_whitespace();
    let _ = parts.next()?;
    let old_part = parts.next()?;
    let new_part = parts.next()?;
    let (old_start, old_count) = parse_hunk_range(old_part, '-')?;
    let (new_start, new_count) = parse_hunk_range(new_part, '+')?;
    Some((old_start, old_count, new_start, new_count))
}

fn parse_hunk_range(part: &str, prefix: char) -> Option<(i32, i32)> {
    let value = part.strip_prefix(prefix)?;
    let (start, count) = value.split_once(',').map_or((value, "1"), |(start, count)| (start, count));
    Some((start.parse().ok()?, count.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::{parse, parse_into};
    use crate::core::diff::types::LineKind;
    use crate::core::text::TextBuffer;

    #[test]
    fn parses_single_file_patch() {
        let patch = concat!(
            "diff --git a/src/a.cpp b/src/a.cpp
",
            "index 111..222 100644
",
            "--- a/src/a.cpp
",
            "+++ b/src/a.cpp
",
            "@@ -1,3 +1,4 @@
",
            " int a = 1;
",
            "-int b = 2;
",
            "+int b = 3;
",
            "+int c = 4;
",
            " return a + b;
",
        );

        let document = parse(patch);
        assert_eq!(document.files.len(), 1);

        let file = &document.files[0];
        assert_eq!(file.path, "src/a.cpp");
        assert_eq!(file.additions, 2);
        assert_eq!(file.deletions, 1);
        assert_eq!(file.hunks.len(), 1);

        let hunk = &file.hunks[0];
        assert_eq!((hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count), (1, 3, 1, 4));
        assert_eq!(hunk.lines.len(), 5);
        assert_eq!(hunk.lines[1].kind, LineKind::Removed);
        assert_eq!(hunk.lines[2].kind, LineKind::Added);
        assert_eq!(hunk.lines[1].old_line_number, Some(2));
        assert_eq!(hunk.lines[2].new_line_number, Some(2));
    }

    #[test]
    fn parse_into_populates_text_buffer() {
        let patch = concat!(
            "diff --git a/a.py b/a.py
",
            "@@ -1 +1 @@
",
            "-print(\"old\")\n",
            "+print(\"new\")\n",
        );
        let mut text_buffer = TextBuffer::default();
        let document = parse_into(patch, &mut text_buffer);
        let removed = &document.files[0].hunks[0].lines[0];
        let added = &document.files[0].hunks[0].lines[1];
        assert_eq!(text_buffer.view(removed.text_range), "print(\"old\")");
        assert_eq!(text_buffer.view(added.text_range), "print(\"new\")");
    }
}
