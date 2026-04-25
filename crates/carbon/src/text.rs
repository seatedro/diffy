use std::sync::Arc;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct LineId(pub u32);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TextByteRange {
    pub start: u32,
    pub len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextStore {
    bytes: Arc<[u8]>,
    line_starts: Arc<[u32]>,
    trailing_newline: bool,
}

impl Default for TextStore {
    fn default() -> Self {
        Self::from_bytes(Vec::new())
    }
}

impl TextStore {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self::from_bytes(text.into().into_bytes())
    }

    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        let trailing_newline = bytes.last().is_some_and(|b| *b == b'\n');
        let line_starts = Arc::from(index_line_starts_scalar(&bytes));
        Self {
            bytes: Arc::from(bytes),
            line_starts,
            trailing_newline,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes).ok()
    }

    pub fn len(&self) -> u32 {
        self.bytes.len().min(u32::MAX as usize) as u32
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn line_count(&self) -> u32 {
        if self.bytes.is_empty() {
            0
        } else {
            self.line_starts.len().min(u32::MAX as usize) as u32
        }
    }

    pub fn has_trailing_newline(&self) -> bool {
        self.trailing_newline
    }

    pub fn no_newline_at_eof(&self) -> bool {
        !self.bytes.is_empty() && !self.trailing_newline
    }

    pub fn line_start(&self, line: LineId) -> Option<u32> {
        self.line_starts.get(line.0 as usize).copied()
    }

    pub fn line_range(&self, line: LineId) -> Option<TextByteRange> {
        if line.0 >= self.line_count() {
            return None;
        }
        let start = self.line_start(line)? as usize;
        let next_start = self
            .line_starts
            .get(line.0 as usize + 1)
            .copied()
            .map(|n| n as usize)
            .unwrap_or(self.bytes.len());
        let mut end = next_start;
        if end > start && self.bytes.get(end - 1) == Some(&b'\n') {
            end -= 1;
        }
        if end > start && self.bytes.get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
        Some(TextByteRange {
            start: start.min(u32::MAX as usize) as u32,
            len: end.saturating_sub(start).min(u32::MAX as usize) as u32,
        })
    }

    pub fn line_bytes(&self, line: LineId) -> Option<&[u8]> {
        let range = self.line_range(line)?;
        self.bytes
            .get(range.start as usize..range.start.saturating_add(range.len) as usize)
    }

    pub fn line_str(&self, line: LineId) -> Option<&str> {
        std::str::from_utf8(self.line_bytes(line)?).ok()
    }

    pub fn byte_range_for_lines(&self, start: LineId, count: u32) -> Option<TextByteRange> {
        if count == 0 {
            return Some(TextByteRange {
                start: self.line_start(start).unwrap_or(self.len()),
                len: 0,
            });
        }
        let first = self.line_range(start)?;
        let last = self.line_range(LineId(start.0.checked_add(count - 1)?))?;
        let end = last.start.saturating_add(last.len);
        Some(TextByteRange {
            start: first.start,
            len: end.saturating_sub(first.start),
        })
    }
}

pub fn index_line_starts_scalar(bytes: &[u8]) -> Vec<u32> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut starts = Vec::with_capacity(bytes.len().saturating_div(32).max(1));
    starts.push(0);
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' && idx + 1 < bytes.len() {
            starts.push((idx + 1).min(u32::MAX as usize) as u32);
        }
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::{LineId, TextStore, index_line_starts_scalar};

    #[test]
    fn indexes_lf_and_final_newline_without_phantom_line() {
        let text = TextStore::from_text("a\nb\n");
        assert_eq!(index_line_starts_scalar(b"a\nb\n"), vec![0, 2]);
        assert_eq!(text.line_count(), 2);
        assert!(text.has_trailing_newline());
        assert!(!text.no_newline_at_eof());
        assert_eq!(text.line_str(LineId(0)), Some("a"));
        assert_eq!(text.line_str(LineId(1)), Some("b"));
    }

    #[test]
    fn indexes_crlf_and_trims_line_endings() {
        let text = TextStore::from_text("a\r\nb");
        assert_eq!(text.line_count(), 2);
        assert_eq!(text.line_str(LineId(0)), Some("a"));
        assert_eq!(text.line_str(LineId(1)), Some("b"));
        assert!(text.no_newline_at_eof());
    }

    #[test]
    fn empty_file_has_zero_lines() {
        let text = TextStore::default();
        assert_eq!(text.line_count(), 0);
        assert_eq!(text.line_range(LineId(0)), None);
        assert!(!text.no_newline_at_eof());
    }
}
