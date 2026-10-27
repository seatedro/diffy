use std::sync::Arc;

use diffy_diff::SourceRemap;
use diffy_ui::DiffEvent;
use diffy_ui::diff_view::decorator::LinePointer;

use crate::contracts::shared::{
    CompareGeneration, FileKey, FileRevision, JobId, Side, SourceTarget,
};
use crate::contracts::shell::{Action, AppEvent, Effect};
use crate::contracts::viewport::pipeline::{PreparedDiff, SlotUpdate};
use crate::contracts::viewport::{ViewportAction, ViewportEffect, ViewportEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextDirection {
    Above,
    Below,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffInput(pub DiffEvent);

impl Eq for DiffInput {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineInput(pub LinePointer);

impl Eq for LineInput {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionAction {
    Diff(DiffInput),
    Line(LineInput),
    ClearSelection,
    ToggleSearchCaseSensitive,
    ToggleTextCompareLanguageMenu,
    ScrollTextCompare(TextCompareSide, i32),
    ScrollLines(i32),
    ScrollHalfPage(i32),
    ScrollPages(i32),
    ScrollToTop,
    ScrollToBottom,
    MoveRowCursor(i32),
    GoToThread(i32),
    GoToNextHunk,
    GoToPreviousHunk,
    GoToNextFile,
    GoToPreviousFile,
    OpenSearch,
    CloseSearch,
    SearchNext,
    SearchPrevious,
    CopySelection,
    AnnotateSelection,
    /// The gutter comment control of a line: the selection when it covers
    /// the line, else the line.
    AnnotateLine(SourceTarget),
    /// A drag from a line's gutter comment control: the press
    /// (`drag: false`, position unused) marks that line, each move the
    /// lines from it to the one under the pointer.
    DragAnnotation(SourceTarget, LineInput),
    SetSourceSelection(Option<SourceTarget>),
    ExpandContext {
        file: FileKey,
        hunk: u32,
        direction: ContextDirection,
        amount: u32,
    },
    ExpandAllContext,
    SetTextCompareView(TextCompareView),
    SwapTextCompareSides,
    ClearTextCompareSide(TextCompareSide),
    SetTextCompareLanguage(TextCompareLanguage),
    CompareText,
}

#[derive(Debug, Clone)]
pub enum ViewportInstall {
    Begin {
        generation: CompareGeneration,
    },
    File {
        prepared: PreparedDiff,
        remap: Option<SourceRemap>,
    },
    Failed {
        generation: CompareGeneration,
        key: FileKey,
        revision: FileRevision,
        message: String,
    },
    Remove {
        key: FileKey,
        revision: FileRevision,
    },
    Order {
        generation: CompareGeneration,
        files: Arc<[FileKey]>,
    },
    TextCompare {
        prepared: PreparedDiff,
    },
}

#[derive(Debug, Clone)]
pub enum InstallItem {
    View(ViewportInstall),
    Slot(SlotUpdate),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    pub job: JobId,
    pub query: String,
    pub case_sensitive: bool,
    pub documents: Vec<PreparedDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionEffect {
    Search(Box<SearchRequest>),
    CancelSearch(JobId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SearchMatch {
    pub file: FileKey,
    pub revision: FileRevision,
    pub side: Side,
    pub line: u32,
    pub start_byte: u32,
    pub end_byte: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionFailure {
    NotPorted,
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum InteractionEvent {
    SearchFinished {
        job: JobId,
        matches: Arc<[SearchMatch]>,
    },
    SearchFailed {
        job: JobId,
        failure: InteractionFailure,
    },
}

impl From<InteractionAction> for ViewportAction {
    fn from(value: InteractionAction) -> Self {
        Self::Interaction(value)
    }
}

impl From<InteractionAction> for Action {
    fn from(value: InteractionAction) -> Self {
        Self::Viewport(ViewportAction::Interaction(value))
    }
}

impl From<InteractionEffect> for ViewportEffect {
    fn from(value: InteractionEffect) -> Self {
        Self::Interaction(value)
    }
}

impl From<InteractionEffect> for Effect {
    fn from(value: InteractionEffect) -> Self {
        Self::Viewport(ViewportEffect::Interaction(value))
    }
}

impl From<InteractionEvent> for ViewportEvent {
    fn from(value: InteractionEvent) -> Self {
        Self::Interaction(value)
    }
}

impl From<InteractionEvent> for AppEvent {
    fn from(value: InteractionEvent) -> Self {
        Self::Viewport(ViewportEvent::Interaction(value))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_through_root_contracts() {
        assert_eq!(
            Action::from(InteractionAction::GoToNextHunk),
            Action::Viewport(ViewportAction::Interaction(InteractionAction::GoToNextHunk))
        );
        assert_eq!(
            Effect::from(InteractionEffect::CancelSearch(JobId(3))),
            Effect::Viewport(ViewportEffect::Interaction(
                InteractionEffect::CancelSearch(JobId(3))
            ))
        );
        assert!(matches!(
            AppEvent::from(InteractionEvent::SearchFailed {
                job: JobId(3),
                failure: InteractionFailure::NotPorted
            }),
            AppEvent::Viewport(ViewportEvent::Interaction(
                InteractionEvent::SearchFailed { .. }
            ))
        ));
    }
}
