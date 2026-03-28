use std::fmt::{self, Display};
use std::str::FromStr;

use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum CompareMode {
    #[default]
    TwoDot,
    ThreeDot,
    SingleCommit,
}

impl CompareMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TwoDot => "two-dot",
            Self::ThreeDot => "three-dot",
            Self::SingleCommit => "single-commit",
        }
    }
}

impl Display for CompareMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CompareMode {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "two-dot" => Ok(Self::TwoDot),
            "three-dot" => Ok(Self::ThreeDot),
            "single-commit" => Ok(Self::SingleCommit),
            _ => Err("invalid compare mode"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum LayoutMode {
    #[default]
    Unified,
    Split,
}

impl LayoutMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unified => "unified",
            Self::Split => "split",
        }
    }
}

impl Display for LayoutMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LayoutMode {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "unified" => Ok(Self::Unified),
            "split" => Ok(Self::Split),
            _ => Err("invalid layout mode"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum RendererKind {
    #[default]
    Builtin,
    Difftastic,
}

impl RendererKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Difftastic => "difftastic",
        }
    }
}

impl Display for RendererKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RendererKind {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "builtin" => Ok(Self::Builtin),
            "difftastic" => Ok(Self::Difftastic),
            _ => Err("invalid renderer kind"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CompareSpec {
    pub mode: CompareMode,
    pub left_ref: String,
    pub right_ref: String,
    pub renderer: RendererKind,
    pub layout: LayoutMode,
}

#[cfg(test)]
mod tests {
    use super::{CompareMode, LayoutMode, RendererKind};

    #[test]
    fn compare_mode_roundtrip() {
        let parsed: CompareMode = "three-dot".parse().unwrap();
        assert_eq!(parsed, CompareMode::ThreeDot);
        assert_eq!(parsed.to_string(), "three-dot");
    }

    #[test]
    fn layout_and_renderer_roundtrip() {
        let layout: LayoutMode = "split".parse().unwrap();
        let renderer: RendererKind = "difftastic".parse().unwrap();
        assert_eq!(layout.to_string(), "split");
        assert_eq!(renderer.to_string(), "difftastic");
    }
}
