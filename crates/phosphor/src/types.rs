use carbon::{TextByteRange, u32_to_usize_saturating, usize_to_u32_saturating};

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HighlightKind {
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HighlightSpan {
    pub offset: u32,
    pub length: u32,
    pub kind: HighlightKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HighlightSpanRange {
    pub offset: u32,
    pub count: u32,
}

impl HighlightSpanRange {
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HighlightLine {
    pub byte_range: TextByteRange,
    pub spans: HighlightSpanRange,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HighlightLineBuffer {
    lines: Vec<HighlightLine>,
    spans: Vec<HighlightSpan>,
}

impl HighlightLineBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(line_capacity: usize, span_capacity: usize) -> Self {
        Self {
            lines: Vec::with_capacity(line_capacity),
            spans: Vec::with_capacity(span_capacity),
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.spans.clear();
    }

    pub fn lines(&self) -> &[HighlightLine] {
        &self.lines
    }

    pub fn spans(&self) -> &[HighlightSpan] {
        &self.spans
    }

    pub fn spans_for_line(&self, line: HighlightLine) -> &[HighlightSpan] {
        let start = u32_to_usize_saturating(line.spans.offset);
        let end = start.saturating_add(u32_to_usize_saturating(line.spans.count));
        self.spans.get(start..end).unwrap_or(&[])
    }

    pub(crate) fn span_count(&self) -> usize {
        self.spans.len()
    }

    pub(crate) fn push_span(&mut self, span: HighlightSpan) {
        self.spans.push(span);
    }

    pub(crate) fn push_line_range(
        &mut self,
        byte_range: TextByteRange,
        offset: usize,
        count: usize,
    ) {
        self.lines.push(HighlightLine {
            byte_range,
            spans: HighlightSpanRange {
                offset: usize_to_u32_saturating(offset),
                count: usize_to_u32_saturating(count),
            },
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageId {
    Bash,
    C,
    Cpp,
    Go,
    JavaScript,
    Json,
    Nix,
    Python,
    Rust,
    Toml,
    TypeScript,
    TypeScriptTsx,
    Zig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageMetadata {
    pub id: LanguageId,
    pub extensions: &'static [&'static str],
    pub common: bool,
}

impl LanguageId {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Go => "go",
            Self::JavaScript => "javascript",
            Self::Json => "json",
            Self::Nix => "nix",
            Self::Python => "python",
            Self::Rust => "rust",
            Self::Toml => "toml",
            Self::TypeScript => "typescript",
            Self::TypeScriptTsx => "tsx",
            Self::Zig => "zig",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "bash" => Some(Self::Bash),
            "c" => Some(Self::C),
            "cpp" => Some(Self::Cpp),
            "go" => Some(Self::Go),
            "javascript" => Some(Self::JavaScript),
            "json" => Some(Self::Json),
            "nix" => Some(Self::Nix),
            "python" => Some(Self::Python),
            "rust" => Some(Self::Rust),
            "toml" => Some(Self::Toml),
            "typescript" => Some(Self::TypeScript),
            "tsx" => Some(Self::TypeScriptTsx),
            "zig" => Some(Self::Zig),
            _ => None,
        }
    }
}

impl std::fmt::Display for LanguageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}
