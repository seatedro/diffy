use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TextRange {
    pub offset: usize,
    pub len: usize,
}

impl TextRange {
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

#[derive(Debug, Clone, Default)]
pub struct TextBuffer {
    storage: String,
}

impl TextBuffer {
    pub fn append(&mut self, text: &str) -> TextRange {
        let range = TextRange {
            offset: self.storage.len(),
            len: text.len(),
        };
        self.storage.push_str(text);
        range
    }

    pub fn view(&self, range: TextRange) -> &str {
        if range.is_empty() {
            return "";
        }
        let end = range.offset.saturating_add(range.len);
        self.storage.get(range.offset..end).unwrap_or("")
    }

    pub fn clear(&mut self) {
        self.storage.clear();
    }

    pub fn size(&self) -> usize {
        self.storage.len()
    }

    pub fn reserve(&mut self, additional: usize) {
        self.storage.reserve(additional);
    }
}

#[cfg(test)]
mod tests {
    use super::{TextBuffer, TextRange};

    #[test]
    fn append_and_view_across_chunks() {
        let mut buffer = TextBuffer::default();
        let first = buffer.append("hello");
        let second = buffer.append(" world");
        let third = buffer.append("!");

        assert_eq!(buffer.size(), 12);
        assert_eq!(buffer.view(first), "hello");
        assert_eq!(buffer.view(second), " world");
        assert_eq!(buffer.view(third), "!");
        assert_eq!(buffer.view(TextRange { offset: 3, len: 7 }), "lo worl");
    }

    #[test]
    fn empty_append_preserves_offsets() {
        let mut buffer = TextBuffer::default();
        let empty = buffer.append("");
        let text = buffer.append("abc");

        assert_eq!(empty.len, 0);
        assert_eq!(text.offset, 0);
        assert_eq!(text.len, 3);
        assert_eq!(buffer.view(text), "abc");
    }
}
