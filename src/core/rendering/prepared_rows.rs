use crate::core::diff::types::{DiffLine, FileDiff};
use crate::core::rendering::flat_rows::{DiffRowType, FlatDiffRow};
use crate::core::text::buffer::{TextBuffer, TextRange};
use crate::core::text::token::TokenRange;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PreparedRowsCacheKey {
    pub file_index: i32,
    pub compare_generation: i32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PreparedRow {
    pub flat: FlatDiffRow,
    pub text_range: TextRange,
    pub syntax_tokens: TokenRange,
    pub change_tokens: TokenRange,
    pub measured_width: f64,
}

pub fn prepare_rows(
    flat_rows: &[FlatDiffRow],
    file_diffs: &[FileDiff],
    text_buffer: &TextBuffer,
    measure_fn: &dyn Fn(&str) -> f64,
) -> Vec<PreparedRow> {
    let mut prepared = Vec::with_capacity(flat_rows.len());

    for flat in flat_rows {
        let Some(file) = index_i32(file_diffs, flat.file_index) else {
            prepared.push(PreparedRow {
                flat: flat.clone(),
                ..PreparedRow::default()
            });
            continue;
        };

        let (text_range, syntax_tokens, change_tokens, measured_width) = match flat.row_type {
            DiffRowType::FileHeader => {
                let measured_width = measure_fn(&file.path);
                (
                    TextRange::default(),
                    TokenRange::default(),
                    TokenRange::default(),
                    measured_width,
                )
            }
            DiffRowType::HunkSeparator => {
                let header = hunk_for_flat(file, flat).map_or("", |hunk| hunk.header.as_str());
                let measured_width = measure_fn(header);
                (
                    TextRange::default(),
                    TokenRange::default(),
                    TokenRange::default(),
                    measured_width,
                )
            }
            DiffRowType::Modified => prepare_modified_row(file, flat, text_buffer, measure_fn),
            DiffRowType::Context | DiffRowType::Added | DiffRowType::Removed => {
                if let Some(line) = primary_line_for_flat(file, flat) {
                    let measured_width = measure_fn(text_buffer.view(line.text_range));
                    (
                        line.text_range,
                        line.syntax_tokens,
                        line.change_tokens,
                        measured_width,
                    )
                } else {
                    (
                        TextRange::default(),
                        TokenRange::default(),
                        TokenRange::default(),
                        0.0,
                    )
                }
            }
        };

        prepared.push(PreparedRow {
            flat: flat.clone(),
            text_range,
            syntax_tokens,
            change_tokens,
            measured_width,
        });
    }

    prepared
}

fn prepare_modified_row(
    file: &FileDiff,
    flat: &FlatDiffRow,
    text_buffer: &TextBuffer,
    measure_fn: &dyn Fn(&str) -> f64,
) -> (TextRange, TokenRange, TokenRange, f64) {
    let removed = line_by_index(file, flat.hunk_index, flat.old_line_index);
    let added = line_by_index(file, flat.hunk_index, flat.new_line_index);

    let removed_width = removed.map_or(0.0, |line| measure_fn(text_buffer.view(line.text_range)));
    let added_width = added.map_or(0.0, |line| measure_fn(text_buffer.view(line.text_range)));
    let measured_width = removed_width.max(added_width);

    let primary = added.or(removed);
    if let Some(line) = primary {
        (
            line.text_range,
            line.syntax_tokens,
            line.change_tokens,
            measured_width,
        )
    } else {
        (
            TextRange::default(),
            TokenRange::default(),
            TokenRange::default(),
            measured_width,
        )
    }
}

fn primary_line_for_flat<'a>(file: &'a FileDiff, flat: &FlatDiffRow) -> Option<&'a DiffLine> {
    let line_index = if flat.line_index >= 0 {
        flat.line_index
    } else if flat.new_line_index >= 0 {
        flat.new_line_index
    } else {
        flat.old_line_index
    };
    line_by_index(file, flat.hunk_index, line_index)
}

fn hunk_for_flat<'a>(
    file: &'a FileDiff,
    flat: &FlatDiffRow,
) -> Option<&'a crate::core::diff::types::Hunk> {
    index_i32(&file.hunks, flat.hunk_index)
}

fn line_by_index(file: &FileDiff, hunk_index: i32, line_index: i32) -> Option<&DiffLine> {
    let hunk = index_i32(&file.hunks, hunk_index)?;
    index_i32(&hunk.lines, line_index)
}

fn index_i32<T>(slice: &[T], index: i32) -> Option<&T> {
    usize::try_from(index)
        .ok()
        .and_then(|index| slice.get(index))
}

#[cfg(test)]
mod tests {
    use super::prepare_rows;
    use crate::core::diff::types::{DiffLine, FileDiff, Hunk, LineKind};
    use crate::core::rendering::flat_rows::{DiffRowType, flatten_file_diff};
    use crate::core::text::buffer::TextBuffer;
    use crate::core::text::token::{DiffTokenSpan, SyntaxTokenKind, TokenBuffer};

    #[test]
    fn prepares_rows_from_file_diff_and_text_buffer() {
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        let removed_range = text_buffer.append("old value");
        let added_range = text_buffer.append("new value");
        let removed_tokens = token_buffer.append(&[DiffTokenSpan {
            offset: 0,
            length: 3,
            kind: SyntaxTokenKind::Keyword,
        }]);
        let added_changes = token_buffer.append(&[DiffTokenSpan {
            offset: 4,
            length: 5,
            kind: SyntaxTokenKind::String,
        }]);

        let file = FileDiff {
            path: "src/lib.rs".to_owned(),
            hunks: vec![Hunk {
                header: "@@ -1 +1 @@".to_owned(),
                lines: vec![
                    DiffLine {
                        kind: LineKind::Removed,
                        old_line_number: Some(1),
                        text_range: removed_range,
                        syntax_tokens: removed_tokens,
                        ..DiffLine::default()
                    },
                    DiffLine {
                        kind: LineKind::Added,
                        new_line_number: Some(1),
                        text_range: added_range,
                        change_tokens: added_changes,
                        ..DiffLine::default()
                    },
                ],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        };

        let rows = flatten_file_diff(&file, 0);
        let prepared = prepare_rows(&rows, &[file], &text_buffer, &|text| text.len() as f64);

        assert_eq!(prepared.len(), 3);
        assert_eq!(prepared[0].flat.row_type, DiffRowType::FileHeader);
        assert_eq!(prepared[0].measured_width, 10.0);
        assert_eq!(prepared[1].flat.row_type, DiffRowType::HunkSeparator);
        assert_eq!(prepared[1].measured_width, 11.0);
        assert_eq!(prepared[2].flat.row_type, DiffRowType::Modified);
        assert_eq!(prepared[2].measured_width, 9.0);
        assert_eq!(text_buffer.view(prepared[2].text_range), "new value");
        assert_eq!(prepared[2].change_tokens, added_changes);
    }
}
