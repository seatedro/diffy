use std::path::PathBuf;

use crate::core::compare::{CompareOutput, CompareSpec};
use crate::core::forge::github::{
    DeviceFlowState, GitHubUser, PullRequestInfo, PullRequestReviewComment,
};
use crate::core::syntax::annotator::{SyntaxLineTokens, SyntaxRowWindow};
use crate::core::update::{AvailableUpdate, StagedUpdate};
use crate::core::vcs::git::{BranchInfo, CommitInfo, StatusItem, TagInfo};
use crate::core::vcs::model::{
    ChangeBucket, FileChange, FileChangeStatus, RefKind, RepoCapabilities, RepoLocation, VcsChange,
    VcsRef, VcsSnapshot,
};
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

impl RepositoryChangeKind {
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Both, _) | (_, Self::Both) => Self::Both,
            (Self::Git, Self::Worktree) | (Self::Worktree, Self::Git) => Self::Both,
            (Self::Git, Self::Git) => Self::Git,
            (Self::Worktree, Self::Worktree) => Self::Worktree,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RepositorySnapshot {
    pub location: RepoLocation,
    pub path: PathBuf,
    pub reason: RepositorySyncReason,
    pub change_kind: Option<RepositoryChangeKind>,
    pub capabilities: RepoCapabilities,
    pub refs: Vec<VcsRef>,
    pub changes: Vec<VcsChange>,
    pub file_changes: Vec<FileChange>,
    pub branches: Vec<BranchInfo>,
    pub tags: Vec<TagInfo>,
    pub commits: Vec<CommitInfo>,
    pub status_items: Vec<StatusItem>,
}

impl RepositorySnapshot {
    pub fn from_git_parts(
        path: PathBuf,
        reason: RepositorySyncReason,
        change_kind: Option<RepositoryChangeKind>,
        branches: Vec<BranchInfo>,
        tags: Vec<TagInfo>,
        commits: Vec<CommitInfo>,
        status_items: Vec<StatusItem>,
    ) -> Self {
        let neutral = crate::core::vcs::git::adapter::git_snapshot_from_parts(
            path.clone(),
            reason,
            change_kind,
            &branches,
            &tags,
            &commits,
            &status_items,
        );
        Self::from_git_parts_with_neutral(
            path,
            reason,
            change_kind,
            branches,
            tags,
            commits,
            status_items,
            neutral,
        )
    }

    pub fn from_git_parts_with_neutral(
        path: PathBuf,
        reason: RepositorySyncReason,
        change_kind: Option<RepositoryChangeKind>,
        branches: Vec<BranchInfo>,
        tags: Vec<TagInfo>,
        commits: Vec<CommitInfo>,
        status_items: Vec<StatusItem>,
        neutral: VcsSnapshot,
    ) -> Self {
        Self {
            location: neutral.location,
            path,
            reason,
            change_kind,
            capabilities: neutral.capabilities,
            refs: neutral.refs,
            changes: neutral.changes,
            file_changes: neutral.file_changes,
            branches,
            tags,
            commits,
            status_items,
        }
    }

    pub fn from_vcs_snapshot(snapshot: VcsSnapshot) -> Self {
        let path = snapshot.location.workspace_root.clone();
        let branches = snapshot
            .refs
            .iter()
            .filter_map(branch_info_from_vcs_ref)
            .collect();
        let tags = snapshot
            .refs
            .iter()
            .filter_map(tag_info_from_vcs_ref)
            .collect();
        let commits = snapshot
            .changes
            .iter()
            .map(commit_info_from_vcs_change)
            .collect();
        let status_items = snapshot
            .file_changes
            .iter()
            .map(status_item_from_file_change)
            .collect();
        Self {
            location: snapshot.location,
            path,
            reason: snapshot.reason,
            change_kind: snapshot.change_kind,
            capabilities: snapshot.capabilities,
            refs: snapshot.refs,
            changes: snapshot.changes,
            file_changes: snapshot.file_changes,
            branches,
            tags,
            commits,
            status_items,
        }
    }
}

fn branch_info_from_vcs_ref(reference: &VcsRef) -> Option<BranchInfo> {
    let is_remote = match reference.kind {
        RefKind::Branch => false,
        RefKind::RemoteBranch => true,
        _ => return None,
    };
    Some(BranchInfo {
        name: reference.name.clone(),
        is_remote,
        is_head: reference.active,
        target_oid: reference.target.id.clone(),
        upstream: reference.upstream.clone(),
        ahead_behind: reference.ahead_behind,
    })
}

fn tag_info_from_vcs_ref(reference: &VcsRef) -> Option<TagInfo> {
    (reference.kind == RefKind::Tag).then(|| TagInfo {
        name: reference.name.clone(),
        target_oid: reference.target.id.clone(),
    })
}

fn commit_info_from_vcs_change(change: &VcsChange) -> CommitInfo {
    CommitInfo {
        oid: change.revision.id.clone(),
        short_oid: change.short_revision.clone(),
        summary: change.summary.clone(),
        author_name: change.author_name.clone(),
        timestamp: change.timestamp,
    }
}

fn status_item_from_file_change(change: &FileChange) -> StatusItem {
    let scope = match change.bucket {
        ChangeBucket::Staged => crate::core::vcs::git::StatusScope::Staged,
        ChangeBucket::Untracked => crate::core::vcs::git::StatusScope::Untracked,
        ChangeBucket::WorkingCopy | ChangeBucket::Unstaged | ChangeBucket::Conflicted => {
            crate::core::vcs::git::StatusScope::Unstaged
        }
    };
    StatusItem {
        path: change.path.clone(),
        scope,
        status: file_change_status_label(change.status, change.bucket).to_owned(),
    }
}

fn file_change_status_label(status: FileChangeStatus, bucket: ChangeBucket) -> &'static str {
    match (status, bucket) {
        (FileChangeStatus::Added, _) => "A",
        (FileChangeStatus::Deleted, _) => "D",
        (FileChangeStatus::Renamed, _) => "R",
        (FileChangeStatus::Copied, _) => "C",
        (FileChangeStatus::Untracked, _) => "U",
        (FileChangeStatus::Conflicted, _) | (_, ChangeBucket::Conflicted) => "!",
        (FileChangeStatus::TypeChanged, _) => "T",
        (FileChangeStatus::Binary, _) => "B",
        (FileChangeStatus::Modified, _) => "M",
    }
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
pub struct CompareHistoryReady {
    pub generation: u64,
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
pub struct CompareFileStat {
    pub index: usize,
    pub path: String,
    pub additions: i32,
    pub deletions: i32,
}

#[derive(Debug, Clone)]
pub struct CompareFileStatsReady {
    pub generation: u64,
    pub stats: Vec<CompareFileStat>,
    pub request_complete: bool,
}

#[derive(Debug, Clone)]
pub struct CompareStatsReady {
    pub generation: u64,
    pub additions: i32,
    pub deletions: i32,
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    RepositoryDialogClosed { path: Option<PathBuf> },
    BrowserOpenFailed { message: String },
}

#[derive(Debug, Clone)]
pub enum RepositoryEvent {
    RepositorySnapshotReady(RepositorySnapshot),
    RepositorySnapshotFailed {
        path: PathBuf,
        reason: RepositorySyncReason,
        message: String,
    },
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
    ContextLinesReady(ContextLinesReady),
    ContextLinesFailed {
        generation: u64,
        file_index: usize,
        message: String,
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
}

#[derive(Debug, Clone)]
pub enum CompareEvent {
    CompareFinished(CompareFinished),
    CompareHistoryReady(CompareHistoryReady),
    CompareHistoryFailed {
        generation: u64,
        message: String,
    },
    CompareFailed {
        generation: u64,
        message: String,
    },
    CompareProgressUpdate {
        generation: u64,
        phase: ComparePhase,
    },
    CompareStatsReady(CompareStatsReady),
    CompareStatsFailed {
        generation: u64,
        message: String,
    },
    CompareFileFinished(CompareFileFinished),
    CompareFileStatsReady(CompareFileStatsReady),
    CompareFileStatsFailed {
        generation: u64,
        message: String,
    },
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
    RefResolved {
        query: String,
        generation: u64,
        short_oid: String,
        summary: String,
    },
    RefResolveFailed {
        generation: u64,
    },
}

#[derive(Debug, Clone)]
pub enum GitHubEvent {
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
    PullRequestReviewCommentsLoaded {
        owner: String,
        repo: String,
        number: i32,
        comments: Vec<PullRequestReviewComment>,
    },
    PullRequestReviewCommentsLoadFailed {
        owner: String,
        repo: String,
        number: i32,
        message: String,
    },
    PullRequestReviewCommentCreated {
        owner: String,
        repo: String,
        number: i32,
        comment: PullRequestReviewComment,
    },
    PullRequestReviewCommentCreateFailed {
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
}

#[derive(Debug, Clone)]
pub enum SettingsEvent {
    SettingsSaved,
    SettingsSaveFailed { message: String },
}

#[derive(Debug, Clone)]
pub enum UpdateEvent {
    UpdateAvailable {
        update: AvailableUpdate,
        silent: bool,
    },
    UpdateNotAvailable {
        silent: bool,
    },
    UpdateCheckFailed {
        message: String,
        silent: bool,
    },
    UpdateStaged {
        staged: StagedUpdate,
        silent: bool,
    },
    UpdateInstallFailed {
        message: String,
        silent: bool,
    },
}

#[derive(Debug, Clone)]
pub enum SyntaxEvent {
    FileSyntaxReady(FileSyntaxReady),
    SyntaxPackInstallStarted { language: String },
    SyntaxPackInstalled { language: String },
    SyntaxPackInstallFinished { language: String },
    SyntaxPackInstallFailed { language: String },
}

#[derive(Debug, Clone)]
pub enum AiEvent {
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
pub enum AppEvent {
    Ui(UiEvent),
    Repository(RepositoryEvent),
    Compare(CompareEvent),
    GitHub(GitHubEvent),
    Settings(SettingsEvent),
    Update(UpdateEvent),
    Syntax(SyntaxEvent),
    Ai(AiEvent),
}

impl From<UiEvent> for AppEvent {
    fn from(event: UiEvent) -> Self {
        Self::Ui(event)
    }
}

impl From<RepositoryEvent> for AppEvent {
    fn from(event: RepositoryEvent) -> Self {
        Self::Repository(event)
    }
}

impl From<CompareEvent> for AppEvent {
    fn from(event: CompareEvent) -> Self {
        Self::Compare(event)
    }
}

impl From<GitHubEvent> for AppEvent {
    fn from(event: GitHubEvent) -> Self {
        Self::GitHub(event)
    }
}

impl From<SettingsEvent> for AppEvent {
    fn from(event: SettingsEvent) -> Self {
        Self::Settings(event)
    }
}

impl From<UpdateEvent> for AppEvent {
    fn from(event: UpdateEvent) -> Self {
        Self::Update(event)
    }
}

impl From<SyntaxEvent> for AppEvent {
    fn from(event: SyntaxEvent) -> Self {
        Self::Syntax(event)
    }
}

impl From<AiEvent> for AppEvent {
    fn from(event: AiEvent) -> Self {
        Self::Ai(event)
    }
}

#[derive(Debug, Clone)]
pub struct ContextLinesReady {
    pub generation: u64,
    pub file_index: usize,
    pub path: String,
    pub hunk_index: usize,
    pub direction: ContextDirection,
    pub amount: u32,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
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
