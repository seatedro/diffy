use std::path::PathBuf;

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::ui::state::FocusTarget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Bootstrap,
    OpenRepositoryDialog,
    OpenRepository(PathBuf),
    SetLeftRef(String),
    SetRightRef(String),
    SetCompareMode(CompareMode),
    SetLayoutMode(LayoutMode),
    SetRenderer(RendererKind),
    SetFocus(Option<FocusTarget>),
    InsertText(String),
    Backspace,
    SelectRefSuggestion(usize),
    StartCompare,
    SelectFile(usize),
    SelectFilePath(String),
    SelectNextFile,
    SelectPreviousFile,
    ScrollFileList(i32),
    ScrollViewportLines(i32),
    ScrollViewportPages(i32),
    ScrollViewportTo(u32),
    HoverViewportRow(Option<usize>),
    FocusViewport,
    HoverFile(Option<usize>),
    OpenPullRequest(String),
    StartGitHubDeviceFlow,
    DismissToast(usize),
    ToggleWrap,
    SetWrapColumn(u32),
}
