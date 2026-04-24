use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use git2::{BranchType, Status, StatusOptions};

use crate::apprt::ProgressReporter;
use crate::apprt::runtime::RuntimeEventSender;
use crate::core::compare::{ComparePhase, ProgressSink};
use crate::core::vcs::git::{
    GitService, StatusItem, StatusOperation, StatusScope, status::status_items_from_entry,
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

    pub fn dispatch_sync(
        &self,
        path: PathBuf,
        reason: RepositorySyncReason,
        reporter_generation: Option<u64>,
    ) {
        let _ = self.sender.send(GitWorkerCommand::Sync {
            path,
            reason,
            reporter_generation,
        });
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

    pub fn dispatch_patch_operation(
        &self,
        path: PathBuf,
        patch: String,
        scope: StatusScope,
        operation: StatusOperation,
    ) {
        let _ = self.sender.send(GitWorkerCommand::ApplyPatch {
            path,
            patch,
            scope,
            operation,
        });
    }

    pub fn dispatch_commit(&self, path: PathBuf, message: String) {
        let _ = self.sender.send(GitWorkerCommand::Commit { path, message });
    }

    pub fn dispatch_fetch(&self, path: PathBuf, remote: String, toast_id: u64) {
        let _ = self.sender.send(GitWorkerCommand::Fetch {
            path,
            remote,
            toast_id,
        });
    }

    pub fn dispatch_push(
        &self,
        path: PathBuf,
        remote: String,
        refspec: String,
        force_with_lease: bool,
        toast_id: u64,
    ) {
        let _ = self.sender.send(GitWorkerCommand::Push {
            path,
            remote,
            refspec,
            force_with_lease,
            toast_id,
        });
    }

    pub fn dispatch_pull_ff(&self, path: PathBuf, remote: String, branch: String, toast_id: u64) {
        let _ = self.sender.send(GitWorkerCommand::PullFf {
            path,
            remote,
            branch,
            toast_id,
        });
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
        /// When `Some`, the worker emits `CompareProgressUpdate` events
        /// tagged with this generation so the loading panel follows the
        /// repo-open phases. `None` for background resyncs.
        reporter_generation: Option<u64>,
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
    ApplyPatch {
        path: PathBuf,
        patch: String,
        scope: StatusScope,
        operation: StatusOperation,
    },
    Commit {
        path: PathBuf,
        message: String,
    },
    Fetch {
        path: PathBuf,
        remote: String,
        toast_id: u64,
    },
    Push {
        path: PathBuf,
        remote: String,
        refspec: String,
        force_with_lease: bool,
        toast_id: u64,
    },
    PullFf {
        path: PathBuf,
        remote: String,
        branch: String,
        toast_id: u64,
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
            Some(GitWorkerCommand::Sync {
                path,
                reason,
                reporter_generation,
            }) => {
                pending_dirty = None;
                sync_repository(&mut state, &event_sender, path, reason, reporter_generation);
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
            Some(GitWorkerCommand::ApplyPatch {
                path,
                patch,
                scope,
                operation,
            }) => {
                pending_dirty = None;
                apply_patch_operation(&mut state, &event_sender, path, &patch, scope, operation);
            }
            Some(GitWorkerCommand::Commit { path, message }) => {
                pending_dirty = None;
                apply_commit(&mut state, &event_sender, path, &message);
            }
            Some(GitWorkerCommand::Fetch {
                path,
                remote,
                toast_id,
            }) => {
                pending_dirty = None;
                apply_fetch(&mut state, &event_sender, path, remote, toast_id);
            }
            Some(GitWorkerCommand::Push {
                path,
                remote,
                refspec,
                force_with_lease,
                toast_id,
            }) => {
                pending_dirty = None;
                apply_push(
                    &mut state,
                    &event_sender,
                    path,
                    remote,
                    refspec,
                    force_with_lease,
                    toast_id,
                );
            }
            Some(GitWorkerCommand::PullFf {
                path,
                remote,
                branch,
                toast_id,
            }) => {
                pending_dirty = None;
                apply_pull_ff(&mut state, &event_sender, path, remote, branch, toast_id);
            }
            Some(GitWorkerCommand::Dirty { path }) => {
                pending_dirty = Some(path);
            }
            None => {
                let Some(path) = pending_dirty.take() else {
                    continue;
                };
                sync_repository(
                    &mut state,
                    &event_sender,
                    path,
                    RepositorySyncReason::Dirty,
                    None,
                );
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

    sync_repository_forced(state, event_sender, path, RepositorySyncReason::Dirty);
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

    sync_repository_forced(state, event_sender, path, RepositorySyncReason::Dirty);
}

fn apply_patch_operation(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    patch: &str,
    _scope: StatusScope,
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

    let location = match operation {
        StatusOperation::Discard => git2::ApplyLocation::WorkDir,
        _ => git2::ApplyLocation::Index,
    };

    if let Err(error) = state.git.apply_patch(patch, location) {
        tracing::error!(
            ?operation,
            path = %path.display(),
            error = %error,
            patch = %patch,
            "patch apply failed"
        );
        event_sender.send(AppEvent::StatusOperationFailed {
            path: path.clone(),
            message: format!("{} failed: {}", operation.label(), error),
        });
        return;
    }

    sync_repository_forced(state, event_sender, path, RepositorySyncReason::Dirty);
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
    sync_repository_forced(state, event_sender, path, RepositorySyncReason::Dirty);
}

fn ensure_open(state: &mut GitWorkerState, path: &PathBuf) -> std::result::Result<(), String> {
    if state.active_path.as_ref() != Some(path) {
        state.git.close();
        state.snapshot = None;
        state.active_path = Some(path.clone());
    }
    if !state.git.is_open() {
        state
            .git
            .open(path.to_string_lossy().as_ref())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn apply_fetch(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    remote: String,
    toast_id: u64,
) {
    tracing::debug!(path = %path.display(), %remote, toast_id, "git: fetch requested");
    if let Err(message) = ensure_open(state, &path) {
        tracing::warn!(path = %path.display(), %message, "git: fetch ensure_open failed");
        event_sender.send(AppEvent::FetchFailed {
            toast_id,
            remote,
            message,
        });
        return;
    }

    let progress_sender = event_sender.clone();
    let result = state
        .git
        .fetch_remote(&remote, move |received, total, bytes| {
            progress_sender.send(AppEvent::FetchProgress {
                toast_id,
                received_objects: received,
                total_objects: total,
                received_bytes: bytes,
            });
        });

    match result {
        Ok(()) => {
            tracing::debug!(path = %path.display(), %remote, "git: fetch complete");
            event_sender.send(AppEvent::FetchComplete {
                toast_id,
                path: path.clone(),
                remote,
            });
            sync_repository_forced(state, event_sender, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %remote, %error, "git: fetch failed");
            event_sender.send(AppEvent::FetchFailed {
                toast_id,
                remote,
                message: error.to_string(),
            });
        }
    }
}

fn apply_push(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    remote: String,
    refspec: String,
    force_with_lease: bool,
    toast_id: u64,
) {
    tracing::debug!(
        path = %path.display(),
        %remote,
        %refspec,
        force_with_lease,
        toast_id,
        "git: push requested",
    );
    if let Err(message) = ensure_open(state, &path) {
        tracing::warn!(path = %path.display(), %message, "git: push ensure_open failed");
        event_sender.send(AppEvent::PushFailed {
            toast_id,
            remote,
            message,
        });
        return;
    }

    // Parse the branch out of the refspec for the completion event, e.g.
    // `refs/heads/foo:refs/heads/foo` → `foo`.
    let branch = refspec
        .rsplit(':')
        .next()
        .and_then(|dst| dst.rsplit('/').next())
        .unwrap_or("")
        .to_owned();

    let progress_sender = event_sender.clone();
    let result = state.git.push(
        &remote,
        &refspec,
        force_with_lease,
        move |current, total, bytes| {
            progress_sender.send(AppEvent::PushProgress {
                toast_id,
                current,
                total,
                bytes,
            });
        },
    );

    match result {
        Ok(()) => {
            tracing::debug!(path = %path.display(), %remote, %branch, "git: push complete");
            event_sender.send(AppEvent::PushComplete {
                toast_id,
                path: path.clone(),
                remote,
                branch,
            });
            sync_repository_forced(state, event_sender, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %remote, %error, "git: push failed");
            event_sender.send(AppEvent::PushFailed {
                toast_id,
                remote,
                message: error.to_string(),
            });
        }
    }
}

fn apply_pull_ff(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    remote: String,
    branch: String,
    toast_id: u64,
) {
    tracing::debug!(
        path = %path.display(),
        %remote,
        %branch,
        toast_id,
        "git: pull-ff requested",
    );
    if let Err(message) = ensure_open(state, &path) {
        tracing::warn!(path = %path.display(), %message, "git: pull-ff ensure_open failed");
        event_sender.send(AppEvent::PullFailed {
            toast_id,
            remote,
            branch,
            message,
        });
        return;
    }

    let progress_sender = event_sender.clone();
    let result = state
        .git
        .pull_ff(&remote, &branch, move |received, total, bytes| {
            progress_sender.send(AppEvent::FetchProgress {
                toast_id,
                received_objects: received,
                total_objects: total,
                received_bytes: bytes,
            });
        });

    match result {
        Ok(outcome) => {
            let (already_up_to_date, behind) = match outcome {
                crate::core::vcs::git::PullOutcome::AlreadyUpToDate => (true, 0),
                crate::core::vcs::git::PullOutcome::FastForwarded { behind } => (false, behind),
            };
            tracing::debug!(
                path = %path.display(),
                %remote,
                %branch,
                already_up_to_date,
                behind,
                "git: pull-ff complete",
            );
            event_sender.send(AppEvent::PullComplete {
                toast_id,
                path: path.clone(),
                remote,
                branch,
                already_up_to_date,
                behind,
            });
            sync_repository_forced(state, event_sender, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %remote,
                %branch,
                %error,
                "git: pull-ff failed",
            );
            event_sender.send(AppEvent::PullFailed {
                toast_id,
                remote,
                branch,
                message: error.to_string(),
            });
        }
    }
}

fn sync_repository_forced(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    reason: RepositorySyncReason,
) {
    sync_repository_inner(state, event_sender, path, reason, true, None);
}

fn sync_repository(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    reason: RepositorySyncReason,
    reporter_generation: Option<u64>,
) {
    sync_repository_inner(
        state,
        event_sender,
        path,
        reason,
        false,
        reporter_generation,
    );
}

fn sync_repository_inner(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    reason: RepositorySyncReason,
    force_emit: bool,
    reporter_generation: Option<u64>,
) {
    let reporter = reporter_generation
        .map(|generation| ProgressReporter::new(generation, event_sender.clone()));
    let reporter_ref: Option<&dyn ProgressSink> = reporter.as_ref().map(|r| r as &dyn ProgressSink);

    if let Some(r) = reporter_ref {
        r.phase(ComparePhase::OpeningRepo);
    }
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

    let bundle = match collect_snapshot(&state.git, path.clone(), reason, reporter_ref) {
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

    let should_emit = force_emit || reason == RepositorySyncReason::Open || change_kind.is_some();
    tracing::debug!(
        path = %path.display(),
        ?reason,
        ?change_kind,
        force_emit,
        should_emit,
        prev_had_snapshot = state.snapshot.is_some(),
        "sync_repository: computed"
    );
    state.snapshot = Some(bundle.state);

    if should_emit {
        let mut snapshot = bundle.snapshot;
        snapshot.change_kind = if reason == RepositorySyncReason::Open {
            None
        } else {
            // Force-emitted user operations may have no detected diff (same
            // coarse status bits) but still changed the index content, so
            // report Worktree to drive the UI refresh downstream.
            change_kind.or(if force_emit {
                Some(RepositoryChangeKind::Worktree)
            } else {
                None
            })
        };
        event_sender.send(AppEvent::RepositorySnapshotReady(snapshot));
    } else {
        tracing::debug!(
            path = %path.display(),
            ?reason,
            "sync_repository: suppressing snapshot (no diff detected)"
        );
    }
}

fn collect_snapshot(
    git: &GitService,
    path: PathBuf,
    reason: RepositorySyncReason,
    reporter: Option<&dyn ProgressSink>,
) -> crate::core::error::Result<SnapshotBundle> {
    // Phase boundaries roughly track the I/O cost: branch/tag enumeration
    // first, then commit walk, then status. The repository open itself
    // was already reported before we got here.
    if let Some(r) = reporter {
        r.phase(ComparePhase::ResolvingRefs);
    }
    let branches = git.branches()?;
    let tags = git.tags()?;

    if let Some(r) = reporter {
        r.phase(ComparePhase::FetchingHistory);
    }
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
        collect_snapshot(&git, path.to_path_buf(), RepositorySyncReason::Open, None)
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
