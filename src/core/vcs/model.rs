use std::path::PathBuf;

use serde::Serialize;

use crate::core::compare::{LayoutMode, RendererKind};
use crate::events::{RepositoryChangeKind, RepositorySyncReason};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum VcsKind {
    Git,
    Jj,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepoLocation {
    pub kind: VcsKind,
    pub workspace_root: PathBuf,
    pub store_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RepoCapabilities {
    pub staging_area: bool,
    pub branches: bool,
    pub bookmarks: bool,
    pub tags: bool,
    pub remotes: bool,
    pub pull_fast_forward: bool,
    pub partial_file_restore: bool,
    pub partial_hunk_mutation: bool,
    pub operation_log: bool,
    pub github_pull_requests: bool,
}

impl RepoCapabilities {
    pub const fn git() -> Self {
        Self {
            staging_area: true,
            branches: true,
            bookmarks: false,
            tags: true,
            remotes: true,
            pull_fast_forward: true,
            partial_file_restore: true,
            partial_hunk_mutation: true,
            operation_log: false,
            github_pull_requests: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct RevisionId {
    pub backend: VcsKind,
    pub id: String,
}

impl RevisionId {
    pub fn git(id: impl Into<String>) -> Self {
        Self {
            backend: VcsKind::Git,
            id: id.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RefKind {
    Branch,
    RemoteBranch,
    Bookmark,
    RemoteBookmark,
    Tag,
    Head,
    WorkingCopy,
    PullRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VcsRef {
    pub name: String,
    pub kind: RefKind,
    pub target: RevisionId,
    pub active: bool,
    pub upstream: Option<String>,
    pub ahead_behind: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VcsChange {
    pub revision: RevisionId,
    pub change_id: Option<String>,
    pub short_revision: String,
    pub summary: String,
    pub author_name: String,
    pub timestamp: i64,
    pub flags: ChangeFlags,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ChangeFlags {
    pub current: bool,
    pub working_copy: bool,
    pub divergent: bool,
    pub immutable: bool,
    pub conflicted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FileChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Binary,
    Conflicted,
    Untracked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ChangeBucket {
    WorkingCopy,
    Staged,
    Unstaged,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileChangeStatus,
    pub bucket: ChangeBucket,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VcsCompareSpec {
    WorkingCopy,
    Change { revision: String },
    Range { from: String, to: String },
    MergeBaseRange { base: String, head: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcsCompareRequest {
    pub spec: VcsCompareSpec,
    pub layout: LayoutMode,
    pub renderer: RendererKind,
}

#[derive(Debug, Clone)]
pub struct VcsSnapshot {
    pub location: RepoLocation,
    pub reason: RepositorySyncReason,
    pub change_kind: Option<RepositoryChangeKind>,
    pub capabilities: RepoCapabilities,
    pub refs: Vec<VcsRef>,
    pub changes: Vec<VcsChange>,
    pub file_changes: Vec<FileChange>,
}
