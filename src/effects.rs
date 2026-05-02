use std::path::PathBuf;

use crate::ai::Provider;
use crate::core::compare::{CompareSpec, RendererKind};
use crate::core::forge::github::CreatePullRequestReviewComment;
use crate::core::syntax::annotator::SyntaxRowWindow;
use crate::core::update::{AvailableUpdate, StagedUpdate};
use crate::core::vcs::git::status::StatusScope;
use crate::core::vcs::git::{StatusItem, StatusOperation};
use crate::events::RepositorySyncReason;
use crate::platform::persistence::Settings;
use crate::platform::secrets::AiKeyKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareRequest {
    pub repo_path: PathBuf,
    pub spec: CompareSpec,
    pub github_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareWorkPriority {
    InteractiveSelectedFile,
    VisibleSidebarStats,
    VisibleViewportDiff,
    Overscan,
    TotalStats,
    Warmup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareStatsRequest {
    pub repo_path: PathBuf,
    pub spec: CompareSpec,
    pub priority: CompareWorkPriority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareHistoryRequest {
    pub repo_path: PathBuf,
    pub left_ref: String,
    pub right_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareFileRequest {
    pub repo_path: PathBuf,
    pub spec: CompareSpec,
    pub path: String,
    pub index: usize,
    pub deferred_file: Option<carbon::FileDiff>,
    pub priority: CompareWorkPriority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareFileStatsItem {
    pub index: usize,
    pub file: carbon::FileDiff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareFileStatsRequest {
    pub repo_path: PathBuf,
    pub files: Vec<CompareFileStatsItem>,
    pub priority: CompareWorkPriority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusDiffRequest {
    pub repo_path: PathBuf,
    pub item: StatusItem,
    pub renderer: RendererKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadFileSyntaxRequest {
    pub repo_path: PathBuf,
    pub file_index: usize,
    pub path: String,
    pub carbon_file: carbon::FileDiff,
    pub left_ref: String,
    pub right_ref: String,
    pub window: SyntaxRowWindow,
    pub request_id: u64,
    pub cache_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusOperationRequest {
    pub repo_path: PathBuf,
    pub item: StatusItem,
    pub operation: StatusOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchStatusOperationRequest {
    pub repo_path: PathBuf,
    pub items: Vec<StatusItem>,
    pub operation: StatusOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchOperationRequest {
    pub repo_path: PathBuf,
    pub patch: String,
    pub scope: StatusScope,
    pub operation: StatusOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    pub repo_path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRemoteRequest {
    pub repo_path: PathBuf,
    pub remote: String,
    pub toast_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRequest {
    pub repo_path: PathBuf,
    pub remote: String,
    pub refspec: String,
    pub force_with_lease: bool,
    pub toast_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullFfRequest {
    pub repo_path: PathBuf,
    pub remote: String,
    pub branch: String,
    pub toast_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task<T> {
    pub generation: u64,
    pub request: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEffect {
    OpenRepositoryDialog,
    OpenBrowser { url: String },
    SetClipboard(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryEffect {
    WatchRepository {
        path: Option<PathBuf>,
    },
    SyncRepository {
        path: PathBuf,
        reason: RepositorySyncReason,
        reporter_generation: Option<u64>,
    },
    FetchRemote(FetchRemoteRequest),
    Push(PushRequest),
    PullFf(PullFfRequest),
    LoadStatusDiff {
        task: Task<StatusDiffRequest>,
        index: usize,
    },
    ApplyStatusOperation(StatusOperationRequest),
    ApplyBatchStatusOperation(BatchStatusOperationRequest),
    ApplyPatchOperation(PatchOperationRequest),
    CreateCommit(CommitRequest),
    FetchContextLines(FetchContextLinesRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareEffect {
    Run(Task<CompareRequest>),
    LoadStats(Task<CompareStatsRequest>),
    LoadHistory(Task<CompareHistoryRequest>),
    LoadFile(Task<CompareFileRequest>),
    LoadFileStats(Task<CompareFileStatsRequest>),
    ResolveRef {
        repo_path: PathBuf,
        query: String,
        generation: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubEffect {
    LoadPullRequest {
        url: String,
        repo_path: PathBuf,
        github_token: Option<String>,
    },
    StartDeviceFlow {
        client_id: String,
    },
    PollDeviceFlow {
        client_id: String,
        device_code: String,
        interval_seconds: u32,
    },
    FetchGitHubUser {
        token: String,
    },
    FetchAvatar {
        url: String,
    },
    PeekPullRequest {
        owner: String,
        repo: String,
        number: i32,
        github_token: Option<String>,
    },
    FetchPullRequestReviewComments {
        owner: String,
        repo: String,
        number: i32,
        github_token: Option<String>,
    },
    CreatePullRequestReviewComment {
        owner: String,
        repo: String,
        number: i32,
        github_token: Option<String>,
        comment: CreatePullRequestReviewComment,
    },
    LoadGitHubToken,
    SaveGitHubToken(String),
    ClearGitHubToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsEffect {
    SaveSettings(Settings),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateEffect {
    CheckForUpdates {
        silent: bool,
    },
    StageUpdate {
        update: AvailableUpdate,
        silent: bool,
    },
    ApplyStagedUpdate(StagedUpdate),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxEffect {
    LoadFileSyntax(Task<LoadFileSyntaxRequest>),
    InstallCommonSyntaxPacks,
    EnsureSyntaxPackForPath { path: String },
    EnsureSyntaxPacksForPaths { paths: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiEffect {
    LoadAiKeys,
    SaveAiKey { kind: AiKeyKind, value: String },
    ClearAiKey { kind: AiKeyKind },
    GenerateCommitMessage(GenerateCommitMessageRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Ui(UiEffect),
    Repository(RepositoryEffect),
    Compare(CompareEffect),
    GitHub(GitHubEffect),
    Settings(SettingsEffect),
    Update(UpdateEffect),
    Syntax(SyntaxEffect),
    Ai(AiEffect),
}

impl From<UiEffect> for Effect {
    fn from(effect: UiEffect) -> Self {
        Self::Ui(effect)
    }
}

impl From<RepositoryEffect> for Effect {
    fn from(effect: RepositoryEffect) -> Self {
        Self::Repository(effect)
    }
}

impl From<CompareEffect> for Effect {
    fn from(effect: CompareEffect) -> Self {
        Self::Compare(effect)
    }
}

impl From<GitHubEffect> for Effect {
    fn from(effect: GitHubEffect) -> Self {
        Self::GitHub(effect)
    }
}

impl From<SettingsEffect> for Effect {
    fn from(effect: SettingsEffect) -> Self {
        Self::Settings(effect)
    }
}

impl From<UpdateEffect> for Effect {
    fn from(effect: UpdateEffect) -> Self {
        Self::Update(effect)
    }
}

impl From<SyntaxEffect> for Effect {
    fn from(effect: SyntaxEffect) -> Self {
        Self::Syntax(effect)
    }
}

impl From<AiEffect> for Effect {
    fn from(effect: AiEffect) -> Self {
        Self::Ai(effect)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateCommitMessageRequest {
    pub repo_path: PathBuf,
    pub has_staged: bool,
    pub provider: Provider,
    pub api_key: String,
    pub steering_prompt: String,
    pub subject_override: Option<String>,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchContextLinesRequest {
    pub repo_path: PathBuf,
    pub old_reference: String,
    pub new_reference: String,
    pub path: String,
    pub generation: u64,
    pub file_index: usize,
    pub hunk_index: usize,
    pub direction: crate::events::ContextDirection,
    pub amount: u32,
}
