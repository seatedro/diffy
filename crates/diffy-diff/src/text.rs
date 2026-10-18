//! Line-indexed text of one side of a file.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

const ESCAPE: &str = "\u{FFFD}";
const ESCAPE_LEN: usize = ESCAPE.len();

/// Text split into lines without copying. A final line without a newline
/// still counts as a line; a trailing newline does not start an empty one.
/// Line content excludes the `\n` but keeps a `\r` before it, so applying a
/// diff reproduces CRLF text byte for byte; [`TextStore::display_line`]
/// drops the `\r`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextStore {
    text: Arc<str>,
    line_starts: Arc<[u32]>,
    escapes: Option<Arc<ByteEscapes>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteEscapes {
    at: Box<[u32]>,
    bytes: Box<[u8]>,
}

impl ByteEscapes {
    pub fn len(&self) -> usize {
        self.at.len()
    }

    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, u8)> + '_ {
        self.at
            .iter()
            .zip(self.bytes.iter())
            .map(|(&at, &byte)| (at as usize, byte))
    }
}

impl Default for TextStore {
    fn default() -> Self {
        Self::new("")
    }
}

impl TextStore {
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Self::with_escapes(text.into(), None)
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        if let Ok(text) = std::str::from_utf8(bytes) {
            return Self::new(text);
        }
        let mut text = String::with_capacity(bytes.len() + bytes.len() / 8);
        let mut at = Vec::new();
        let mut escaped = Vec::new();
        for chunk in bytes.utf8_chunks() {
            text.push_str(chunk.valid());
            for &byte in chunk.invalid() {
                at.push(text.len() as u32);
                escaped.push(byte);
                text.push_str(ESCAPE);
            }
        }
        let escapes = ByteEscapes {
            at: at.into(),
            bytes: escaped.into(),
        };
        Self::with_escapes(text.into(), Some(Arc::new(escapes)))
    }

    fn with_escapes(text: Arc<str>, escapes: Option<Arc<ByteEscapes>>) -> Self {
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
            escapes,
        }
    }

    pub fn escapes(&self) -> Option<&ByteEscapes> {
        self.escapes.as_deref()
    }

    pub fn is_exact(&self) -> bool {
        self.escapes.is_none()
    }

    pub fn source_len(&self) -> usize {
        self.source_offset(self.text.len())
    }

    pub fn source_offset(&self, display: usize) -> usize {
        let Some(escapes) = self.escapes.as_deref() else {
            return display;
        };
        let before = escapes
            .at
            .partition_point(|&at| at as usize + ESCAPE_LEN <= display);
        match escapes.at.get(before) {
            Some(&at) if (at as usize) < display => at as usize - before * (ESCAPE_LEN - 1),
            _ => display - before * (ESCAPE_LEN - 1),
        }
    }

    pub fn display_offset(&self, source: usize) -> usize {
        let Some(escapes) = self.escapes.as_deref() else {
            return source;
        };
        let mut k = 0;
        let mut hi = escapes.at.len();
        while k < hi {
            let mid = (k + hi) / 2;
            if (escapes.at[mid] as usize) - mid * (ESCAPE_LEN - 1) < source {
                k = mid + 1;
            } else {
                hi = mid;
            }
        }
        source + k * (ESCAPE_LEN - 1)
    }

    pub fn source_bytes(&self, range: Range<usize>) -> Cow<'_, [u8]> {
        let end = range.end.min(self.text.len());
        let start = range.start.min(end);
        let bytes = &self.text.as_bytes()[start..end];
        let Some(escapes) = self.escapes.as_deref() else {
            return Cow::Borrowed(bytes);
        };
        let first = escapes.at.partition_point(|&at| (at as usize) < start);
        let last = escapes.at.partition_point(|&at| (at as usize) < end);
        if first == last {
            return Cow::Borrowed(bytes);
        }
        let mut out = Vec::with_capacity(bytes.len());
        let mut cursor = start;
        for i in first..last {
            let at = escapes.at[i] as usize;
            out.extend_from_slice(&self.text.as_bytes()[cursor..at]);
            out.push(escapes.bytes[i]);
            cursor = (at + ESCAPE_LEN).min(end);
        }
        out.extend_from_slice(&self.text.as_bytes()[cursor..end]);
        Cow::Owned(out)
    }

    pub fn to_source_bytes(&self) -> Cow<'_, [u8]> {
        self.source_bytes(0..self.text.len())
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
    fn invalid_bytes_are_escaped_and_map_back_exactly() {
        let source = b"ok\xff\n\xfe\xfdz\r\nend";
        let store = TextStore::from_bytes(source);
        assert_eq!(store.as_str(), "ok\u{FFFD}\n\u{FFFD}\u{FFFD}z\r\nend");
        assert!(!store.is_exact());
        assert_eq!(store.line_count(), 3);
        assert_eq!(store.display_line(1), Some("\u{FFFD}\u{FFFD}z"));
        assert_eq!(store.to_source_bytes().as_ref(), source);
        assert_eq!(store.source_len(), source.len());
        let line = store.line_range(1).unwrap();
        assert_eq!(store.source_bytes(line).as_ref(), b"\xfe\xfdz\r");
        for (display, source) in [(0, 0), (2, 2), (3, 2), (5, 3), (6, 4), (12, 6), (13, 7)] {
            assert_eq!(store.source_offset(display), source, "display {display}");
        }
        for source_at in 0..=source.len() {
            let display = store.display_offset(source_at);
            assert_eq!(
                store.source_offset(display),
                source_at,
                "source {source_at}"
            );
        }
        assert!(TextStore::from_bytes(b"plain\n").is_exact());
    }

    #[test]
    fn display_lines_drop_the_carriage_return() {
        let store = TextStore::new("a\r\n\r\nb");
        let lines: Vec<_> = (0..3).map(|i| store.display_line(i).unwrap()).collect();
        assert_eq!(lines, ["a", "", "b"]);
        assert_eq!(store.display_range(1), Some(3..3));
    }
}
