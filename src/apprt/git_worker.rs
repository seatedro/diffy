use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use git2::{BranchType, Status, StatusOptions};

use crate::apprt::runtime::RuntimeEventSender;
use crate::core::vcs::git::{
    GitService, StatusItem, StatusOperation, status::status_items_from_entry,
};
use crate::events::{AppEvent, RepositoryChangeKind, RepositorySnapshot, RepositorySyncReason};

const GIT_DIRTY_DEBOUNCE: Duration = Duration::from_millis(150);

pub struct GitWorker {
    sender: Sender<GitWorkerCommand>,
}

impl GitWorker {
    pub fn new(event_sender: RuntimeEventSender) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || git_worker_loop(event_sender, receiver));
        Self { sender }
    }

    pub fn dispatch_sync(&self, path: PathBuf, reason: RepositorySyncReason) {
        let _ = self.sender.send(GitWorkerCommand::Sync { path, reason });
    }

    pub fn dispatch_operation(&self, path: PathBuf, item: StatusItem, operation: StatusOperation) {
        let _ = self.sender.send(GitWorkerCommand::ApplyOperation {
            path,
            item,
            operation,
        });
    }

    pub fn dispatch_batch_operation(
        &self,
        path: PathBuf,
        items: Vec<StatusItem>,
        operation: StatusOperation,
    ) {
        let _ = self.sender.send(GitWorkerCommand::ApplyBatchOperation {
            path,
            items,
            operation,
        });
    }

    pub fn dispatch_commit(&self, path: PathBuf, message: String) {
        let _ = self.sender.send(GitWorkerCommand::Commit { path, message });
    }

    pub(crate) fn sender(&self) -> Sender<GitWorkerCommand> {
        self.sender.clone()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum GitWorkerCommand {
    Sync {
        path: PathBuf,
        reason: RepositorySyncReason,
    },
    ApplyOperation {
        path: PathBuf,
        item: StatusItem,
        operation: StatusOperation,
    },
    ApplyBatchOperation {
        path: PathBuf,
        items: Vec<StatusItem>,
        operation: StatusOperation,
    },
    Commit {
        path: PathBuf,
        message: String,
    },
    Dirty {
        path: PathBuf,
    },
}

#[derive(Default)]
struct GitWorkerState {
    git: GitService,
    active_path: Option<PathBuf>,
    snapshot: Option<RepositorySnapshotState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepositorySnapshotState {
    head_oid: Option<String>,
    branch_targets: Vec<BranchTarget>,
    tag_targets: Vec<(String, String)>,
    statuses: Vec<(String, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BranchTarget {
    name: String,
    is_remote: bool,
    is_head: bool,
    target_oid: Option<String>,
}

struct SnapshotBundle {
    snapshot: RepositorySnapshot,
    state: RepositorySnapshotState,
}

fn git_worker_loop(event_sender: RuntimeEventSender, receiver: Receiver<GitWorkerCommand>) {
    let mut state = GitWorkerState::default();
    let mut pending_dirty: Option<PathBuf> = None;

    loop {
        let command = if pending_dirty.is_some() {
            match receiver.recv_timeout(GIT_DIRTY_DEBOUNCE) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match receiver.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            }
        };

        match command {
            Some(GitWorkerCommand::Sync { path, reason }) => {
                pending_dirty = None;
                sync_repository(&mut state, &event_sender, path, reason);
            }
            Some(GitWorkerCommand::ApplyOperation {
                path,
                item,
                operation,
            }) => {
                pending_dirty = None;
                apply_status_operation(&mut state, &event_sender, path, item, operation);
            }
            Some(GitWorkerCommand::ApplyBatchOperation {
                path,
                items,
                operation,
            }) => {
                pending_dirty = None;
                apply_batch_status_operation(&mut state, &event_sender, path, items, operation);
            }
            Some(GitWorkerCommand::Commit { path, message }) => {
                pending_dirty = None;
                apply_commit(&mut state, &event_sender, path, &message);
            }
            Some(GitWorkerCommand::Dirty { path }) => {
                pending_dirty = Some(path);
            }
            None => {
                let Some(path) = pending_dirty.take() else {
                    continue;
                };
                sync_repository(&mut state, &event_sender, path, RepositorySyncReason::Dirty);
            }
        }
    }
}

fn apply_status_operation(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    item: StatusItem,
    operation: StatusOperation,
) {
    if state.active_path.as_ref() != Some(&path) {
        state.git.close();
        state.snapshot = None;
        state.active_path = Some(path.clone());
    }

    if !state.git.is_open() {
        if let Err(error) = state.git.open(path.to_string_lossy().as_ref()) {
            event_sender.send(AppEvent::StatusOperationFailed {
                path,
                message: error.to_string(),
            });
            return;
        }
    }

    if let Err(error) = state.git.apply_status_operation(&item, operation) {
        event_sender.send(AppEvent::StatusOperationFailed {
            path: path.clone(),
            message: error.to_string(),
        });
        return;
    }

    sync_repository(state, event_sender, path, RepositorySyncReason::Dirty);
}

fn apply_batch_status_operation(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    items: Vec<StatusItem>,
    operation: StatusOperation,
) {
    if state.active_path.as_ref() != Some(&path) {
        state.git.close();
        state.snapshot = None;
        state.active_path = Some(path.clone());
    }

    if !state.git.is_open() {
        if let Err(error) = state.git.open(path.to_string_lossy().as_ref()) {
            event_sender.send(AppEvent::StatusOperationFailed {
                path,
                message: error.to_string(),
            });
            return;
        }
    }

    if let Err(error) = state.git.apply_batch_status_operation(&items, operation) {
        event_sender.send(AppEvent::StatusOperationFailed {
            path: path.clone(),
            message: error.to_string(),
        });
        return;
    }

    sync_repository(state, event_sender, path, RepositorySyncReason::Dirty);
}

fn apply_commit(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    message: &str,
) {
    if state.active_path.as_ref() != Some(&path) {
        state.git.close();
        state.snapshot = None;
        state.active_path = Some(path.clone());
    }

    if !state.git.is_open() {
        if let Err(error) = state.git.open(path.to_string_lossy().as_ref()) {
            event_sender.send(AppEvent::CommitFailed {
                path,
                message: error.to_string(),
            });
            return;
        }
    }

    if let Err(error) = state.git.commit(message) {
        event_sender.send(AppEvent::CommitFailed {
            path,
            message: error.to_string(),
        });
        return;
    }

    event_sender.send(AppEvent::CommitCreated { path: path.clone() });
    sync_repository(state, event_sender, path, RepositorySyncReason::Dirty);
}

fn sync_repository(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    reason: RepositorySyncReason,
) {
    if state.active_path.as_ref() != Some(&path) {
        state.git.close();
        state.snapshot = None;
        state.active_path = Some(path.clone());
    }

    if !state.git.is_open() {
        if let Err(error) = state.git.open(path.to_string_lossy().as_ref()) {
            event_sender.send(AppEvent::RepositorySnapshotFailed {
                path,
                reason,
                message: error.to_string(),
            });
            return;
        }
    }

    let bundle = match collect_snapshot(&state.git, path.clone(), reason) {
        Ok(bundle) => bundle,
        Err(error) => {
            event_sender.send(AppEvent::RepositorySnapshotFailed {
                path,
                reason,
                message: error.to_string(),
            });
            return;
        }
    };

    let change_kind = state
        .snapshot
        .as_ref()
        .and_then(|previous| previous.diff_kind(&bundle.state));

    let should_emit = reason == RepositorySyncReason::Open || change_kind.is_some();
    state.snapshot = Some(bundle.state);

    if should_emit {
        let mut snapshot = bundle.snapshot;
        snapshot.change_kind = if reason == RepositorySyncReason::Open {
            None
        } else {
            change_kind
        };
        event_sender.send(AppEvent::RepositorySnapshotReady(snapshot));
    }
}

fn collect_snapshot(
    git: &GitService,
    path: PathBuf,
    reason: RepositorySyncReason,
) -> crate::core::error::Result<SnapshotBundle> {
    let branches = git.branches()?;
    let tags = git.tags()?;
    let commits = git.commits("HEAD", 200).unwrap_or_default();
    let repo = git.repo()?;

    let mut branch_targets = repo
        .branches(None)?
        .flatten()
        .filter_map(|(branch, branch_type)| {
            let name = branch.name().ok().flatten()?.to_owned();
            let target_oid = branch
                .get()
                .resolve()
                .ok()
                .and_then(|reference| reference.target())
                .map(|oid| oid.to_string());
            Some(BranchTarget {
                name,
                is_remote: branch_type == BranchType::Remote,
                is_head: branch.is_head(),
                target_oid,
            })
        })
        .collect::<Vec<_>>();
    branch_targets.sort();

    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);

    let statuses = repo.statuses(Some(&mut options))?;
    let mut status_entries = statuses
        .iter()
        .map(|entry| {
            (
                entry.path().unwrap_or_default().to_owned(),
                sanitize_status(entry.status()).bits(),
            )
        })
        .collect::<Vec<_>>();
    status_entries.sort();
    let mut status_items = statuses
        .iter()
        .flat_map(|entry| {
            status_items_from_entry(
                entry.path().unwrap_or_default().to_owned(),
                sanitize_status(entry.status()),
            )
        })
        .collect::<Vec<_>>();
    status_items.sort_by(|left, right| {
        left.scope
            .label()
            .cmp(right.scope.label())
            .then(left.path.cmp(&right.path))
    });

    let snapshot = RepositorySnapshot {
        path,
        reason,
        change_kind: None,
        branches: branches.clone(),
        tags: tags.clone(),
        commits: commits.clone(),
        status_items,
    };
    let state = RepositorySnapshotState {
        head_oid: repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| oid.to_string()),
        branch_targets,
        tag_targets: tags
            .iter()
            .map(|tag| (tag.name.clone(), tag.target_oid.clone()))
            .collect(),
        statuses: status_entries,
    };

    Ok(SnapshotBundle { snapshot, state })
}

fn sanitize_status(status: Status) -> Status {
    status
        & (Status::INDEX_NEW
            | Status::INDEX_MODIFIED
            | Status::INDEX_DELETED
            | Status::INDEX_RENAMED
            | Status::INDEX_TYPECHANGE
            | Status::WT_NEW
            | Status::WT_MODIFIED
            | Status::WT_DELETED
            | Status::WT_TYPECHANGE
            | Status::WT_RENAMED
            | Status::CONFLICTED)
}

impl RepositorySnapshotState {
    fn diff_kind(&self, next: &Self) -> Option<RepositoryChangeKind> {
        let worktree_changed = self.statuses != next.statuses;
        let git_changed = self.head_oid != next.head_oid
            || self.branch_targets != next.branch_targets
            || self.tag_targets != next.tag_targets;

        match (git_changed, worktree_changed) {
            (false, false) => None,
            (true, false) => Some(RepositoryChangeKind::Git),
            (false, true) => Some(RepositoryChangeKind::Worktree),
            (true, true) => Some(RepositoryChangeKind::Both),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Repository, Signature};
    use tempfile::TempDir;

    use super::{RepositorySnapshotState, collect_snapshot};
    use crate::core::vcs::git::GitService;
    use crate::events::{RepositoryChangeKind, RepositorySyncReason};

    fn commit_file(repo: &Repository, relative_path: &str, content: &str, message: &str) -> String {
        let workdir = repo.workdir().expect("repo workdir");
        let full_path = workdir.join(relative_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full_path, content).unwrap();

        let mut index = repo.index().unwrap();
        index.add_path(Path::new(relative_path)).unwrap();
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = Signature::now("Diffy", "diffy@example.com").unwrap();
        let parents = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| repo.find_commit(oid).unwrap())
            .into_iter()
            .collect::<Vec<_>>();
        let parent_refs = parents.iter().collect::<Vec<_>>();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parent_refs,
        )
        .unwrap()
        .to_string()
    }

    fn load_state(path: &Path) -> RepositorySnapshotState {
        let mut git = GitService::new();
        git.open(path.to_string_lossy().as_ref()).unwrap();
        collect_snapshot(&git, path.to_path_buf(), RepositorySyncReason::Open)
            .unwrap()
            .state
    }

    #[test]
    fn detects_worktree_only_changes() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", "old\n", "initial");
        let before = load_state(repo_dir.path());
        fs::write(repo_dir.path().join("src/lib.rs"), "new\n").unwrap();
        let after = load_state(repo_dir.path());

        assert_eq!(
            before.diff_kind(&after),
            Some(RepositoryChangeKind::Worktree)
        );
    }

    #[test]
    fn detects_git_only_changes() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", "old\n", "initial");
        let before = load_state(repo_dir.path());
        commit_file(&repo, "src/lib.rs", "new\n", "second");
        let after = load_state(repo_dir.path());

        assert_eq!(before.diff_kind(&after), Some(RepositoryChangeKind::Git));
    }
}
