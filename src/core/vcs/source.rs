use std::sync::Arc;

use diffy_diff::TextStore;

const BINARY_PROBE_BYTES: usize = 8000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceText {
    Text(TextStore),
    Bytes(Arc<[u8]>),
}

impl Default for SourceText {
    fn default() -> Self {
        Self::Text(TextStore::default())
    }
}

impl SourceText {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        if probe_binary(&bytes) {
            return Self::Bytes(bytes.into());
        }
        match String::from_utf8(bytes) {
            Ok(text) => Self::Text(TextStore::new(text)),
            Err(error) => Self::Bytes(error.into_bytes().into()),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Text(text) => text.as_str().as_bytes(),
            Self::Bytes(bytes) => bytes,
        }
    }

    pub fn text(&self) -> Option<&TextStore> {
        match self {
            Self::Text(text) => Some(text),
            Self::Bytes(_) => None,
        }
    }

    pub fn is_binary(&self) -> bool {
        match self {
            Self::Text(_) => false,
            Self::Bytes(bytes) => probe_binary(bytes),
        }
    }

    pub fn is_utf8(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    pub fn len(&self) -> usize {
        self.bytes().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn probe_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct SourceIdentity {
    pub old: Option<Arc<str>>,
    pub new: Option<Arc<str>>,
}

impl SourceIdentity {
    pub fn matches(&self, other: &SourceIdentity) -> bool {
        object_matches(self.old.as_deref(), other.old.as_deref())
            && object_matches(self.new.as_deref(), other.new.as_deref())
    }
}

fn object_matches(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            let len = left.len().min(right.len());
            len >= 7 && left[..len].eq_ignore_ascii_case(&right[..len])
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_sources_share_one_text_store() {
        let source = SourceText::from_bytes(b"a\r\nb".to_vec());
        let text = source.text().expect("utf-8 text");
        assert_eq!(text.line(0), Some("a\r"));
        assert_eq!(source.bytes(), b"a\r\nb");
        assert!(!source.is_binary());
    }

    #[test]
    fn invalid_utf8_keeps_exact_bytes() {
        let source = SourceText::from_bytes(vec![b'a', 0xff, b'\n']);
        assert!(source.text().is_none());
        assert_eq!(source.bytes(), &[b'a', 0xff, b'\n']);
        assert!(!source.is_binary());
    }

    #[test]
    fn nul_bytes_mark_binary() {
        let source = SourceText::from_bytes(b"a\0b".to_vec());
        assert!(source.is_binary());
        assert!(source.text().is_none());
    }

    #[test]
    fn identity_accepts_abbreviated_objects() {
        let full = SourceIdentity {
            old: Some(Arc::from("0123456789abcdef")),
            new: None,
        };
        let short = SourceIdentity {
            old: Some(Arc::from("0123456")),
            new: None,
        };
        let other = SourceIdentity {
            old: Some(Arc::from("0123457")),
            new: None,
        };
        assert!(full.matches(&short));
        assert!(!full.matches(&other));
    }
}
