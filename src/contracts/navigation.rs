use std::path::PathBuf;
use std::sync::Arc;

use crate::ai::rank::RankCandidate;
use crate::contracts::shared::{CompareGeneration, FileKey, JobId};
use crate::core::compare::CompareMode;
use crate::core::vcs::model::{
    FileChange, PublishPlan, RepoCapabilities, RepoLocation, VcsChange, VcsOperationLogEntry,
    VcsRef, VcsSnapshot,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompareField {
    #[default]
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarTab {
    #[default]
    Files,
    Commits,
    Outline,
    OperationLog,
}

impl SidebarTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Files => "Changes",
            Self::Commits => "History",
            Self::Outline => "Outline",
            Self::OperationLog => "Ops",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarMode {
    #[default]
    List,
    Tree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositorySyncReason {
    Open,
    Dirty,
    Rescan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryChangeKind {
    Worktree,
    Metadata,
    Both,
}

impl RepositoryChangeKind {
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Both, _) | (_, Self::Both) => Self::Both,
            (Self::Metadata, Self::Worktree) | (Self::Worktree, Self::Metadata) => Self::Both,
            (Self::Metadata, Self::Metadata) => Self::Metadata,
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
    pub operation_log: Vec<VcsOperationLogEntry>,
    pub file_changes: Vec<FileChange>,
    /// The publish plan computed against this exact snapshot, so plan and
    /// refs update atomically. `None` when the backend has nothing
    /// publishable (no remotes, nothing described).
    pub publish_plan: Option<PublishPlan>,
}

impl RepositorySnapshot {
    pub fn from_vcs_snapshot(snapshot: VcsSnapshot) -> Self {
        let path = snapshot.location.workspace_root.clone();
        Self {
            location: snapshot.location,
            path,
            reason: snapshot.reason,
            change_kind: snapshot.change_kind,
            capabilities: snapshot.capabilities,
            refs: snapshot.refs,
            changes: snapshot.changes,
            operation_log: snapshot.operation_log,
            file_changes: snapshot.file_changes,
            publish_plan: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RepositoryRevision(pub u64);

#[derive(Debug, Clone)]
pub struct RepositoryContext {
    pub revision: RepositoryRevision,
    pub snapshot: Arc<RepositorySnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionOrigin {
    Sidebar,
    Viewport,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SelectedFile {
    pub key: FileKey,
    pub origin: SelectionOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationAction {
    OpenRepository(PathBuf),
    OpenRepositoryDialog,
    ShowWorkingTree,
    NewTextCompare,
    RefreshRepository,
    SetRef { field: CompareField, value: String },
    ChooseRef(CompareField),
    SwapRefs,
    ToggleCommittedOnly,
    ResolveRef { field: CompareField, query: String },
    SetCompareMode(CompareMode),
    CycleCompareMode,
    ApplyComparePreset(String),
    StartCompare,
    CancelCompare,
    SelectFile(SelectedFile),
    SelectFilePath(String),
    SelectNextFile,
    SelectPreviousFile,
    ToggleFolder(String),
    ExpandAllFolders,
    CollapseAllFolders,
    ToggleFileViewed(FileKey),
    SetFilter(String),
    ClearFilter,
    EscapeFilter,
    ScrollFiles(i32),
    ScrollFilesTo(u32),
    ToggleSidebarMode,
    ToggleSmartSort,
    ToggleSidebar,
    SetSidebarTab(SidebarTab),
    SelectHistoryCommit(String),
    MoveSelection(i32),
    ClearHistoryCommit,
    CompareWithWorkingTree(String),
    ScrollHistory(i32),
    SelectOperation(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRepositoryRequest {
    pub path: PathBuf,
    pub reason: RepositorySyncReason,
    pub progress: Option<CompareGeneration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRequest {
    pub generation: CompareGeneration,
    pub repo_path: PathBuf,
    pub left_ref: String,
    pub right_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveRefRequest {
    pub request: JobId,
    pub repo_path: PathBuf,
    pub field: CompareField,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankFilesRequest {
    pub job: JobId,
    pub files: Vec<RankCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationEffect {
    WatchRepository { path: Option<PathBuf> },
    WatchCompareWorktree { path: Option<PathBuf> },
    SyncRepository(SyncRepositoryRequest),
    LoadHistory(HistoryRequest),
    ResolveRef(ResolveRefRequest),
}

#[derive(Debug, Clone)]
pub enum NavigationEvent {
    SnapshotReady {
        revision: RepositoryRevision,
        snapshot: Arc<RepositorySnapshot>,
    },
    SnapshotFailed {
        path: PathBuf,
        reason: RepositorySyncReason,
        message: String,
    },
    HistoryReady {
        generation: CompareGeneration,
        commits: Arc<[VcsChange]>,
    },
    HistoryFailed {
        generation: CompareGeneration,
        message: String,
    },
    RefResolved {
        request: JobId,
        field: CompareField,
        query: String,
        short_oid: String,
        summary: String,
    },
    RefResolveFailed {
        request: JobId,
        field: CompareField,
        query: String,
    },
    CompareWorktreeChanged {
        path: PathBuf,
    },
    FilesRanked {
        job: JobId,
        ranking: Result<Vec<u32>, String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::shell::{Action, AppEvent, Effect};

    #[test]
    fn routes_through_root_contracts() {
        assert_eq!(
            Action::from(NavigationAction::SwapRefs),
            Action::Navigation(NavigationAction::SwapRefs)
        );
        assert_eq!(
            Effect::from(NavigationEffect::WatchRepository { path: None }),
            Effect::Navigation(NavigationEffect::WatchRepository { path: None })
        );
        assert!(matches!(
            AppEvent::from(NavigationEvent::RefResolveFailed {
                request: JobId(1),
                field: CompareField::Left,
                query: String::new()
            }),
            AppEvent::Navigation(NavigationEvent::RefResolveFailed { .. })
        ));
    }
}
