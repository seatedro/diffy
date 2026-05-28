use std::path::PathBuf;
use std::sync::Arc;

use crate::ai::Provider;
use crate::core::compare::{CompareFileStatsTarget, CompareFileSummary, RendererKind};
use crate::core::forge::github::{
    CreatePullRequestReview, CreatePullRequestReviewComment, CreatePullRequestReviewReply,
    SubmitPullRequestReview, UpdatePullRequestReviewComment,
};
use crate::core::review::{ReviewDecision, ReviewSession, ReviewTarget};
use crate::core::syntax::annotator::SyntaxRowWindow;
use crate::core::update::{AvailableUpdate, StagedUpdate};
use crate::core::vcs::model::{
    ChangeBucket, FileChange, FileOperation, PublishAction, VcsCompareRequest, VcsOperation,
};
use crate::events::RepositorySyncReason;
use crate::platform::persistence::Settings;
use crate::platform::secrets::AiKeyKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareRequest {
    pub repo_path: PathBuf,
    pub request: VcsCompareRequest,
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

impl CompareWorkPriority {
    pub const fn rank(self) -> u8 {
        match self {
            Self::InteractiveSelectedFile => 60,
            Self::VisibleViewportDiff => 50,
            Self::VisibleSidebarStats => 40,
            Self::Overscan => 30,
            Self::TotalStats => 20,
            Self::Warmup => 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareStatsRequest {
    pub repo_path: PathBuf,
    pub request: VcsCompareRequest,
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
    pub request: VcsCompareRequest,
    pub path: String,
    pub index: usize,
    pub deferred_file: Option<CompareFileSummary>,
    pub priority: CompareWorkPriority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareFileStatsItem {
    pub index: usize,
    pub target: CompareFileStatsTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareFileStatsRequest {
    pub repo_path: PathBuf,
    pub request: VcsCompareRequest,
    pub files: Vec<CompareFileStatsItem>,
    pub priority: CompareWorkPriority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusDiffRequest {
    pub repo_path: PathBuf,
    pub file_change: FileChange,
    pub renderer: RendererKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadFileSyntaxRequest {
    pub repo_path: PathBuf,
    pub file_index: usize,
    pub path: String,
    pub carbon_file: Arc<carbon::FileDiff>,
    pub carbon_expansion: carbon::ExpansionState,
    pub left_ref: String,
    pub right_ref: String,
    pub window: SyntaxRowWindow,
    pub request_id: u64,
    pub cache_generation: u64,
    pub syntax_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOperationRequest {
    pub repo_path: PathBuf,
    pub file_change: FileChange,
    pub operation: FileOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchFileOperationRequest {
    pub repo_path: PathBuf,
    pub file_changes: Vec<FileChange>,
    pub operation: FileOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchOperationRequest {
    pub repo_path: PathBuf,
    pub patch: String,
    pub bucket: ChangeBucket,
    pub operation: FileOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    pub repo_path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcsOperationRequest {
    pub repo_path: PathBuf,
    pub operation: VcsOperation,
    pub toast_id: u64,
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
pub struct PublishRequest {
    pub repo_path: PathBuf,
    pub action: Option<PublishAction>,
    pub toast_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishPlanRequest {
    pub repo_path: PathBuf,
    pub toast_id: Option<u64>,
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
    PublishDefault(PublishRequest),
    LoadPublishPlan(PublishPlanRequest),
    PullFf(PullFfRequest),
    LoadStatusDiff {
        task: Task<StatusDiffRequest>,
        index: usize,
    },
    ApplyFileOperation(FileOperationRequest),
    ApplyBatchFileOperation(BatchFileOperationRequest),
    ApplyPatchOperation(PatchOperationRequest),
    CreateCommit(CommitRequest),
    RunOperation(VcsOperationRequest),
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
    FetchPullRequestReviewData {
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
    CreatePullRequestReviewReply {
        owner: String,
        repo: String,
        number: i32,
        comment_id: i64,
        github_token: Option<String>,
        reply: CreatePullRequestReviewReply,
    },
    UpdatePullRequestReviewComment {
        owner: String,
        repo: String,
        number: i32,
        comment_id: i64,
        github_token: Option<String>,
        update: UpdatePullRequestReviewComment,
    },
    DeletePullRequestReviewComment {
        owner: String,
        repo: String,
        number: i32,
        comment_id: i64,
        github_token: Option<String>,
    },
    CreatePullRequestReview {
        owner: String,
        repo: String,
        number: i32,
        github_token: Option<String>,
        review: CreatePullRequestReview,
    },
    SubmitReviewSessionDrafts {
        session: ReviewSession,
        decision: ReviewDecision,
        body: Option<String>,
        github_token: Option<String>,
    },
    AddPullRequestReviewThreadReply {
        owner: String,
        repo: String,
        number: i32,
        thread_node_id: String,
        review_node_id: Option<String>,
        github_token: Option<String>,
        body: String,
    },
    UpdatePullRequestReviewCommentGraphql {
        owner: String,
        repo: String,
        number: i32,
        comment_node_id: String,
        github_token: Option<String>,
        body: String,
    },
    DeletePullRequestReviewCommentGraphql {
        owner: String,
        repo: String,
        number: i32,
        comment_node_id: String,
        github_token: Option<String>,
    },
    SetPullRequestReviewThreadResolution {
        owner: String,
        repo: String,
        number: i32,
        thread_node_id: String,
        github_token: Option<String>,
        resolved: bool,
    },
    SubmitPullRequestReview {
        owner: String,
        repo: String,
        number: i32,
        review_id: i64,
        github_token: Option<String>,
        submit: SubmitPullRequestReview,
    },
    LoadReviewSession {
        target: ReviewTarget,
        pull_request: crate::core::forge::github::PullRequestInfo,
    },
    SaveReviewSession {
        session: ReviewSession,
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
    SetFileSyntaxEpoch { epoch: u64 },
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
