use serde::Serialize;

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum SyntaxTokenKind {
    #[default]
    Normal = 0,
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Operator,
    Punctuation,
    Variable,
    Constant,
    Builtin,
    Attribute,
    Tag,
    Property,
    Namespace,
    Label,
    Preprocessor,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DiffTokenSpan {
    pub offset: u32,
    pub length: u32,
    pub kind: SyntaxTokenKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TokenRange {
    pub offset: usize,
    pub count: usize,
}

impl TokenRange {
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }
}

#[derive(Debug, Clone, Default)]
pub struct TokenBuffer {
    spans: Vec<DiffTokenSpan>,
}

impl TokenBuffer {
    pub fn append(&mut self, spans: &[DiffTokenSpan]) -> TokenRange {
        let range = TokenRange {
            offset: self.spans.len(),
            count: spans.len(),
        };
        self.spans.extend_from_slice(spans);
        range
    }

    pub fn view(&self, range: TokenRange) -> &[DiffTokenSpan] {
        let end = range.offset.saturating_add(range.count);
        self.spans.get(range.offset..end).unwrap_or(&[])
    }

    pub fn clear(&mut self) {
        self.spans.clear();
    }

    pub fn reserve(&mut self, additional: usize) {
        self.spans.reserve(additional);
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }
}
