use crate::core::diff::{FileDiff, Hunk, LineKind};
use crate::core::syntax::{Highlighter, LanguageRegistry};
use crate::core::text::{DiffTokenSpan, TextBuffer, TokenBuffer};

#[derive(Debug, Clone, Copy)]
struct LineRef {
    hunk_index: usize,
    line_index: usize,
    content_offset: usize,
    content_len: usize,
}

#[derive(Debug)]
pub struct DiffSyntaxAnnotator {
    registry: LanguageRegistry,
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
            registry: LanguageRegistry::new(),
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
        let extension = file_diff
            .path
            .rsplit_once('.')
            .map(|(_, ext)| format!(".{ext}"))
            .unwrap_or_default();
        let Some(grammar) = self.registry.grammar_for_extension(&extension) else {
            return;
        };

        let (old_content, new_content, old_refs, new_refs) =
            build_line_refs(&file_diff.hunks, text_buffer);

        let old_tokens = self
            .highlighter
            .highlight(grammar, &old_content)
            .unwrap_or_default();
        let new_tokens = self
            .highlighter
            .highlight(grammar, &new_content)
            .unwrap_or_default();
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
            });
        }

        if !line_tokens.is_empty() {
            let range = token_buffer.append(&line_tokens);
            hunks[reference.hunk_index].lines[reference.line_index].syntax_tokens = range;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::core::diff::unified_parser::parse_into;
    use crate::core::syntax::{DiffSyntaxAnnotator, SyntaxTokenKind};
    use crate::core::text::{TextBuffer, TokenBuffer};

    #[test]
    fn unified_diff_to_syntax_tokens_pipeline() {
        let patch = concat!(
            "diff --git a/test.py b/test.py\n",
            "--- a/test.py\n",
            "+++ b/test.py\n",
            "@@ -1,2 +1,3 @@\n",
            " def greet(name):\n",
            "-    return \"hi\"\n",
            "+    value = \"hello\"\n",
            "+    return value\n",
        );
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();
        let mut document = parse_into(patch, &mut text_buffer);

        let annotator = DiffSyntaxAnnotator::new();
        annotator.annotate(&mut document.files[0], &mut text_buffer, &mut token_buffer);

        let file = &document.files[0];
        let token_kinds = file
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
        assert!(
            token_kinds.contains(&SyntaxTokenKind::Function)
                || token_kinds.contains(&SyntaxTokenKind::Keyword)
        );
        assert!(token_kinds.contains(&SyntaxTokenKind::String));
    }
}
