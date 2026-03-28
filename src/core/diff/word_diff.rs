use crate::core::text::token::{DiffTokenSpan, SyntaxTokenKind};

pub fn compute_word_diff(
    old_text: &str,
    new_text: &str,
) -> (Vec<DiffTokenSpan>, Vec<DiffTokenSpan>) {
    let old_tokens = tokenize(old_text);
    let new_tokens = tokenize(new_text);
    let old_len = old_tokens.len();
    let new_len = new_tokens.len();

    let mut lcs = vec![vec![0_usize; new_len + 1]; old_len + 1];
    for i in (0..old_len).rev() {
        for j in (0..new_len).rev() {
            lcs[i][j] = if old_tokens[i].text == new_tokens[j].text {
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

    while i < old_len && j < new_len {
        if old_tokens[i].text == new_tokens[j].text {
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            removed.push(old_tokens[i].span());
            i += 1;
        } else {
            added.push(new_tokens[j].span());
            j += 1;
        }
    }

    while i < old_len {
        removed.push(old_tokens[i].span());
        i += 1;
    }
    while j < new_len {
        added.push(new_tokens[j].span());
        j += 1;
    }

    (removed, added)
}

#[derive(Debug, Clone, Copy)]
struct Word<'a> {
    offset: usize,
    len: usize,
    text: &'a str,
}

impl Word<'_> {
    fn span(self) -> DiffTokenSpan {
        DiffTokenSpan {
            offset: self.offset as u32,
            length: self.len as u32,
            kind: SyntaxTokenKind::Normal,
        }
    }
}

fn tokenize(text: &str) -> Vec<Word<'_>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        let start = index;
        let byte = bytes[index];

        if is_word(byte) {
            while index < bytes.len() && is_word(bytes[index]) {
                index += 1;
            }
        } else if byte.is_ascii_whitespace() {
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
        } else {
            index += 1;
        }

        tokens.push(Word {
            offset: start,
            len: index - start,
            text: &text[start..index],
        });
    }

    tokens
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::compute_word_diff;

    #[test]
    fn identical_lines_have_no_changes() {
        let (removed, added) = compute_word_diff("hello world", "hello world");
        assert!(removed.is_empty());
        assert!(added.is_empty());
    }

    #[test]
    fn single_word_change_marks_both_sides() {
        let (removed, added) = compute_word_diff("int foo = 1;", "int bar = 1;");
        assert_eq!(removed.len(), 1);
        assert_eq!(added.len(), 1);
        assert_eq!((removed[0].offset, removed[0].length), (4, 3));
        assert_eq!((added[0].offset, added[0].length), (4, 3));
    }

    #[test]
    fn empty_to_content_marks_additions() {
        let (removed, added) = compute_word_diff("", "hello world");
        assert!(removed.is_empty());
        assert!(!added.is_empty());
    }
}
