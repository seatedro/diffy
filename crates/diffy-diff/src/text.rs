//! Line-indexed text of one side of a file.

use std::ops::Range;
use std::sync::Arc;

/// Text split into lines without copying. A final line without a newline
/// still counts as a line; a trailing newline does not start an empty one.
/// Line content excludes the `\n` but keeps a `\r` before it, so applying a
/// diff reproduces CRLF text byte for byte; [`TextStore::display_line`]
/// drops the `\r`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextStore {
    text: Arc<str>,
    line_starts: Arc<[u32]>,
}

impl Default for TextStore {
    fn default() -> Self {
        Self::new("")
    }
}

impl TextStore {
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        let text: Arc<str> = text.into();
        let mut starts = Vec::with_capacity(text.len() / 32 + 1);
        if !text.is_empty() {
            starts.push(0);
        }
        // `find` scans with core's word-at-a-time memchr, several times
        // faster than a byte loop over a large text.
        let mut at = 0;
        while let Some(i) = text[at..].find('\n') {
            at += i + 1;
            if at < text.len() {
                starts.push(at as u32);
            }
        }
        Self {
            text,
            line_starts: starts.into(),
        }
    }

    /// The whole text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The whole text, shared.
    pub fn shared(&self) -> &Arc<str> {
        &self.text
    }

    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// Whether the text is non-empty and its last line has no newline.
    pub fn no_newline_at_eof(&self) -> bool {
        !self.text.is_empty() && !self.text.ends_with('\n')
    }

    /// Byte range of line `index` without its newline.
    pub fn line_range(&self, index: u32) -> Option<Range<usize>> {
        let i = index as usize;
        let start = *self.line_starts.get(i)? as usize;
        let next = self
            .line_starts
            .get(i + 1)
            .map_or(self.text.len(), |&s| s as usize);
        let end = if self.text.as_bytes().get(next.wrapping_sub(1)) == Some(&b'\n') {
            next - 1
        } else {
            next
        };
        Some(start..end.max(start))
    }

    /// Line `index` without its newline, `\r` kept.
    pub fn line(&self, index: u32) -> Option<&str> {
        self.text.get(self.line_range(index)?)
    }

    /// Line `index` as displayed and copied: without a trailing `\r`.
    pub fn display_line(&self, index: u32) -> Option<&str> {
        self.line(index).map(|l| l.strip_suffix('\r').unwrap_or(l))
    }

    /// Byte range of the display text of line `index`.
    pub fn display_range(&self, index: u32) -> Option<Range<usize>> {
        let range = self.line_range(index)?;
        let cr = self.text.as_bytes().get(range.end.wrapping_sub(1)) == Some(&b'\r')
            && range.end > range.start;
        Some(range.start..range.end - usize::from(cr))
    }
}

#[cfg(test)]
mod tests {
    use super::TextStore;

    /// Each line of `text` as `index:content`, for table tests.
    fn dump(text: &str) -> String {
        let store = TextStore::new(text);
        (0..store.line_count())
            .map(|i| format!("{i}:{}", store.line(i).unwrap().escape_debug()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn lines_split_without_a_phantom_final_line() {
        let cases = [
            ("", ""),
            ("a", "0:a"),
            ("a\n", "0:a"),
            ("a\n\n", "0:a 1:"),
            ("a\r\nb", "0:a\\r 1:b"),
        ];
        for (text, expected) in cases {
            assert_eq!(dump(text), expected, "{text:?}");
        }
    }

    #[test]
    fn display_lines_drop_the_carriage_return() {
        let store = TextStore::new("a\r\n\r\nb");
        let lines: Vec<_> = (0..3).map(|i| store.display_line(i).unwrap()).collect();
        assert_eq!(lines, ["a", "", "b"]);
        assert_eq!(store.display_range(1), Some(3..3));
    }
}
