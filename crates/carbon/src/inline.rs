#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ChangeIntensity {
    #[default]
    Novel,
    NovelWord,
    UnchangedContext,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct InlineSpan {
    pub offset: u32,
    pub len: u32,
    pub intensity: ChangeIntensity,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum InlineDiffMode {
    #[default]
    Word,
    WordAlt,
    Char,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InlineOptions {
    pub mode: InlineDiffMode,
    pub max_line_len: usize,
}

impl Default for InlineOptions {
    fn default() -> Self {
        Self {
            mode: InlineDiffMode::Word,
            max_line_len: 1_000,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineDiff {
    pub old: Vec<InlineSpan>,
    pub new: Vec<InlineSpan>,
}

pub fn compute_inline_diff(old: &str, new: &str, options: InlineOptions) -> InlineDiff {
    if options.mode == InlineDiffMode::None
        || old.len() > options.max_line_len
        || new.len() > options.max_line_len
    {
        return InlineDiff::default();
    }

    let old_tokens = match options.mode {
        InlineDiffMode::Char => tokenize_chars(old),
        InlineDiffMode::Word | InlineDiffMode::WordAlt | InlineDiffMode::None => {
            tokenize_words(old)
        }
    };
    let new_tokens = match options.mode {
        InlineDiffMode::Char => tokenize_chars(new),
        InlineDiffMode::Word | InlineDiffMode::WordAlt | InlineDiffMode::None => {
            tokenize_words(new)
        }
    };
    let (mut old_spans, mut new_spans) = lcs_diff(&old_tokens, &new_tokens);
    if options.mode == InlineDiffMode::WordAlt {
        join_close_spans(&mut old_spans);
        join_close_spans(&mut new_spans);
    }
    InlineDiff {
        old: old_spans,
        new: new_spans,
    }
}

#[derive(Debug, Clone, Copy)]
struct Token<'a> {
    offset: usize,
    len: usize,
    text: &'a str,
}

impl Token<'_> {
    fn span(self) -> InlineSpan {
        InlineSpan {
            offset: self.offset.min(u32::MAX as usize) as u32,
            len: self.len.min(u32::MAX as usize) as u32,
            intensity: ChangeIntensity::NovelWord,
        }
    }
}

fn tokenize_words(text: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        let kind = WordTokenKind::for_char(ch);
        let mut end = start + ch.len_utf8();
        while let Some(&(next_start, next_ch)) = chars.peek() {
            if kind == WordTokenKind::Other || WordTokenKind::for_char(next_ch) != kind {
                break;
            }
            chars.next();
            end = next_start + next_ch.len_utf8();
        }
        tokens.push(Token {
            offset: start,
            len: end - start,
            text: &text[start..end],
        });
    }
    tokens
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordTokenKind {
    Word,
    Whitespace,
    Other,
}

impl WordTokenKind {
    fn for_char(ch: char) -> Self {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            Self::Word
        } else if ch.is_whitespace() {
            Self::Whitespace
        } else {
            Self::Other
        }
    }
}

fn tokenize_chars(text: &str) -> Vec<Token<'_>> {
    text.char_indices()
        .map(|(offset, ch)| Token {
            offset,
            len: ch.len_utf8(),
            text: &text[offset..offset + ch.len_utf8()],
        })
        .collect()
}

fn lcs_diff(old: &[Token<'_>], new: &[Token<'_>]) -> (Vec<InlineSpan>, Vec<InlineSpan>) {
    let mut lcs = vec![vec![0_usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lcs[i][j] = if old[i].text == new[j].text {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut removed = Vec::new();
    let mut added = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < old.len() && j < new.len() {
        if old[i].text == new[j].text {
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            removed.push(old[i].span());
            i += 1;
        } else {
            added.push(new[j].span());
            j += 1;
        }
    }
    removed.extend(old[i..].iter().map(|token| token.span()));
    added.extend(new[j..].iter().map(|token| token.span()));
    (removed, added)
}

fn join_close_spans(spans: &mut Vec<InlineSpan>) {
    if spans.len() < 2 {
        return;
    }
    spans.sort_by_key(|span| span.offset);
    let mut joined = Vec::<InlineSpan>::with_capacity(spans.len());
    for span in spans.drain(..) {
        if let Some(last) = joined.last_mut() {
            let last_end = last.offset.saturating_add(last.len);
            if span.offset <= last_end.saturating_add(1) {
                let end = span.offset.saturating_add(span.len);
                last.len = end.saturating_sub(last.offset);
                continue;
            }
        }
        joined.push(span);
    }
    *spans = joined;
}

#[cfg(test)]
mod tests {
    use super::{InlineDiffMode, InlineOptions, compute_inline_diff};

    #[test]
    fn word_diff_marks_changed_word_on_both_sides() {
        let diff = compute_inline_diff("let old = 1;", "let new = 1;", InlineOptions::default());
        assert_eq!((diff.old[0].offset, diff.old[0].len), (4, 3));
        assert_eq!((diff.new[0].offset, diff.new[0].len), (4, 3));
    }

    #[test]
    fn char_diff_can_mark_single_character() {
        let diff = compute_inline_diff(
            "abc",
            "axc",
            InlineOptions {
                mode: InlineDiffMode::Char,
                ..InlineOptions::default()
            },
        );
        assert_eq!((diff.old[0].offset, diff.old[0].len), (1, 1));
        assert_eq!((diff.new[0].offset, diff.new[0].len), (1, 1));
    }

    #[test]
    fn long_line_guard_disables_inline_diff() {
        let diff = compute_inline_diff(
            "abcdef",
            "abqdef",
            InlineOptions {
                max_line_len: 3,
                ..InlineOptions::default()
            },
        );
        assert!(diff.old.is_empty());
        assert!(diff.new.is_empty());
    }
}
