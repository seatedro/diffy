use std::path::PathBuf;

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::core::vcs::model::{PublishAction, VcsOperation};
use crate::platform::secrets::AiKeyKind;
use crate::ui::state::{CompareField, FocusTarget, SettingsSection, SidebarTab};
use crate::ui::theme::ThemeMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    Bootstrap,
    OpenRepositoryDialog,
    SetFocus(Option<FocusTarget>),
    CopyText(String),
    DismissToast(usize),
    HoverToast(Option<usize>),
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceAction {
    OpenRepository(PathBuf),
    ShowWorkingTree,
    RefreshRepository,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareAction {
    SetLeftRef(String),
    SetRightRef(String),
    SwapRefs,
    SetActiveRefField(CompareField),
    SwapDraftRefs,
    CommitRefPicker,
    CancelRefPicker,
    SetCompareMode(CompareMode),
    CycleCompareMode,
    OpenCompareMenu,
    ApplyComparePreset(String),
    SetLayoutMode(LayoutMode),
    SetRenderer(RendererKind),
    StartCompare,
    CancelCompare,
    SelectSidebarCommit(String),
    ClearSidebarCommit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryAction {
    StageSelectedFile,
    UnstageSelectedFile,
    DiscardSelectedFile,
    StageFile(usize),
    UnstageFile(usize),
    StageAllFiles,
    UnstageAllFiles,
    StageHunk,
    UnstageHunk,
    DiscardHunk,
    StageHunkAt(i16),
    UnstageHunkAt(i16),
    DiscardHunkAt(i16),
    ToggleLineSelection(usize),
    ToggleLineSelectionRange(usize, usize),
    ToggleCurrentLineSelection,
    ToggleCurrentLineSelectionRange,
    StageSelectedLines,
    UnstageSelectedLines,
    DiscardSelectedLines,
    ClearLineSelection,
    SubmitCommit,
    RunOperation(VcsOperation),
    FetchRemote(String),
    FetchAllRemotes,
    PushCurrentBranch { force_with_lease: bool },
    PublishDefault,
    OpenPublishMenu,
    Publish(PublishAction),
    PullCurrentBranch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileListAction {
    SelectFile(usize),
    SelectFilePath(String),
    SelectNextFile,
    SelectPreviousFile,
    ScrollFileList(i32),
    ScrollFileListPx(i32),
    ScrollFileListToPx(u32),
    HoverFile(Option<usize>),
    ToggleFolder(String),
    ToggleFileViewed(usize),
    SetSidebarFilter(String),
    ClearSidebarFilter,
    ToggleSidebarMode,
    ToggleSidebar,
    SetSidebarTab(SidebarTab),
    ScrollCommitListPx(i32),
    ExpandAllFolders,
    CollapseAllFolders,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayAction {
    OpenRepoPicker,
    OpenRefPicker(CompareField),
    OpenCommandPalette,
    OpenGitHubAuthModal,
    CloseOverlay,
    MoveOverlaySelection(i32),
    ConfirmOverlaySelection,
    TabCompletePickerDir,
    SelectOverlayEntry(usize),
    HoverOverlayEntry(Option<usize>),
    ScrollActiveOverlayListPx(i32),
    ShowKeyboardShortcuts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorAction {
    ScrollViewportLines(i32),
    ScrollViewportPx(i32),
    ScrollViewportPages(i32),
    ScrollViewportTo(u32),
    ScrollViewportToGlobal(u32),
    BeginViewportScrollbarDrag {
        content_height_px: u32,
        viewport_height_px: u32,
        scroll_top_px: u32,
        max_scroll_top_px: u32,
    },
    EndViewportScrollbarDrag,
    HoverViewportRow(Option<usize>),
    GoToNextHunk,
    GoToPreviousHunk,
    GoToNextFile,
    GoToPreviousFile,
    FocusViewport,
    OpenSearch,
    CloseSearch,
    SearchNext,
    SearchPrevious,
    ScrollViewportHalfPage(i32),
    MoveRowCursor(i32),
    EditorClick(i32, i32),
    EditorDrag(i32, i32),
    EditorScrollPx(i32),
    ExpandContextAbove(usize, u32),
    ExpandContextBelow(usize, u32),
    ExpandAllContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextEditAction {
    InsertText(String),
    Backspace,
    BackspaceWord,
    BackspaceLine,
    DeleteForward,
    DeleteForwardWord,
    CursorLeft,
    CursorRight,
    CursorUp,
    CursorDown,
    CursorWordLeft,
    CursorWordRight,
    CursorHome,
    CursorEnd,
    CursorSoftHome,
    CursorSoftEnd,
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    SelectWordLeft,
    SelectWordRight,
    SelectHome,
    SelectEnd,
    SelectSoftHome,
    SelectSoftEnd,
    SelectAll,
    Copy,
    Cut,
    Paste(String),
    SetTextCursor(usize),
    ExtendTextSelection(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    ToggleWrap,
    SetWrapColumn(u32),
    SetSidebarWidthPx(u32),
    IncreaseUiScale,
    DecreaseUiScale,
    SetUiScalePct(u16),
    ToggleThemeMode,
    SetThemeMode(ThemeMode),
    SetThemeName(String),
    SetWheelScrollLines(u8),
    ToggleContinuousScroll,
    OpenThemePicker,
    OpenSettings,
    CloseSettings,
    ToggleAutoUpdate,
    SetSettingsSection(SettingsSection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubAction {
    StartGitHubDeviceFlow,
    OpenDeviceFlowBrowser,
    OpenAccountMenu,
    SignOutGitHub,
    OpenReviewCommentComposer,
    SubmitReviewComment,
    CancelReviewComment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateAction {
    CheckForUpdates,
    InstallUpdate,
    RestartToUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowAction {
    Minimize,
    ToggleMaximize,
    Close,
    BeginDrag,
    BeginResize(ResizeEdge),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxAction {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiAction {
    SetAiKey { kind: AiKeyKind, value: String },
    ClearAiKey { kind: AiKeyKind },
    SetAiKeyEditing { kind: AiKeyKind, editing: bool },
    GenerateCommitMessage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    App(AppAction),
    Workspace(WorkspaceAction),
    Compare(CompareAction),
    Repository(RepositoryAction),
    FileList(FileListAction),
    Overlay(OverlayAction),
    Editor(EditorAction),
    TextEdit(TextEditAction),
    Settings(SettingsAction),
    GitHub(GitHubAction),
    Update(UpdateAction),
    Window(WindowAction),
    Syntax(SyntaxAction),
    Ai(AiAction),
    Noop,
}

impl From<AppAction> for Action {
    fn from(action: AppAction) -> Self {
        Self::App(action)
    }
}

impl From<WorkspaceAction> for Action {
    fn from(action: WorkspaceAction) -> Self {
        Self::Workspace(action)
    }
}

impl From<CompareAction> for Action {
    fn from(action: CompareAction) -> Self {
        Self::Compare(action)
    }
}

impl From<RepositoryAction> for Action {
    fn from(action: RepositoryAction) -> Self {
        Self::Repository(action)
    }
}

impl From<FileListAction> for Action {
    fn from(action: FileListAction) -> Self {
        Self::FileList(action)
    }
}

impl From<OverlayAction> for Action {
    fn from(action: OverlayAction) -> Self {
        Self::Overlay(action)
    }
}

impl From<EditorAction> for Action {
    fn from(action: EditorAction) -> Self {
        Self::Editor(action)
    }
}

impl From<TextEditAction> for Action {
    fn from(action: TextEditAction) -> Self {
        Self::TextEdit(action)
    }
}

impl From<SettingsAction> for Action {
    fn from(action: SettingsAction) -> Self {
        Self::Settings(action)
    }
}

impl From<GitHubAction> for Action {
    fn from(action: GitHubAction) -> Self {
        Self::GitHub(action)
    }
}

impl From<UpdateAction> for Action {
    fn from(action: UpdateAction) -> Self {
        Self::Update(action)
    }
}

impl From<WindowAction> for Action {
    fn from(action: WindowAction) -> Self {
        Self::Window(action)
    }
}

impl From<SyntaxAction> for Action {
    fn from(action: SyntaxAction) -> Self {
        Self::Syntax(action)
    }
}

impl From<AiAction> for Action {
    fn from(action: AiAction) -> Self {
        Self::Ai(action)
    }
}

pub fn editor_scroll_px(delta: i32) -> Action {
    EditorAction::EditorScrollPx(delta).into()
}

pub fn scroll_active_overlay_list_px(delta: i32) -> Action {
    OverlayAction::ScrollActiveOverlayListPx(delta).into()
}

pub fn scroll_commit_list_px(delta: i32) -> Action {
    FileListAction::ScrollCommitListPx(delta).into()
}

pub fn select_file(index: usize) -> Action {
    FileListAction::SelectFile(index).into()
}

pub fn toggle_folder(path: String) -> Action {
    FileListAction::ToggleFolder(path).into()
}
