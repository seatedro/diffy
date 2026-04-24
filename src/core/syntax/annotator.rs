use std::collections::HashSet;
use std::ops::Range;

use crate::core::diff::{FileDiff, Hunk, LineKind};
use crate::core::rendering::{DiffRowType, flatten_file_diff};
use crate::core::syntax::Highlighter;
use crate::core::text::{DiffTokenSpan, TextBuffer, TokenBuffer};

#[derive(Debug, Clone, Copy)]
struct LineRef {
    hunk_index: usize,
    line_index: usize,
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

    pub fn annotate_full_file_window(
        &self,
        file_diff: &FileDiff,
        file_index: usize,
        old_lines: Option<&[String]>,
        new_lines: Option<&[String]>,
        window: SyntaxRowWindow,
    ) -> Vec<SyntaxLineTokens> {
        if file_diff.is_binary || window.end <= window.start {
            return Vec::new();
        }

        let (old_refs, new_refs) = build_full_file_refs(file_diff, file_index, window);
        let language = self.highlighter.resolve_language(&file_diff.path);
        let mut out = Vec::new();

        if let Some(lines) = old_lines {
            let (source, line_offsets) = source_from_lines(lines);
            let byte_refs = byte_refs_for_refs(&old_refs, &line_offsets, lines);
            let ranges = byte_ranges_for_refs(&byte_refs);
            let tokens = match self
                .highlighter
                .highlight_resolved_ranges(language, &source, &ranges)
            {
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
            out.extend(collect_line_tokens(&tokens, &byte_refs));
        }

        if let Some(lines) = new_lines {
            let (source, line_offsets) = source_from_lines(lines);
            let byte_refs = byte_refs_for_refs(&new_refs, &line_offsets, lines);
            let ranges = byte_ranges_for_refs(&byte_refs);
            let tokens = match self
                .highlighter
                .highlight_resolved_ranges(language, &source, &ranges)
            {
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
            out.extend(collect_line_tokens(&tokens, &byte_refs));
        }

        out
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

    pub fn annotate_full_file_window_from_cache(
        &self,
        file_diff: &FileDiff,
        file_index: usize,
        old_syntax: Option<&FullFileSyntax>,
        new_syntax: Option<&FullFileSyntax>,
        window: SyntaxRowWindow,
    ) -> Vec<SyntaxLineTokens> {
        if file_diff.is_binary || window.end <= window.start {
            return Vec::new();
        }

        let (old_refs, new_refs) = build_full_file_refs(file_diff, file_index, window);
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
        while token_index < tokens.len()
            && (tokens[token_index].offset as usize + tokens[token_index].length as usize)
                <= line_start
        {
            token_index += 1;
        }

        let mut line_tokens = Vec::new();
        for token in tokens.iter().skip(token_index) {
            let start = token.offset as usize;
            if start >= line_end {
                break;
            }
            let end = start + token.length as usize;
            let clipped_start = start.max(line_start);
            let clipped_end = end.min(line_end);
            if clipped_end <= clipped_start {
                continue;
            }
            line_tokens.push(DiffTokenSpan {
                offset: (clipped_start - line_start) as u32,
                length: (clipped_end - clipped_start) as u32,
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

fn build_full_file_refs(
    file_diff: &FileDiff,
    file_index: usize,
    window: SyntaxRowWindow,
) -> (Vec<LineRef>, Vec<LineRef>) {
    let rows = flatten_file_diff(file_diff, file_index);
    let end = window.end.min(rows.len());
    let mut old_refs = Vec::new();
    let mut new_refs = Vec::new();
    let mut old_seen = HashSet::new();
    let mut new_seen = HashSet::new();

    for row in rows
        .iter()
        .skip(window.start)
        .take(end.saturating_sub(window.start))
    {
        match row.row_type {
            DiffRowType::Context => {
                push_full_file_ref(
                    file_diff,
                    row.hunk_index,
                    row.line_index,
                    true,
                    &mut old_seen,
                    &mut old_refs,
                );
                push_full_file_ref(
                    file_diff,
                    row.hunk_index,
                    row.line_index,
                    false,
                    &mut new_seen,
                    &mut new_refs,
                );
            }
            DiffRowType::Removed => {
                push_full_file_ref(
                    file_diff,
                    row.hunk_index,
                    row.old_line_index,
                    true,
                    &mut old_seen,
                    &mut old_refs,
                );
            }
            DiffRowType::Added => {
                push_full_file_ref(
                    file_diff,
                    row.hunk_index,
                    row.new_line_index,
                    false,
                    &mut new_seen,
                    &mut new_refs,
                );
            }
            DiffRowType::Modified => {
                push_full_file_ref(
                    file_diff,
                    row.hunk_index,
                    row.old_line_index,
                    true,
                    &mut old_seen,
                    &mut old_refs,
                );
                push_full_file_ref(
                    file_diff,
                    row.hunk_index,
                    row.new_line_index,
                    false,
                    &mut new_seen,
                    &mut new_refs,
                );
            }
            DiffRowType::FileHeader | DiffRowType::HunkSeparator => {}
        }
    }

    (old_refs, new_refs)
}

fn push_full_file_ref(
    file_diff: &FileDiff,
    hunk_index: i32,
    line_index: i32,
    old_side: bool,
    seen: &mut HashSet<(usize, usize)>,
    refs: &mut Vec<LineRef>,
) {
    if hunk_index < 0 || line_index < 0 {
        return;
    }
    let hunk_index = hunk_index as usize;
    let line_index = line_index as usize;
    if !seen.insert((hunk_index, line_index)) {
        return;
    }
    let Some(line) = file_diff
        .hunks
        .get(hunk_index)
        .and_then(|hunk| hunk.lines.get(line_index))
    else {
        return;
    };
    let Some(line_number) = (if old_side {
        line.old_line_number
    } else {
        line.new_line_number
    }) else {
        return;
    };
    if line_number <= 0 {
        return;
    }
    refs.push(LineRef {
        hunk_index,
        line_index,
        content_offset: (line_number - 1) as usize,
        content_len: 0,
    });
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

fn byte_refs_for_refs(refs: &[LineRef], line_offsets: &[usize], lines: &[String]) -> Vec<LineRef> {
    refs.iter()
        .filter_map(|reference| {
            let line_index = reference.content_offset;
            let start = *line_offsets.get(line_index)?;
            let len = lines.get(line_index)?.len();
            Some(LineRef {
                hunk_index: reference.hunk_index,
                line_index: reference.line_index,
                content_offset: start,
                content_len: len,
            })
        })
        .collect()
}

fn byte_ranges_for_refs(refs: &[LineRef]) -> Vec<Range<usize>> {
    let mut ranges = refs
        .iter()
        .filter_map(|reference| {
            (reference.content_len > 0).then_some(
                reference.content_offset..reference.content_offset + reference.content_len,
            )
        })
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start);
    ranges
}

fn byte_refs_for_cached_refs(refs: &[LineRef], syntax: &FullFileSyntax) -> Vec<LineRef> {
    refs.iter()
        .filter_map(|reference| {
            let line_index = reference.content_offset;
            Some(LineRef {
                hunk_index: reference.hunk_index,
                line_index: reference.line_index,
                content_offset: *syntax.line_offsets.get(line_index)?,
                content_len: *syntax.line_lengths.get(line_index)?,
            })
        })
        .collect()
}

fn collect_line_tokens(tokens: &[DiffTokenSpan], refs: &[LineRef]) -> Vec<SyntaxLineTokens> {
    let mut token_index = 0usize;
    let mut out = Vec::new();
    for reference in refs {
        let line_start = reference.content_offset;
        let line_end = line_start + reference.content_len;
        while token_index < tokens.len()
            && (tokens[token_index].offset as usize + tokens[token_index].length as usize)
                <= line_start
        {
            token_index += 1;
        }

        let mut line_tokens = Vec::new();
        for token in tokens.iter().skip(token_index) {
            let start = token.offset as usize;
            if start >= line_end {
                break;
            }
            let end = start + token.length as usize;
            let clipped_start = start.max(line_start);
            let clipped_end = end.min(line_end);
            if clipped_end <= clipped_start {
                continue;
            }
            line_tokens.push(DiffTokenSpan {
                offset: (clipped_start - line_start) as u32,
                length: (clipped_end - clipped_start) as u32,
                kind: token.kind,
                ..DiffTokenSpan::default()
            });
        }

        out.push(SyntaxLineTokens {
            hunk_index: reference.hunk_index,
            line_index: reference.line_index,
            tokens: line_tokens,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::core::diff::unified_parser::parse_into;
    use crate::core::syntax::DiffSyntaxAnnotator;
    use crate::core::text::{TextBuffer, TokenBuffer};

    #[test]
    fn annotator_degrades_missing_packs_to_plain_text() {
        let highlighter = phosphor::Highlighter::new();
        if highlighter.is_parser_available(phosphor::LanguageId::Json) {
            return;
        }

        let patch = concat!(
            "diff --git a/test.json b/test.json\n",
            "--- a/test.json\n",
            "+++ b/test.json\n",
            "@@ -1,2 +1,3 @@\n",
            " {\n",
            "-  \"name\": \"old\"\n",
            "+  \"name\": \"new\",\n",
            "+  \"fast\": true\n",
        );
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();
        let mut document = parse_into(patch, &mut text_buffer);

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
        let patch = concat!(
            "diff --git a/test.unknown b/test.unknown\n",
            "--- a/test.unknown\n",
            "+++ b/test.unknown\n",
            "@@ -1 +1 @@\n",
            "-old plain text\n",
            "+new plain text\n",
        );
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();
        let mut document = parse_into(patch, &mut text_buffer);

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
