use crate::core::syntax::Highlighter;
use crate::core::text::DiffTokenSpan;
use carbon::{LineId, TextByteRange, TextStore};

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

/// Half-open window of 0-based source lines on one diff side.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceLineWindow {
    pub start: usize,
    pub end: usize,
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

    pub fn estimated_bytes(&self) -> usize {
        self.line_offsets
            .len()
            .saturating_mul(std::mem::size_of::<usize>())
            .saturating_add(
                self.line_lengths
                    .len()
                    .saturating_mul(std::mem::size_of::<usize>()),
            )
            .saturating_add(
                self.tokens
                    .len()
                    .saturating_mul(std::mem::size_of::<DiffTokenSpan>()),
            )
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

    pub fn highlight_full_lines(&self, path: &str, lines: &[String]) -> FullFileSyntax {
        let text = text_store_from_lines(lines);
        self.highlight_full_text_store(path, &text)
    }

    pub fn highlight_full_text_store(&self, path: &str, text: &TextStore) -> FullFileSyntax {
        let (line_offsets, line_lengths) = line_ranges_from_text_store(text);
        let language = self.highlighter.resolve_language(path);
        let tokens = match self
            .highlighter
            .highlight_text_store_resolved(language, text)
        {
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

    /// Highlights only the given source-line window of `text`. Tree-sitter
    /// still parses the full source (windowed parsing would lose syntactic
    /// context), but query/span extraction — the expensive part on very large
    /// files — is restricted to the window's byte range. The returned value
    /// keeps full-file line tables so byte mapping works for any line, while
    /// tokens only cover the window.
    pub fn highlight_window_text_store(
        &self,
        path: &str,
        text: &TextStore,
        lines: SourceLineWindow,
    ) -> FullFileSyntax {
        let (line_offsets, line_lengths) = line_ranges_from_text_store(text);
        let line_count = line_lengths.len();
        let start_line = lines.start.min(line_count);
        let end_line = lines.end.clamp(start_line, line_count);
        let text_len = carbon::u32_to_usize_saturating(text.len());
        let start_byte = line_offsets.get(start_line).copied().unwrap_or(text_len);
        // `line_offsets` carries a trailing entry at `text.len()`, so the
        // exclusive end line maps to the byte just past the window.
        let end_byte = line_offsets.get(end_line).copied().unwrap_or(text_len);

        let mut tokens = Vec::new();
        if end_byte > start_byte {
            let language = self.highlighter.resolve_language(path);
            let range = TextByteRange {
                start: usize_to_u32_saturating(start_byte),
                len: usize_to_u32_saturating(end_byte - start_byte),
            };
            match self
                .highlighter
                .highlight_text_store_resolved_ranges(language, text, &[range])
            {
                Ok(spans) => tokens = spans,
                Err(error) => {
                    tracing::warn!(
                        path = %path,
                        ?language,
                        %error,
                        "windowed syntax highlight failed"
                    );
                }
            }
        }

        FullFileSyntax {
            line_offsets,
            line_lengths,
            tokens,
        }
    }

    pub fn annotate_carbon_full_file_window_from_cache(
        &self,
        file: &carbon::FileDiff,
        expansion: &carbon::ExpansionState,
        file_index: usize,
        old_syntax: Option<&FullFileSyntax>,
        new_syntax: Option<&FullFileSyntax>,
        window: SyntaxRowWindow,
    ) -> Vec<SyntaxLineTokens> {
        if file.is_binary || window.end <= window.start {
            return Vec::new();
        }

        let (old_refs, new_refs) = build_carbon_full_file_refs(file, expansion, file_index, window);
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

/// Returns the (old, new) source-line bounds touched by a projected-row
/// window, so callers can request windowed highlighting per side. `None`
/// means the side has no content rows inside the window.
pub fn carbon_window_source_line_bounds(
    file: &carbon::FileDiff,
    expansion: &carbon::ExpansionState,
    window: SyntaxRowWindow,
) -> (Option<SourceLineWindow>, Option<SourceLineWindow>) {
    if file.is_binary || window.end <= window.start {
        return (None, None);
    }
    let (old_refs, new_refs) = build_carbon_full_file_refs(file, expansion, 0, window);
    (source_line_bounds(&old_refs), source_line_bounds(&new_refs))
}

fn source_line_bounds(refs: &[LineRef]) -> Option<SourceLineWindow> {
    // At this stage `content_offset` holds the 0-based source line index.
    let min = refs.iter().map(|r| r.content_offset).min()?;
    let max = refs.iter().map(|r| r.content_offset).max()?;
    Some(SourceLineWindow {
        start: min,
        end: max.saturating_add(1),
    })
}

fn build_carbon_full_file_refs(
    file: &carbon::FileDiff,
    expansion: &carbon::ExpansionState,
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
        expansion,
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

fn text_store_from_lines(lines: &[String]) -> TextStore {
    let size = lines.iter().map(|line| line.len() + 1).sum();
    let mut source = String::with_capacity(size);
    for line in lines {
        source.push_str(line);
        source.push('\n');
    }
    TextStore::from_text(source)
}

fn line_ranges_from_text_store(text: &TextStore) -> (Vec<usize>, Vec<usize>) {
    let line_count = carbon::u32_to_usize_saturating(text.line_count());
    let mut offsets = Vec::with_capacity(line_count + 1);
    let mut lengths = Vec::with_capacity(line_count);
    for index in 0..line_count {
        if let Some(range) = text.line_range(LineId(carbon::usize_to_u32_saturating(index))) {
            offsets.push(carbon::u32_to_usize_saturating(range.start));
            lengths.push(carbon::u32_to_usize_saturating(range.len));
        }
    }
    offsets.push(carbon::u32_to_usize_saturating(text.len()));
    (offsets, lengths)
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
