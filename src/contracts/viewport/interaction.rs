#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionAction {
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionEffect {
    Noop,
}

#[derive(Debug, Clone)]
pub enum InteractionEvent {
    Noop,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextCompareView {
    #[default]
    Edit,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextCompareSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextCompareLanguage {
    #[default]
    Auto,
    PlainText,
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Json,
    Toml,
    Shell,
    Nix,
    C,
    Cpp,
    Zig,
}

impl TextCompareLanguage {
    pub const OPTIONS: &'static [Self] = &[
        Self::Auto,
        Self::PlainText,
        Self::Rust,
        Self::TypeScript,
        Self::JavaScript,
        Self::Python,
        Self::Go,
        Self::Json,
        Self::Toml,
        Self::Shell,
        Self::Nix,
        Self::C,
        Self::Cpp,
        Self::Zig,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::PlainText => "Plain text",
            Self::Rust => "Rust",
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
            Self::Python => "Python",
            Self::Go => "Go",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Shell => "Shell",
            Self::Nix => "Nix",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Zig => "Zig",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::PlainText => "Text",
            Self::Rust => "Rust",
            Self::TypeScript => "TS",
            Self::JavaScript => "JS",
            Self::Python => "Py",
            Self::Go => "Go",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Shell => "Sh",
            Self::Nix => "Nix",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Zig => "Zig",
        }
    }

    pub fn scratch_path(self) -> &'static str {
        match self {
            Self::Auto | Self::PlainText => "text.txt",
            Self::Rust => "scratch.rs",
            Self::TypeScript => "scratch.ts",
            Self::JavaScript => "scratch.js",
            Self::Python => "scratch.py",
            Self::Go => "scratch.go",
            Self::Json => "scratch.json",
            Self::Toml => "scratch.toml",
            Self::Shell => "scratch.sh",
            Self::Nix => "scratch.nix",
            Self::C => "scratch.c",
            Self::Cpp => "scratch.cpp",
            Self::Zig => "scratch.zig",
        }
    }
}
