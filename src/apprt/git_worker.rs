use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::apprt::ProgressReporter;
use crate::apprt::runtime::RuntimeEventSender;
use crate::core::compare::{ComparePhase, ProgressSink};
use crate::core::vcs::git::{
    BranchInfo, CommitInfo, GitService, PatchApplyTarget, StatusItem, StatusOperation, StatusScope,
    TagInfo, status::status_items_from_entry,
};
use crate::events::{
    RepositoryChangeKind, RepositoryEvent, RepositorySnapshot, RepositorySyncReason,
};

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
        change_hint: RepositoryChangeKind,
    },
}

#[derive(Default)]
struct GitWorkerState {
    git: GitService,
    active_path: Option<PathBuf>,
    snapshot: Option<SnapshotBundle>,
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

#[derive(Debug, Clone)]
struct SnapshotBundle {
    snapshot: RepositorySnapshot,
    state: RepositorySnapshotState,
}

fn git_worker_loop(event_sender: RuntimeEventSender, receiver: Receiver<GitWorkerCommand>) {
    let mut state = GitWorkerState::default();
    let mut pending_dirty: Option<(PathBuf, RepositoryChangeKind)> = None;

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
                sync_repository(
                    &mut state,
                    &event_sender,
                    path,
                    reason,
                    None,
                    reporter_generation,
                );
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
            Some(GitWorkerCommand::Dirty { path, change_hint }) => {
                pending_dirty = Some(match pending_dirty.take() {
                    Some((pending_path, pending_hint)) if pending_path == path => {
                        (path, merge_change_kind(pending_hint, change_hint))
                    }
                    _ => (path, change_hint),
                });
            }
            None => {
                let Some((path, change_hint)) = pending_dirty.take() else {
                    continue;
                };
                sync_repository(
                    &mut state,
                    &event_sender,
                    path,
                    RepositorySyncReason::Dirty,
                    Some(change_hint),
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
            event_sender.send(RepositoryEvent::StatusOperationFailed {
                path,
                message: error.to_string(),
            });
            return;
        }
    }

    if let Err(error) = state.git.apply_status_operation(&item, operation) {
        event_sender.send(RepositoryEvent::StatusOperationFailed {
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
            event_sender.send(RepositoryEvent::StatusOperationFailed {
                path,
                message: error.to_string(),
            });
            return;
        }
    }

    if let Err(error) = state.git.apply_batch_status_operation(&items, operation) {
        event_sender.send(RepositoryEvent::StatusOperationFailed {
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
            event_sender.send(RepositoryEvent::StatusOperationFailed {
                path,
                message: error.to_string(),
            });
            return;
        }
    }

    let location = match operation {
        StatusOperation::Discard => PatchApplyTarget::Workdir,
        _ => PatchApplyTarget::Index,
    };

    if let Err(error) = state.git.apply_patch(patch, location) {
        tracing::error!(
            ?operation,
            path = %path.display(),
            error = %error,
            patch = %patch,
            "patch apply failed"
        );
        event_sender.send(RepositoryEvent::StatusOperationFailed {
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
            event_sender.send(RepositoryEvent::CommitFailed {
                path,
                message: error.to_string(),
            });
            return;
        }
    }

    if let Err(error) = state.git.commit(message) {
        event_sender.send(RepositoryEvent::CommitFailed {
            path,
            message: error.to_string(),
        });
        return;
    }

    event_sender.send(RepositoryEvent::CommitCreated { path: path.clone() });
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
        event_sender.send(RepositoryEvent::FetchFailed {
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
            progress_sender.send(RepositoryEvent::FetchProgress {
                toast_id,
                received_objects: received,
                total_objects: total,
                received_bytes: bytes,
            });
        });

    match result {
        Ok(()) => {
            tracing::debug!(path = %path.display(), %remote, "git: fetch complete");
            event_sender.send(RepositoryEvent::FetchComplete {
                toast_id,
                path: path.clone(),
                remote,
            });
            sync_repository_forced(state, event_sender, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %remote, %error, "git: fetch failed");
            event_sender.send(RepositoryEvent::FetchFailed {
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
        event_sender.send(RepositoryEvent::PushFailed {
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
            progress_sender.send(RepositoryEvent::PushProgress {
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
            event_sender.send(RepositoryEvent::PushComplete {
                toast_id,
                path: path.clone(),
                remote,
                branch,
            });
            sync_repository_forced(state, event_sender, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %remote, %error, "git: push failed");
            event_sender.send(RepositoryEvent::PushFailed {
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
        event_sender.send(RepositoryEvent::PullFailed {
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
            progress_sender.send(RepositoryEvent::FetchProgress {
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
            event_sender.send(RepositoryEvent::PullComplete {
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
            event_sender.send(RepositoryEvent::PullFailed {
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
    sync_repository_inner(state, event_sender, path, reason, true, None, None);
}

fn sync_repository(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    reason: RepositorySyncReason,
    dirty_hint: Option<RepositoryChangeKind>,
    reporter_generation: Option<u64>,
) {
    sync_repository_inner(
        state,
        event_sender,
        path,
        reason,
        false,
        dirty_hint,
        reporter_generation,
    );
}

fn sync_repository_inner(
    state: &mut GitWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    reason: RepositorySyncReason,
    force_emit: bool,
    dirty_hint: Option<RepositoryChangeKind>,
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
            event_sender.send(RepositoryEvent::RepositorySnapshotFailed {
                path,
                reason,
                message: error.to_string(),
            });
            return;
        }
    }

    let bundle = match collect_snapshot_for_sync(
        &state.git,
        path.clone(),
        reason,
        reporter_ref,
        if force_emit { None } else { dirty_hint },
        state.snapshot.as_ref(),
    ) {
        Ok(bundle) => bundle,
        Err(error) => {
            event_sender.send(RepositoryEvent::RepositorySnapshotFailed {
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
        .and_then(|previous| previous.state.diff_kind(&bundle.state));

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

    if should_emit {
        let mut snapshot = bundle.snapshot.clone();
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
        event_sender.send(RepositoryEvent::RepositorySnapshotReady(snapshot));
    } else {
        tracing::debug!(
            path = %path.display(),
            ?reason,
            "sync_repository: suppressing snapshot (no diff detected)"
        );
    }
    state.snapshot = Some(bundle);
}

fn collect_snapshot_for_sync(
    git: &GitService,
    path: PathBuf,
    reason: RepositorySyncReason,
    reporter: Option<&dyn ProgressSink>,
    dirty_hint: Option<RepositoryChangeKind>,
    previous: Option<&SnapshotBundle>,
) -> crate::core::error::Result<SnapshotBundle> {
    let Some(previous) = previous else {
        return collect_snapshot(git, path, reason, reporter);
    };
    match (reason, dirty_hint) {
        (RepositorySyncReason::Dirty, Some(RepositoryChangeKind::Worktree)) => {
            collect_worktree_snapshot(git, path, reason, previous)
        }
        (RepositorySyncReason::Dirty, Some(RepositoryChangeKind::Git)) => {
            collect_git_snapshot(git, path, reason, reporter, previous)
        }
        _ => collect_snapshot(git, path, reason, reporter),
    }
}

fn collect_snapshot(
    git: &GitService,
    path: PathBuf,
    reason: RepositorySyncReason,
    reporter: Option<&dyn ProgressSink>,
) -> crate::core::error::Result<SnapshotBundle> {
    let _span =
        crate::core::perf::PerfSpan::new("git.snapshot", format!("reason={reason:?} mode=full"));
    let repo_path = git.repo_path().to_owned();
    let (refs, (status_items, status_entries)) = thread::scope(
        |scope| -> crate::core::error::Result<(
            GitSnapshotRefs,
            (Vec<StatusItem>, Vec<(String, u32)>),
        )> {
            let status_handle = scope.spawn(move || {
                let status_git = GitService::new_with_repo_path(repo_path);
                collect_status(&status_git)
            });
            let refs = collect_git_refs(git, reporter);
            let status = status_handle.join().unwrap_or_else(|_| {
                Err(crate::core::error::DiffyError::General(
                    "status worker panicked".to_owned(),
                ))
            });
            Ok((refs?, status?))
        },
    )?;

    let snapshot = RepositorySnapshot {
        path,
        reason,
        change_kind: None,
        branches: refs.branches,
        tags: refs.tags,
        commits: refs.commits,
        status_items,
    };
    let state = RepositorySnapshotState {
        head_oid: refs.head_oid,
        branch_targets: refs.branch_targets,
        tag_targets: refs.tag_targets,
        statuses: status_entries,
    };

    Ok(SnapshotBundle { snapshot, state })
}

fn collect_worktree_snapshot(
    git: &GitService,
    path: PathBuf,
    reason: RepositorySyncReason,
    previous: &SnapshotBundle,
) -> crate::core::error::Result<SnapshotBundle> {
    let _span = crate::core::perf::PerfSpan::new(
        "git.snapshot",
        format!("reason={reason:?} mode=worktree"),
    );
    let (status_items, status_entries) = collect_status(git)?;
    let mut snapshot = previous.snapshot.clone();
    snapshot.path = path;
    snapshot.reason = reason;
    snapshot.change_kind = None;
    snapshot.status_items = status_items;
    let mut state = previous.state.clone();
    state.statuses = status_entries;
    Ok(SnapshotBundle { snapshot, state })
}

fn collect_git_snapshot(
    git: &GitService,
    path: PathBuf,
    reason: RepositorySyncReason,
    reporter: Option<&dyn ProgressSink>,
    previous: &SnapshotBundle,
) -> crate::core::error::Result<SnapshotBundle> {
    let _span =
        crate::core::perf::PerfSpan::new("git.snapshot", format!("reason={reason:?} mode=git"));
    let refs = collect_git_refs(git, reporter)?;
    let mut snapshot = previous.snapshot.clone();
    snapshot.path = path;
    snapshot.reason = reason;
    snapshot.change_kind = None;
    snapshot.branches = refs.branches;
    snapshot.tags = refs.tags;
    snapshot.commits = refs.commits;
    let mut state = previous.state.clone();
    state.head_oid = refs.head_oid;
    state.branch_targets = refs.branch_targets;
    state.tag_targets = refs.tag_targets;
    Ok(SnapshotBundle { snapshot, state })
}

struct GitSnapshotRefs {
    branches: Vec<BranchInfo>,
    tags: Vec<TagInfo>,
    commits: Vec<CommitInfo>,
    head_oid: Option<String>,
    branch_targets: Vec<BranchTarget>,
    tag_targets: Vec<(String, String)>,
}

fn collect_git_refs(
    git: &GitService,
    reporter: Option<&dyn ProgressSink>,
) -> crate::core::error::Result<GitSnapshotRefs> {
    // Phase boundaries roughly track the I/O cost: branch/tag enumeration
    // first, then commit walk. The repository open itself was already reported.
    if let Some(r) = reporter {
        r.phase(ComparePhase::ResolvingRefs);
    }
    let branches = git.branches()?;
    let tags = git.tags()?;

    if let Some(r) = reporter {
        r.phase(ComparePhase::FetchingHistory);
    }
    let commits = git.commits("HEAD", 200).unwrap_or_default();
    let mut branch_targets = branches
        .iter()
        .map(|branch| BranchTarget {
            name: branch.name.clone(),
            is_remote: branch.is_remote,
            is_head: branch.is_head,
            target_oid: (!branch.target_oid.is_empty()).then(|| branch.target_oid.clone()),
        })
        .collect::<Vec<_>>();
    branch_targets.sort();
    let mut tag_targets = tags
        .iter()
        .map(|tag| (tag.name.clone(), tag.target_oid.clone()))
        .collect::<Vec<_>>();
    tag_targets.sort();

    Ok(GitSnapshotRefs {
        branches,
        tags,
        commits,
        head_oid: git.resolve_ref("HEAD").ok(),
        branch_targets,
        tag_targets,
    })
}

fn collect_status(
    git: &GitService,
) -> crate::core::error::Result<(Vec<StatusItem>, Vec<(String, u32)>)> {
    let statuses = git.status_entries()?;
    let mut status_entries = statuses
        .iter()
        .map(|(path, status)| (path.clone(), sanitize_status(*status).bits()))
        .collect::<Vec<_>>();
    status_entries.sort();
    let mut status_items = statuses
        .iter()
        .flat_map(|(path, status)| status_items_from_entry(path.clone(), sanitize_status(*status)))
        .collect::<Vec<_>>();
    status_items.sort_by(|left, right| {
        left.scope
            .label()
            .cmp(right.scope.label())
            .then(left.path.cmp(&right.path))
    });
    Ok((status_items, status_entries))
}

fn merge_change_kind(
    left: RepositoryChangeKind,
    right: RepositoryChangeKind,
) -> RepositoryChangeKind {
    match (left, right) {
        (RepositoryChangeKind::Both, _) | (_, RepositoryChangeKind::Both) => {
            RepositoryChangeKind::Both
        }
        (RepositoryChangeKind::Git, RepositoryChangeKind::Worktree)
        | (RepositoryChangeKind::Worktree, RepositoryChangeKind::Git) => RepositoryChangeKind::Both,
        (RepositoryChangeKind::Git, RepositoryChangeKind::Git) => RepositoryChangeKind::Git,
        (RepositoryChangeKind::Worktree, RepositoryChangeKind::Worktree) => {
            RepositoryChangeKind::Worktree
        }
    }
}

fn sanitize_status(
    status: crate::core::vcs::git::status::StatusBits,
) -> crate::core::vcs::git::status::StatusBits {
    status
        & (crate::core::vcs::git::status::StatusBits::INDEX_NEW
            | crate::core::vcs::git::status::StatusBits::INDEX_MODIFIED
            | crate::core::vcs::git::status::StatusBits::INDEX_DELETED
            | crate::core::vcs::git::status::StatusBits::INDEX_RENAMED
            | crate::core::vcs::git::status::StatusBits::INDEX_TYPECHANGE
            | crate::core::vcs::git::status::StatusBits::WT_NEW
            | crate::core::vcs::git::status::StatusBits::WT_MODIFIED
            | crate::core::vcs::git::status::StatusBits::WT_DELETED
            | crate::core::vcs::git::status::StatusBits::WT_TYPECHANGE
            | crate::core::vcs::git::status::StatusBits::WT_RENAMED
            | crate::core::vcs::git::status::StatusBits::CONFLICTED)
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
