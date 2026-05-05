use std::fmt;
use std::path::PathBuf;

use serde::{Serialize, Serializer};

use crate::core::compare::{LayoutMode, RendererKind};
use crate::events::{RepositoryChangeKind, RepositorySyncReason};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VcsKind(&'static str);

impl VcsKind {
    pub const GIT: Self = Self("git");
    pub const JJ: Self = Self("jj");

    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl Serialize for VcsKind {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.0)
    }
}

impl fmt::Display for VcsKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

pub const VCS_PROFILE_GIT: &str = "git";
pub const VCS_PROFILE_JJ: &str = "jj";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepoLocation {
    pub kind: VcsKind,
    pub profile: &'static str,
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
    pub create_commit: bool,
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
            create_commit: true,
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
            backend: VcsKind::GIT,
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
    pub short_change_id: Option<String>,
    pub short_change_id_prefix_len: Option<usize>,
    pub short_revision: String,
    pub summary: String,
    pub author_name: String,
    pub timestamp: i64,
    pub flags: ChangeFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VcsOperationLogEntry {
    pub operation_id: String,
    pub short_operation_id: String,
    pub user: String,
    pub time: String,
    pub description: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOperation {
    Stage,
    Unstage,
    Discard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullFastForwardOutcome {
    AlreadyUpToDate,
    FastForwarded { behind: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JjOperation {
    NewChange,
    NewSiblingChange,
    DuplicateChange,
    AbandonChange,
    SquashIntoParent,
    AbsorbIntoStack,
    UndoLastOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VcsOperation {
    Jj(JjOperation),
    JjRebaseCurrentChangeOnto { destination: String },
    JjEditRevision { revision: String, label: String },
    JjRestoreOperation { operation_id: String, label: String },
}

impl JjOperation {
    pub const ALL: [Self; 7] = [
        Self::NewChange,
        Self::NewSiblingChange,
        Self::DuplicateChange,
        Self::AbandonChange,
        Self::SquashIntoParent,
        Self::AbsorbIntoStack,
        Self::UndoLastOperation,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::NewChange => "New Change",
            Self::NewSiblingChange => "New Sibling Change",
            Self::DuplicateChange => "Duplicate Change",
            Self::AbandonChange => "Abandon Change",
            Self::SquashIntoParent => "Squash Into Parent",
            Self::AbsorbIntoStack => "Absorb Into Stack",
            Self::UndoLastOperation => "Undo Last Operation",
        }
    }

    pub const fn detail(self) -> &'static str {
        match self {
            Self::NewChange => "Create a new change after the current one",
            Self::NewSiblingChange => "Create a new change from the current parent",
            Self::DuplicateChange => "Duplicate the current change",
            Self::AbandonChange => "Abandon the current change and rebase descendants",
            Self::SquashIntoParent => "Move current changes into the parent change",
            Self::AbsorbIntoStack => "Move changed lines into closest mutable ancestors",
            Self::UndoLastOperation => "Create a new jj operation that undoes the previous one",
        }
    }

    pub const fn progress_label(self) -> &'static str {
        match self {
            Self::NewChange => "Creating new jj change",
            Self::NewSiblingChange => "Creating sibling jj change",
            Self::DuplicateChange => "Duplicating jj change",
            Self::AbandonChange => "Abandoning jj change",
            Self::SquashIntoParent => "Squashing jj change",
            Self::AbsorbIntoStack => "Absorbing jj changes",
            Self::UndoLastOperation => "Undoing last jj operation",
        }
    }

    pub const fn success_message(self) -> &'static str {
        match self {
            Self::NewChange => "Created new jj change.",
            Self::NewSiblingChange => "Created sibling jj change.",
            Self::DuplicateChange => "Duplicated jj change.",
            Self::AbandonChange => "Abandoned jj change.",
            Self::SquashIntoParent => "Squashed jj change into parent.",
            Self::AbsorbIntoStack => "Absorbed jj changes into stack.",
            Self::UndoLastOperation => "Undid last jj operation.",
        }
    }

    pub const fn confirmation_message(self) -> Option<&'static str> {
        match self {
            Self::AbandonChange => Some(
                "Abandon @ and rebase descendants onto its parent. You can recover through the jj operation log.",
            ),
            Self::SquashIntoParent => Some(
                "Move all changes from @ into @-. The current change may be abandoned if it becomes empty.",
            ),
            Self::AbsorbIntoStack => Some(
                "Move changed lines from @ into the closest mutable ancestors. Review or recover through the jj operation log.",
            ),
            Self::UndoLastOperation => Some(
                "Create a new jj operation that applies the inverse of the previous operation.",
            ),
            _ => None,
        }
    }
}

impl VcsOperation {
    pub fn label(&self) -> String {
        match self {
            Self::Jj(operation) => operation.label().to_owned(),
            Self::JjRebaseCurrentChangeOnto { destination } => {
                format!("Rebase @ Onto {destination}")
            }
            Self::JjEditRevision { label, .. } => format!("Edit {label}"),
            Self::JjRestoreOperation { label, .. } => format!("Restore Operation {label}"),
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Jj(operation) => operation.detail().to_owned(),
            Self::JjRebaseCurrentChangeOnto { destination } => {
                format!("Move the current jj branch onto {destination}")
            }
            Self::JjEditRevision { label, .. } => {
                format!("Set the working-copy change to {label}")
            }
            Self::JjRestoreOperation { label, .. } => {
                format!("Restore the repository to jj operation {label}")
            }
        }
    }

    pub fn progress_label(&self) -> String {
        match self {
            Self::Jj(operation) => operation.progress_label().to_owned(),
            Self::JjRebaseCurrentChangeOnto { destination } => {
                format!("Rebasing jj change onto {destination}")
            }
            Self::JjEditRevision { label, .. } => format!("Editing jj change {label}"),
            Self::JjRestoreOperation { label, .. } => {
                format!("Restoring jj operation {label}")
            }
        }
    }

    pub fn success_message(&self) -> String {
        match self {
            Self::Jj(operation) => operation.success_message().to_owned(),
            Self::JjRebaseCurrentChangeOnto { destination } => {
                format!("Rebased jj change onto {destination}.")
            }
            Self::JjEditRevision { label, .. } => format!("Editing jj change {label}."),
            Self::JjRestoreOperation { label, .. } => {
                format!("Restored jj repository to operation {label}.")
            }
        }
    }

    pub fn confirmation_message(&self) -> Option<String> {
        match self {
            Self::Jj(operation) => operation.confirmation_message().map(str::to_owned),
            Self::JjRebaseCurrentChangeOnto { destination } => {
                Some(format!("Rebase the current jj branch onto {destination}."))
            }
            Self::JjEditRevision { label, .. } => Some(format!(
                "Set the working-copy change to {label}. jj will snapshot current work first."
            )),
            Self::JjRestoreOperation { label, .. } => Some(format!(
                "Restore the repository to operation {label}. This creates a new jj operation that undoes later repo state."
            )),
        }
    }

    pub fn requires_confirmation(&self) -> bool {
        self.confirmation_message().is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishPlan {
    pub primary: PublishAction,
    pub alternatives: Vec<PublishAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishAction {
    pub label: String,
    pub description: String,
    pub kind: PublishActionKind,
    /// Short change-id token (e.g. "strqswum") that may appear inside `label`
    /// or `description`. The UI highlights its unique prefix (bold) followed
    /// by the rest (muted), matching the status-bar identity styling.
    pub change_id_token: Option<ChangeIdToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeIdToken {
    pub text: String,
    pub prefix_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishActionKind {
    PushRef {
        remote: String,
        refspec: String,
        force_with_lease: bool,
    },
    PushChange {
        remote: String,
        revision: String,
    },
    PushBookmark {
        remote: String,
        bookmark: String,
    },
    PushTracked {
        remote: String,
    },
    MoveBookmarkAndPush {
        remote: String,
        bookmark: String,
        revision: String,
        allow_backwards: bool,
    },
    CreateBookmarkAndPush {
        remote: String,
        bookmark: String,
        revision: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishOutcome {
    pub label: String,
}

impl FileOperation {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Unstage => "unstage",
            Self::Discard => "discard",
        }
    }
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
    pub operation_log: Vec<VcsOperationLogEntry>,
    pub file_changes: Vec<FileChange>,
}
