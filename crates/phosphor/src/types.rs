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
