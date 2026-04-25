use crate::core::diff::{FileDiff, Hunk, LineKind};
use crate::core::syntax::Highlighter;
use crate::core::text::{DiffTokenSpan, TextBuffer, TokenBuffer};

#[derive(Debug, Clone, Copy)]
struct LineRef {
    hunk_index: usize,
    line_index: usize,
    side: Option<carbon::DiffSide>,
    source_index: Option<u32>,
    content_offset: usize,
    content_len: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyntaxRowWindow {
    pub start: usize,
    pub end: usize,
}

impl SyntaxRowWindow {
    pub const fn contains(self, other: Self) -> bool {
        self.start <= other.start && self.end >= other.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxLineTokens {
    pub hunk_index: usize,
    pub line_index: usize,
    pub side: Option<carbon::DiffSide>,
    pub source_index: Option<u32>,
    pub tokens: Vec<DiffTokenSpan>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FullFileSyntax {
    line_offsets: Vec<usize>,
    line_lengths: Vec<usize>,
    tokens: Vec<DiffTokenSpan>,
}

impl FullFileSyntax {
    pub fn has_tokens(&self) -> bool {
        !self.tokens.is_empty()
    }
}

#[derive(Debug)]
pub struct DiffSyntaxAnnotator {
    highlighter: Highlighter,
}

impl Default for DiffSyntaxAnnotator {
    fn default() -> Self {
        Self::new()
    }
}

impl DiffSyntaxAnnotator {
    pub fn new() -> Self {
        Self {
            highlighter: Highlighter::new(),
        }
    }

    pub fn annotate(
        &self,
        file_diff: &mut FileDiff,
        text_buffer: &mut TextBuffer,
        token_buffer: &mut TokenBuffer,
    ) {
        if file_diff.is_binary {
            return;
        }

        let (old_content, new_content, old_refs, new_refs) =
            build_line_refs(&file_diff.hunks, text_buffer);
        let language = self.highlighter.resolve_language(&file_diff.path);

        let old_tokens = match self.highlighter.highlight_resolved(language, &old_content) {
            Ok(tokens) => tokens,
            Err(error) => {
                tracing::warn!(
                    path = %file_diff.path,
                    ?language,
                    %error,
                    "syntax highlight failed"
                );
                Vec::new()
            }
        };
        let new_tokens = match self.highlighter.highlight_resolved(language, &new_content) {
            Ok(tokens) => tokens,
            Err(error) => {
                tracing::warn!(
                    path = %file_diff.path,
                    ?language,
                    %error,
                    "syntax highlight failed"
                );
                Vec::new()
            }
        };
        distribute_tokens(&mut file_diff.hunks, token_buffer, &old_tokens, &old_refs);
        distribute_tokens(&mut file_diff.hunks, token_buffer, &new_tokens, &new_refs);
    }

    pub fn annotate_files(
        &self,
        files: &mut [FileDiff],
        text_buffer: &mut TextBuffer,
        token_buffer: &mut TokenBuffer,
    ) {
        for file in files {
            self.annotate(file, text_buffer, token_buffer);
        }
    }

    pub fn highlight_full_lines(&self, path: &str, lines: &[String]) -> FullFileSyntax {
        let (source, line_offsets) = source_from_lines(lines);
        let line_lengths = lines.iter().map(|line| line.len()).collect::<Vec<_>>();
        let language = self.highlighter.resolve_language(path);
        let tokens = match self.highlighter.highlight_resolved(language, &source) {
            Ok(tokens) => tokens,
            Err(error) => {
                tracing::warn!(
                    path = %path,
                    ?language,
                    %error,
                    "syntax highlight failed"
                );
                Vec::new()
            }
        };

        FullFileSyntax {
            line_offsets,
            line_lengths,
            tokens,
        }
    }

    pub fn annotate_carbon_full_file_window_from_cache(
        &self,
        file: &carbon::FileDiff,
        file_index: usize,
        old_syntax: Option<&FullFileSyntax>,
        new_syntax: Option<&FullFileSyntax>,
        window: SyntaxRowWindow,
    ) -> Vec<SyntaxLineTokens> {
        if file.is_binary || window.end <= window.start {
            return Vec::new();
        }

        let (old_refs, new_refs) = build_carbon_full_file_refs(file, file_index, window);
        let mut out = Vec::new();
        if let Some(syntax) = old_syntax {
            let byte_refs = byte_refs_for_cached_refs(&old_refs, syntax);
            out.extend(collect_line_tokens(&syntax.tokens, &byte_refs));
        }
        if let Some(syntax) = new_syntax {
            let byte_refs = byte_refs_for_cached_refs(&new_refs, syntax);
            out.extend(collect_line_tokens(&syntax.tokens, &byte_refs));
        }
        out
    }
}

fn build_line_refs(
    hunks: &[Hunk],
    text_buffer: &TextBuffer,
) -> (String, String, Vec<LineRef>, Vec<LineRef>) {
    let mut old_content = String::new();
    let mut new_content = String::new();
    let mut old_refs = Vec::new();
    let mut new_refs = Vec::new();

    for (hunk_index, hunk) in hunks.iter().enumerate() {
        for (line_index, line) in hunk.lines.iter().enumerate() {
            let text = text_buffer.view(line.text_range);
            if matches!(line.kind, LineKind::Context | LineKind::Removed) {
                let offset = old_content.len();
                old_content.push_str(text);
                old_content.push('\n');
                old_refs.push(LineRef {
                    hunk_index,
                    line_index,
                    side: None,
                    source_index: None,
                    content_offset: offset,
                    content_len: text.len(),
                });
            }
            if matches!(line.kind, LineKind::Context | LineKind::Added) {
                let offset = new_content.len();
                new_content.push_str(text);
                new_content.push('\n');
                new_refs.push(LineRef {
                    hunk_index,
                    line_index,
                    side: None,
                    source_index: None,
                    content_offset: offset,
                    content_len: text.len(),
                });
            }
        }
    }

    (old_content, new_content, old_refs, new_refs)
}

fn distribute_tokens(
    hunks: &mut [Hunk],
    token_buffer: &mut TokenBuffer,
    tokens: &[DiffTokenSpan],
    line_refs: &[LineRef],
) {
    let mut token_index = 0usize;
    for reference in line_refs {
        let line_start = reference.content_offset;
        let line_end = line_start + reference.content_len;
        while token_index < tokens.len() && token_end_usize(tokens[token_index]) <= line_start {
            token_index += 1;
        }

        let mut line_tokens = Vec::new();
        for token in tokens.iter().skip(token_index) {
            let start = u32_to_usize_saturating(token.offset);
            if start >= line_end {
                break;
            }
            let end = start.saturating_add(u32_to_usize_saturating(token.length));
            let clipped_start = start.max(line_start);
            let clipped_end = end.min(line_end);
            if clipped_end <= clipped_start {
                continue;
            }
            line_tokens.push(DiffTokenSpan {
                offset: usize_to_u32_saturating(clipped_start - line_start),
                length: usize_to_u32_saturating(clipped_end - clipped_start),
                kind: token.kind,
                ..DiffTokenSpan::default()
            });
        }

        if !line_tokens.is_empty() {
            let range = token_buffer.append(&line_tokens);
            hunks[reference.hunk_index].lines[reference.line_index].syntax_tokens = range;
        }
    }
}

fn build_carbon_full_file_refs(
    file: &carbon::FileDiff,
    _file_index: usize,
    window: SyntaxRowWindow,
) -> (Vec<LineRef>, Vec<LineRef>) {
    let mut old_refs = Vec::new();
    let mut new_refs = Vec::new();
    let mut projected_index = 1usize;

    carbon::project_file(
        file,
        carbon::ProjectionOptions {
            mode: carbon::ProjectionMode::Unified,
            collapsed_context_threshold: 0,
            include_hunk_headers: true,
        },
        &carbon::ExpansionState::default(),
        |row| {
            let in_window = projected_index >= window.start && projected_index < window.end;
            projected_index = projected_index.saturating_add(1);
            if !in_window {
                return;
            }
            let hunk_index = row
                .hunk_id
                .map(|id| carbon::u32_to_usize_saturating(id.0))
                .unwrap_or_default();
            if let (Some(source_index), Some(line_no)) = (row.old_index, row.old_line) {
                old_refs.push(LineRef {
                    hunk_index,
                    line_index: carbon::u32_to_usize_saturating(source_index),
                    side: Some(carbon::DiffSide::Old),
                    source_index: Some(source_index),
                    content_offset: carbon::u32_to_usize_saturating(line_no.saturating_sub(1)),
                    content_len: 0,
                });
            }
            if let (Some(source_index), Some(line_no)) = (row.new_index, row.new_line) {
                new_refs.push(LineRef {
                    hunk_index,
                    line_index: carbon::u32_to_usize_saturating(source_index),
                    side: Some(carbon::DiffSide::New),
                    source_index: Some(source_index),
                    content_offset: carbon::u32_to_usize_saturating(line_no.saturating_sub(1)),
                    content_len: 0,
                });
            }
        },
    );

    (old_refs, new_refs)
}

fn source_from_lines(lines: &[String]) -> (String, Vec<usize>) {
    let size = lines.iter().map(|line| line.len() + 1).sum();
    let mut source = String::with_capacity(size);
    let mut offsets = Vec::with_capacity(lines.len() + 1);
    for line in lines {
        offsets.push(source.len());
        source.push_str(line);
        source.push('\n');
    }
    offsets.push(source.len());
    (source, offsets)
}

fn byte_refs_for_cached_refs(refs: &[LineRef], syntax: &FullFileSyntax) -> Vec<LineRef> {
    refs.iter()
        .filter_map(|reference| {
            let line_index = reference.content_offset;
            Some(LineRef {
                hunk_index: reference.hunk_index,
                line_index: reference.line_index,
                side: reference.side,
                source_index: reference.source_index,
                content_offset: *syntax.line_offsets.get(line_index)?,
                content_len: *syntax.line_lengths.get(line_index)?,
            })
        })
        .collect()
}

fn usize_to_u32_saturating(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn u32_to_usize_saturating(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn token_end_usize(token: DiffTokenSpan) -> usize {
    u32_to_usize_saturating(token.offset.saturating_add(token.length))
}

fn collect_line_tokens(tokens: &[DiffTokenSpan], refs: &[LineRef]) -> Vec<SyntaxLineTokens> {
    let mut token_index = 0usize;
    let mut out = Vec::new();
    for reference in refs {
        let line_start = reference.content_offset;
        let line_end = line_start + reference.content_len;
        while token_index < tokens.len() && token_end_usize(tokens[token_index]) <= line_start {
            token_index += 1;
        }

        let mut line_tokens = Vec::new();
        for token in tokens.iter().skip(token_index) {
            let start = u32_to_usize_saturating(token.offset);
            if start >= line_end {
                break;
            }
            let end = start.saturating_add(u32_to_usize_saturating(token.length));
            let clipped_start = start.max(line_start);
            let clipped_end = end.min(line_end);
            if clipped_end <= clipped_start {
                continue;
            }
            line_tokens.push(DiffTokenSpan {
                offset: usize_to_u32_saturating(clipped_start - line_start),
                length: usize_to_u32_saturating(clipped_end - clipped_start),
                kind: token.kind,
                ..DiffTokenSpan::default()
            });
        }

        out.push(SyntaxLineTokens {
            hunk_index: reference.hunk_index,
            line_index: reference.line_index,
            side: reference.side,
            source_index: reference.source_index,
            tokens: line_tokens,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::core::diff::{DiffDocument, DiffLine, FileDiff, Hunk, LineKind};
    use crate::core::syntax::DiffSyntaxAnnotator;
    use crate::core::text::{TextBuffer, TokenBuffer};

    fn test_document(
        path: &str,
        lines: &[(&str, LineKind, Option<i32>, Option<i32>)],
        text_buffer: &mut TextBuffer,
    ) -> DiffDocument {
        DiffDocument {
            files: vec![FileDiff {
                path: path.to_owned(),
                status: "M".to_owned(),
                hunks: vec![Hunk {
                    old_start: 1,
                    old_count: lines
                        .iter()
                        .filter(|(_, kind, _, _)| {
                            matches!(kind, LineKind::Context | LineKind::Removed)
                        })
                        .count() as i32,
                    new_start: 1,
                    new_count: lines
                        .iter()
                        .filter(|(_, kind, _, _)| {
                            matches!(kind, LineKind::Context | LineKind::Added)
                        })
                        .count() as i32,
                    header: "@@".to_owned(),
                    lines: lines
                        .iter()
                        .map(|(text, kind, old_line_number, new_line_number)| DiffLine {
                            kind: *kind,
                            old_line_number: *old_line_number,
                            new_line_number: *new_line_number,
                            text_range: text_buffer.append(text),
                            ..DiffLine::default()
                        })
                        .collect(),
                }],
                ..FileDiff::default()
            }],
        }
    }

    #[test]
    fn annotator_degrades_missing_packs_to_plain_text() {
        let highlighter = phosphor::Highlighter::new();
        if highlighter.is_parser_available(phosphor::LanguageId::Json) {
            return;
        }

        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();
        let mut document = test_document(
            "test.json",
            &[
                ("{", LineKind::Context, Some(1), Some(1)),
                ("  \"name\": \"old\"", LineKind::Removed, Some(2), None),
                ("  \"name\": \"new\",", LineKind::Added, None, Some(2)),
                ("  \"fast\": true", LineKind::Added, None, Some(3)),
            ],
            &mut text_buffer,
        );

        let annotator = DiffSyntaxAnnotator::new();
        annotator.annotate(&mut document.files[0], &mut text_buffer, &mut token_buffer);

        let token_kinds = document.files[0]
            .hunks
            .iter()
            .flat_map(|hunk| hunk.lines.iter())
            .flat_map(|line| {
                token_buffer
                    .view(line.syntax_tokens)
                    .iter()
                    .map(|span| span.kind)
            })
            .collect::<Vec<_>>();
        assert!(token_kinds.is_empty());
    }

    #[test]
    fn annotator_degrades_unsupported_languages_to_plain_text() {
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();
        let mut document = test_document(
            "test.unknown",
            &[
                ("old plain text", LineKind::Removed, Some(1), None),
                ("new plain text", LineKind::Added, None, Some(1)),
            ],
            &mut text_buffer,
        );

        let annotator = DiffSyntaxAnnotator::new();
        annotator.annotate(&mut document.files[0], &mut text_buffer, &mut token_buffer);

        let token_kinds = document.files[0]
            .hunks
            .iter()
            .flat_map(|hunk| hunk.lines.iter())
            .flat_map(|line| {
                token_buffer
                    .view(line.syntax_tokens)
                    .iter()
                    .map(|span| span.kind)
            })
            .collect::<Vec<_>>();
        assert!(token_kinds.is_empty());
    }
}
