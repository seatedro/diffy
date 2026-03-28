use std::path::PathBuf;

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Bootstrap,
    OpenRepository(PathBuf),
    SetLeftRef(String),
    SetRightRef(String),
    SetCompareMode(CompareMode),
    SetLayoutMode(LayoutMode),
    SetRenderer(RendererKind),
    StartCompare,
    SelectFile(usize),
    SelectFilePath(String),
    OpenPullRequest(String),
    StartGitHubDeviceFlow,
    DismissToast(usize),
    ToggleWrap,
    SetWrapColumn(u32),
}
