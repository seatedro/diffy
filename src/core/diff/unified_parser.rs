use crate::core::diff::types::{DiffDocument, DiffLine, FileDiff, Hunk, LineKind};
use crate::core::text::buffer::TextBuffer;
use crate::core::text::{ChangeIntensity, DiffTokenSpan, SyntaxTokenKind, TokenBuffer};

pub fn parse(input: &str) -> DiffDocument {
    let mut text_buffer = TextBuffer::default();
    parse_into(input, &mut text_buffer)
}

pub fn parse_into(input: &str, text_buffer: &mut TextBuffer) -> DiffDocument {
    match carbon::parse_unified_patch(input) {
        Ok(document) => lower_carbon_document(&document, text_buffer, None),
        Err(_) => DiffDocument::default(),
    }
}

pub fn lower_carbon_document(
    document: &carbon::DiffDocument,
    text_buffer: &mut TextBuffer,
    token_buffer: Option<&mut TokenBuffer>,
) -> DiffDocument {
    let mut token_buffer = token_buffer;
    DiffDocument {
        files: document
            .files
            .iter()
            .map(|file| lower_carbon_file(file, text_buffer, token_buffer.as_deref_mut()))
            .collect(),
    }
}

pub fn lower_carbon_file(
    file: &carbon::FileDiff,
    text_buffer: &mut TextBuffer,
    mut token_buffer: Option<&mut TokenBuffer>,
) -> FileDiff {
    let mut lowered = FileDiff {
        path: file.path().to_owned(),
        status: legacy_status(file.status),
        is_binary: file.is_binary,
        ..FileDiff::default()
    };

    for hunk in &file.hunks {
        let mut lowered_hunk = Hunk {
            old_start: u32_to_i32_saturating(hunk.old_start),
            old_count: u32_to_i32_saturating(hunk.old_count),
            new_start: u32_to_i32_saturating(hunk.new_start),
            new_count: u32_to_i32_saturating(hunk.new_count),
            header: hunk.header.clone(),
            ..Hunk::default()
        };

        for block in file.hunk_blocks(hunk) {
            match block.kind {
                carbon::BlockKind::Context => {
                    let count = block.old.len.min(block.new.len);
                    for offset in 0..count {
                        let text = line_text(file, carbon::DiffSide::Old, block.old.start + offset)
                            .or_else(|| {
                                line_text(file, carbon::DiffSide::New, block.new.start + offset)
                            })
                            .unwrap_or_default();
                        let text_range = text_buffer.append(text);
                        lowered_hunk.lines.push(DiffLine {
                            kind: LineKind::Context,
                            old_line_number: Some(u32_to_i32_saturating(
                                block.old_line_start + offset,
                            )),
                            new_line_number: Some(u32_to_i32_saturating(
                                block.new_line_start + offset,
                            )),
                            text_range,
                            ..DiffLine::default()
                        });
                    }
                }
                carbon::BlockKind::Change => {
                    for offset in 0..block.old.len {
                        let text = line_text(file, carbon::DiffSide::Old, block.old.start + offset)
                            .unwrap_or_default();
                        let text_range = text_buffer.append(text);
                        let change_tokens =
                            append_whole_line_change_token(token_buffer.as_deref_mut(), text);
                        lowered_hunk.lines.push(DiffLine {
                            kind: LineKind::Removed,
                            old_line_number: Some(u32_to_i32_saturating(
                                block.old_line_start + offset,
                            )),
                            text_range,
                            change_tokens,
                            ..DiffLine::default()
                        });
                        lowered.deletions = lowered.deletions.saturating_add(1);
                    }
                    for offset in 0..block.new.len {
                        let text = line_text(file, carbon::DiffSide::New, block.new.start + offset)
                            .unwrap_or_default();
                        let text_range = text_buffer.append(text);
                        let change_tokens =
                            append_whole_line_change_token(token_buffer.as_deref_mut(), text);
                        lowered_hunk.lines.push(DiffLine {
                            kind: LineKind::Added,
                            new_line_number: Some(u32_to_i32_saturating(
                                block.new_line_start + offset,
                            )),
                            text_range,
                            change_tokens,
                            ..DiffLine::default()
                        });
                        lowered.additions = lowered.additions.saturating_add(1);
                    }
                }
            }
        }

        lowered.hunks.push(lowered_hunk);
    }

    lowered
}

fn line_text(file: &carbon::FileDiff, side: carbon::DiffSide, index: u32) -> Option<&str> {
    file.side_text(side)?.line_str(carbon::LineId(index))
}

fn append_whole_line_change_token(
    token_buffer: Option<&mut TokenBuffer>,
    text: &str,
) -> crate::core::text::TokenRange {
    let Some(token_buffer) = token_buffer else {
        return Default::default();
    };
    token_buffer.append(&[DiffTokenSpan {
        offset: 0,
        length: usize_to_u32_saturating(text.len()),
        kind: SyntaxTokenKind::Normal,
        intensity: ChangeIntensity::NovelWord,
    }])
}

fn legacy_status(status: carbon::FileStatus) -> String {
    match status {
        carbon::FileStatus::Added => "A",
        carbon::FileStatus::Deleted => "D",
        carbon::FileStatus::Renamed | carbon::FileStatus::RenamedModified => "R",
        carbon::FileStatus::Binary => "B",
        carbon::FileStatus::Modified | carbon::FileStatus::ModeChanged => "M",
    }
    .to_owned()
}

fn u32_to_i32_saturating(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn usize_to_u32_saturating(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
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
        assert_eq!(
            (
                hunk.old_start,
                hunk.old_count,
                hunk.new_start,
                hunk.new_count
            ),
            (1, 3, 1, 4)
        );
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

    #[test]
    fn parse_into_accepts_crlf_patch_text() {
        let patch = "diff --git a/a.txt b/a.txt\r\n@@ -1 +1 @@\r\n-old\r\n+new\r\n";
        let mut text_buffer = TextBuffer::default();
        let document = parse_into(patch, &mut text_buffer);

        assert_eq!(document.files.len(), 1);
        assert_eq!(document.files[0].hunks[0].lines.len(), 2);
        assert_eq!(
            text_buffer.view(document.files[0].hunks[0].lines[0].text_range),
            "old"
        );
        assert_eq!(
            text_buffer.view(document.files[0].hunks[0].lines[1].text_range),
            "new"
        );
    }
}
