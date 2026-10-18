use std::borrow::Cow;
use std::sync::Arc;

pub use diffy_diff::ObjectId;
use diffy_diff::TextStore;

const BINARY_PROBE_BYTES: usize = 8000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceText {
    Text(TextStore),
    Binary(Arc<[u8]>),
}

impl Default for SourceText {
    fn default() -> Self {
        Self::Text(TextStore::default())
    }
}

impl SourceText {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        if bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0) {
            return Self::Binary(bytes.into());
        }
        match String::from_utf8(bytes) {
            Ok(text) => Self::Text(TextStore::new(text)),
            Err(error) => Self::Text(TextStore::from_bytes(error.as_bytes())),
        }
    }

    pub fn bytes(&self) -> Cow<'_, [u8]> {
        match self {
            Self::Text(text) => text.to_source_bytes(),
            Self::Binary(bytes) => Cow::Borrowed(bytes),
        }
    }

    pub fn text(&self) -> Option<&TextStore> {
        match self {
            Self::Text(text) => Some(text),
            Self::Binary(_) => None,
        }
    }

    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Binary(_))
    }

    pub fn is_exact_utf8(&self) -> bool {
        self.text().is_some_and(TextStore::is_exact)
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Text(text) => text.source_len(),
            Self::Binary(bytes) => bytes.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct SourceIdentity {
    pub old: Option<ObjectId>,
    pub new: Option<ObjectId>,
}

impl SourceIdentity {
    pub fn matches(&self, other: &SourceIdentity) -> bool {
        let side = |left: &Option<ObjectId>, right: &Option<ObjectId>| match (left, right) {
            (None, None) => true,
            (Some(left), Some(right)) => left.matches(right),
            _ => false,
        };
        side(&self.old, &other.old) && side(&self.new, &other.new)
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
        assert_eq!(&*source.bytes(), b"a\r\nb");
        assert!(source.is_exact_utf8());
    }

    #[test]
    fn invalid_utf8_keeps_exact_bytes() {
        let source = SourceText::from_bytes(vec![b'a', 0xff, b'\n']);
        assert!(!source.is_exact_utf8());
        assert!(!source.is_binary());
        assert_eq!(&*source.bytes(), &[b'a', 0xff, b'\n']);
        assert_eq!(source.len(), 3);
    }

    #[test]
    fn nul_bytes_mark_binary() {
        let source = SourceText::from_bytes(b"a\0b".to_vec());
        assert!(source.is_binary());
        assert!(source.text().is_none());
    }

    #[test]
    fn identity_accepts_abbreviated_objects() {
        let id = |hex: &str| ObjectId::parse(hex);
        let full = SourceIdentity {
            old: id("0123456789abcdef"),
            new: None,
        };
        let short = SourceIdentity {
            old: id("0123456"),
            new: None,
        };
        let other = SourceIdentity {
            old: id("0123457"),
            new: None,
        };
        assert!(full.matches(&short));
        assert!(!full.matches(&other));
    }
}
