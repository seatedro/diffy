use std::path::PathBuf;

use crate::core::compare::{CompareOutput, CompareSpec};
use crate::core::syntax::annotator::{SyntaxLineTokens, SyntaxRowWindow};
use crate::core::vcs::git::{BranchInfo, CommitInfo, StatusItem, TagInfo};
use crate::core::vcs::github::{DeviceFlowState, GitHubUser, PullRequestInfo};
use crate::ui::state::{ComparePhase, PreparedActiveFile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositorySyncReason {
    Open,
    Dirty,
    Rescan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryChangeKind {
    Worktree,
    Git,
    Both,
}

#[derive(Debug, Clone)]
pub struct RepositorySnapshot {
    pub path: PathBuf,
    pub reason: RepositorySyncReason,
    pub change_kind: Option<RepositoryChangeKind>,
    pub branches: Vec<BranchInfo>,
    pub tags: Vec<TagInfo>,
    pub commits: Vec<CommitInfo>,
    pub status_items: Vec<StatusItem>,
}

#[derive(Debug, Clone)]
pub struct CompareFinished {
    pub generation: u64,
    pub spec: CompareSpec,
    pub resolved_left: String,
    pub resolved_right: String,
    pub output: CompareOutput,
    pub range_commits: Vec<CommitInfo>,
}

#[derive(Debug, Clone)]
pub struct StatusDiffFinished {
    pub generation: u64,
    pub index: usize,
    pub item: StatusItem,
    pub output: CompareOutput,
}

#[derive(Debug, Clone)]
pub struct CompareFileFinished {
    pub generation: u64,
    pub index: usize,
    pub path: String,
    pub prepared: PreparedActiveFile,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    RepositoryDialogClosed {
        path: Option<PathBuf>,
    },
    RepositorySnapshotReady(RepositorySnapshot),
    RepositorySnapshotFailed {
        path: PathBuf,
        reason: RepositorySyncReason,
        message: String,
    },
    CompareFinished(CompareFinished),
    CompareFailed {
        generation: u64,
        message: String,
    },
    CompareProgressUpdate {
        generation: u64,
        phase: ComparePhase,
    },
    CompareFileFinished(CompareFileFinished),
    CompareFileFailed {
        generation: u64,
        path: String,
        message: String,
    },
    StatusDiffFinished(StatusDiffFinished),
    StatusDiffFailed {
        generation: u64,
        index: usize,
        message: String,
    },
    FileSyntaxReady(FileSyntaxReady),
    StatusOperationFailed {
        path: PathBuf,
        message: String,
    },
    CommitCreated {
        path: PathBuf,
    },
    CommitFailed {
        path: PathBuf,
        message: String,
    },
    PullRequestLoaded {
        url: String,
        info: PullRequestInfo,
        left_ref: String,
        right_ref: String,
    },
    PullRequestLoadFailed {
        url: String,
        message: String,
    },
    PullRequestPeeked {
        owner: String,
        repo: String,
        number: i32,
        info: PullRequestInfo,
    },
    PullRequestPeekFailed {
        owner: String,
        repo: String,
        number: i32,
        message: String,
    },
    DeviceFlowStarted(DeviceFlowState),
    DeviceFlowStartFailed {
        message: String,
    },
    DeviceFlowCompleted {
        token: String,
    },
    DeviceFlowFailed {
        message: String,
    },
    GitHubTokenLoaded {
        token: Option<String>,
    },
    GitHubTokenLoadFailed {
        message: String,
    },
    GitHubTokenSaveFailed {
        message: String,
    },
    GitHubUserFetched {
        user: GitHubUser,
    },
    GitHubUserFetchFailed {
        message: String,
    },
    AvatarFetched {
        url: String,
        rgba: std::sync::Arc<Vec<u8>>,
        width: u32,
        height: u32,
    },
    AvatarFetchFailed {
        url: String,
        message: String,
    },
    RefResolved {
        query: String,
        generation: u64,
        short_oid: String,
        summary: String,
    },
    RefResolveFailed {
        generation: u64,
    },
    SettingsSaved,
    SettingsSaveFailed {
        message: String,
    },
    BrowserOpenFailed {
        message: String,
    },
    ContextLinesReady(ContextLinesReady),
    ContextLinesFailed {
        generation: u64,
        file_index: usize,
        message: String,
    },
    SyntaxPackInstallStarted {
        language: String,
    },
    SyntaxPackInstalled {
        language: String,
    },
    SyntaxPackInstallFinished {
        language: String,
    },
    SyntaxPackInstallFailed {
        language: String,
    },
    FetchProgress {
        toast_id: u64,
        received_objects: usize,
        total_objects: usize,
        received_bytes: usize,
    },
    FetchComplete {
        toast_id: u64,
        path: PathBuf,
        remote: String,
    },
    FetchFailed {
        toast_id: u64,
        remote: String,
        message: String,
    },
    PushProgress {
        toast_id: u64,
        current: usize,
        total: usize,
        bytes: usize,
    },
    PushComplete {
        toast_id: u64,
        path: PathBuf,
        remote: String,
        branch: String,
    },
    PushFailed {
        toast_id: u64,
        remote: String,
        message: String,
    },
    PullComplete {
        toast_id: u64,
        path: PathBuf,
        remote: String,
        branch: String,
        already_up_to_date: bool,
        behind: usize,
    },
    PullFailed {
        toast_id: u64,
        remote: String,
        branch: String,
        message: String,
    },
    AiKeysLoaded {
        openai: Option<String>,
        anthropic: Option<String>,
    },
    AiKeysLoadFailed {
        message: String,
    },
    AiKeySaveFailed {
        message: String,
    },
    CommitMessageChunk {
        generation: u64,
        chunk: String,
    },
    CommitMessageGenerationFinished {
        generation: u64,
    },
    CommitMessageGenerationFailed {
        generation: u64,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct ContextLinesReady {
    pub generation: u64,
    pub file_index: usize,
    pub path: String,
    pub hunk_index: usize,
    pub direction: ContextDirection,
    pub amount: u32,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextDirection {
    Above,
    Below,
    All,
}

#[derive(Debug, Clone)]
pub struct FileSyntaxReady {
    pub generation: u64,
    pub request_id: u64,
    pub file_index: usize,
    pub path: String,
    pub window: SyntaxRowWindow,
    pub tokens: Vec<SyntaxLineTokens>,
}
