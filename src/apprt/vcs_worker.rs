use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::apprt::ProgressReporter;
use crate::apprt::runtime::RuntimeEventSender;
use crate::core::compare::{ComparePhase, ProgressSink};
use crate::core::vcs::{
    backend::VcsRepository,
    discovery,
    model::{
        ChangeBucket, FileChange, FileOperation, PublishAction, PullFastForwardOutcome,
        RepoLocation, VcsOperation, VcsSnapshot,
    },
};
use crate::events::{
    RepositoryChangeKind, RepositoryEvent, RepositorySnapshot, RepositorySyncReason,
};

const VCS_DIRTY_DEBOUNCE: Duration = Duration::from_millis(150);

pub struct VcsWorker {
    sender: Sender<VcsWorkerCommand>,
}

impl VcsWorker {
    pub fn new(event_sender: RuntimeEventSender) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || vcs_worker_loop(event_sender, receiver));
        Self { sender }
    }

    pub fn dispatch_sync(
        &self,
        path: PathBuf,
        reason: RepositorySyncReason,
        reporter_generation: Option<u64>,
    ) {
        let _ = self.sender.send(VcsWorkerCommand::Sync {
            path,
            reason,
            reporter_generation,
        });
    }

    pub fn dispatch_operation(
        &self,
        path: PathBuf,
        file_change: FileChange,
        operation: FileOperation,
    ) {
        let _ = self.sender.send(VcsWorkerCommand::ApplyOperation {
            path,
            file_change,
            operation,
        });
    }

    pub fn dispatch_batch_operation(
        &self,
        path: PathBuf,
        file_changes: Vec<FileChange>,
        operation: FileOperation,
    ) {
        let _ = self.sender.send(VcsWorkerCommand::ApplyBatchOperation {
            path,
            file_changes,
            operation,
        });
    }

    pub fn dispatch_patch_operation(
        &self,
        path: PathBuf,
        patch: String,
        bucket: ChangeBucket,
        operation: FileOperation,
    ) {
        let _ = self.sender.send(VcsWorkerCommand::ApplyPatch {
            path,
            patch,
            bucket,
            operation,
        });
    }

    pub fn dispatch_commit(&self, path: PathBuf, message: String) {
        let _ = self.sender.send(VcsWorkerCommand::Commit { path, message });
    }

    pub fn dispatch_operation_command(
        &self,
        path: PathBuf,
        operation: VcsOperation,
        toast_id: u64,
    ) {
        let _ = self.sender.send(VcsWorkerCommand::RunOperation {
            path,
            operation,
            toast_id,
        });
    }

    pub fn dispatch_fetch(&self, path: PathBuf, remote: String, toast_id: u64) {
        let _ = self.sender.send(VcsWorkerCommand::Fetch {
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
        let _ = self.sender.send(VcsWorkerCommand::Push {
            path,
            remote,
            refspec,
            force_with_lease,
            toast_id,
        });
    }

    pub fn dispatch_publish(&self, path: PathBuf, action: Option<PublishAction>, toast_id: u64) {
        let _ = self.sender.send(VcsWorkerCommand::Publish {
            path,
            action,
            toast_id,
        });
    }

    pub fn dispatch_publish_plan(&self, path: PathBuf, toast_id: Option<u64>) {
        let _ = self
            .sender
            .send(VcsWorkerCommand::PublishPlan { path, toast_id });
    }

    pub fn dispatch_pull_ff(&self, path: PathBuf, remote: String, branch: String, toast_id: u64) {
        let _ = self.sender.send(VcsWorkerCommand::PullFf {
            path,
            remote,
            branch,
            toast_id,
        });
    }

    pub(crate) fn sender(&self) -> Sender<VcsWorkerCommand> {
        self.sender.clone()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum VcsWorkerCommand {
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
        file_change: FileChange,
        operation: FileOperation,
    },
    ApplyBatchOperation {
        path: PathBuf,
        file_changes: Vec<FileChange>,
        operation: FileOperation,
    },
    ApplyPatch {
        path: PathBuf,
        patch: String,
        bucket: ChangeBucket,
        operation: FileOperation,
    },
    Commit {
        path: PathBuf,
        message: String,
    },
    RunOperation {
        path: PathBuf,
        operation: VcsOperation,
        toast_id: u64,
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
    Publish {
        path: PathBuf,
        action: Option<PublishAction>,
        toast_id: u64,
    },
    PublishPlan {
        path: PathBuf,
        toast_id: Option<u64>,
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
struct VcsWorkerState {
    vcs_repo: Option<Box<dyn VcsRepository>>,
    active_path: Option<PathBuf>,
    last_snapshot: Option<VcsSnapshot>,
}

fn vcs_worker_loop(event_sender: RuntimeEventSender, receiver: Receiver<VcsWorkerCommand>) {
    let mut state = VcsWorkerState::default();
    let mut pending_dirty: Option<(PathBuf, RepositoryChangeKind)> = None;

    loop {
        let command = if pending_dirty.is_some() {
            match receiver.recv_timeout(VCS_DIRTY_DEBOUNCE) {
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
            Some(VcsWorkerCommand::Sync {
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
            Some(VcsWorkerCommand::ApplyOperation {
                path,
                file_change,
                operation,
            }) => {
                pending_dirty = None;
                apply_status_operation(&mut state, &event_sender, path, file_change, operation);
            }
            Some(VcsWorkerCommand::ApplyBatchOperation {
                path,
                file_changes,
                operation,
            }) => {
                pending_dirty = None;
                apply_batch_status_operation(
                    &mut state,
                    &event_sender,
                    path,
                    file_changes,
                    operation,
                );
            }
            Some(VcsWorkerCommand::ApplyPatch {
                path,
                patch,
                bucket,
                operation,
            }) => {
                pending_dirty = None;
                apply_patch_operation(&mut state, &event_sender, path, &patch, bucket, operation);
            }
            Some(VcsWorkerCommand::Commit { path, message }) => {
                pending_dirty = None;
                apply_commit(&mut state, &event_sender, path, &message);
            }
            Some(VcsWorkerCommand::RunOperation {
                path,
                operation,
                toast_id,
            }) => {
                pending_dirty = None;
                apply_vcs_operation(&mut state, &event_sender, path, operation, toast_id);
            }
            Some(VcsWorkerCommand::Fetch {
                path,
                remote,
                toast_id,
            }) => {
                pending_dirty = None;
                apply_fetch(&mut state, &event_sender, path, remote, toast_id);
            }
            Some(VcsWorkerCommand::Push {
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
            Some(VcsWorkerCommand::Publish {
                path,
                action,
                toast_id,
            }) => {
                pending_dirty = None;
                apply_publish(&mut state, &event_sender, path, action, toast_id);
            }
            Some(VcsWorkerCommand::PublishPlan { path, toast_id }) => {
                pending_dirty = None;
                apply_publish_plan(&event_sender, path, toast_id);
            }
            Some(VcsWorkerCommand::PullFf {
                path,
                remote,
                branch,
                toast_id,
            }) => {
                pending_dirty = None;
                apply_pull_ff(&mut state, &event_sender, path, remote, branch, toast_id);
            }
            Some(VcsWorkerCommand::Dirty { path, change_hint }) => {
                pending_dirty = Some(match pending_dirty.take() {
                    Some((pending_path, pending_hint)) if pending_path == path => {
                        (path, pending_hint.merge(change_hint))
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
    state: &mut VcsWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    file_change: FileChange,
    operation: FileOperation,
) {
    let result = discovery::open_repository(&path)
        .and_then(|mut repo| repo.apply_file_operation(&file_change, operation));
    if let Err(error) = result {
        event_sender.send(RepositoryEvent::FileOperationFailed {
            path: path.clone(),
            message: error.to_string(),
        });
        return;
    }

    sync_repository_forced(state, event_sender, path, RepositorySyncReason::Dirty);
}

fn apply_batch_status_operation(
    state: &mut VcsWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    file_changes: Vec<FileChange>,
    operation: FileOperation,
) {
    let result = discovery::open_repository(&path)
        .and_then(|mut repo| repo.apply_batch_file_operation(&file_changes, operation));
    if let Err(error) = result {
        event_sender.send(RepositoryEvent::FileOperationFailed {
            path: path.clone(),
            message: error.to_string(),
        });
        return;
    }

    sync_repository_forced(state, event_sender, path, RepositorySyncReason::Dirty);
}

fn apply_patch_operation(
    state: &mut VcsWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    patch: &str,
    _bucket: ChangeBucket,
    operation: FileOperation,
) {
    if let Err(error) = discovery::open_repository(&path)
        .and_then(|mut repo| repo.apply_patch_operation(patch, operation))
    {
        tracing::error!(
            ?operation,
            path = %path.display(),
            error = %error,
            patch = %patch,
            "patch apply failed"
        );
        event_sender.send(RepositoryEvent::FileOperationFailed {
            path: path.clone(),
            message: format!("{} failed: {}", operation.label(), error),
        });
        return;
    }

    sync_repository_forced(state, event_sender, path, RepositorySyncReason::Dirty);
}

fn apply_commit(
    state: &mut VcsWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    message: &str,
) {
    if let Err(error) =
        discovery::open_repository(&path).and_then(|mut repo| repo.create_commit(message))
    {
        event_sender.send(RepositoryEvent::CommitFailed {
            path,
            message: error.to_string(),
        });
        return;
    }

    event_sender.send(RepositoryEvent::CommitCreated { path: path.clone() });
    sync_repository_forced(state, event_sender, path, RepositorySyncReason::Dirty);
}

fn apply_vcs_operation(
    state: &mut VcsWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    operation: VcsOperation,
    toast_id: u64,
) {
    tracing::debug!(
        path = %path.display(),
        operation = %operation.label(),
        toast_id,
        "vcs: operation requested",
    );
    let result =
        discovery::open_repository(&path).and_then(|mut repo| repo.run_operation(&operation));

    match result {
        Ok(message) => {
            tracing::debug!(
                path = %path.display(),
                operation = %operation.label(),
                "vcs: operation complete",
            );
            event_sender.send(RepositoryEvent::VcsOperationComplete {
                toast_id,
                path: path.clone(),
                operation,
                message,
            });
            sync_repository_forced(state, event_sender, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                operation = %operation.label(),
                %error,
                "vcs: operation failed",
            );
            event_sender.send(RepositoryEvent::VcsOperationFailed {
                toast_id,
                operation,
                message: error.to_string(),
            });
        }
    }
}

fn apply_fetch(
    state: &mut VcsWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    remote: String,
    toast_id: u64,
) {
    tracing::debug!(path = %path.display(), %remote, toast_id, "vcs: fetch requested");
    let result = discovery::open_repository(&path).and_then(|mut repo| repo.fetch_remote(&remote));

    match result {
        Ok(()) => {
            tracing::debug!(path = %path.display(), %remote, "vcs: fetch complete");
            event_sender.send(RepositoryEvent::FetchComplete {
                toast_id,
                path: path.clone(),
                remote,
            });
            sync_repository_forced(state, event_sender, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %remote, %error, "vcs: fetch failed");
            event_sender.send(RepositoryEvent::FetchFailed {
                toast_id,
                remote,
                message: error.to_string(),
            });
        }
    }
}

fn apply_push(
    state: &mut VcsWorkerState,
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
        "vcs: push requested",
    );
    // Parse the branch out of the refspec for the completion event, e.g.
    // `refs/heads/foo:refs/heads/foo` → `foo`.
    let branch = refspec
        .rsplit(':')
        .next()
        .and_then(|dst| dst.rsplit('/').next())
        .unwrap_or("")
        .to_owned();

    let result = discovery::open_repository(&path)
        .and_then(|mut repo| repo.push(&remote, &refspec, force_with_lease));

    match result {
        Ok(()) => {
            tracing::debug!(path = %path.display(), %remote, %branch, "vcs: push complete");
            event_sender.send(RepositoryEvent::PushComplete {
                toast_id,
                path: path.clone(),
                remote,
                branch,
            });
            sync_repository_forced(state, event_sender, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %remote, %error, "vcs: push failed");
            event_sender.send(RepositoryEvent::PushFailed {
                toast_id,
                remote,
                message: error.to_string(),
            });
        }
    }
}

fn apply_publish(
    state: &mut VcsWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    action: Option<PublishAction>,
    toast_id: u64,
) {
    tracing::debug!(path = %path.display(), toast_id, "vcs: publish requested");
    let result = discovery::open_repository(&path).and_then(|mut repo| {
        let action = match action {
            Some(action) => action,
            None => repo.publish_plan()?.primary,
        };
        repo.publish(&action)
    });

    match result {
        Ok(outcome) => {
            tracing::debug!(path = %path.display(), label = %outcome.label, "vcs: publish complete");
            event_sender.send(RepositoryEvent::PublishComplete {
                toast_id,
                path: path.clone(),
                label: outcome.label,
            });
            sync_repository_forced(state, event_sender, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "vcs: publish failed");
            event_sender.send(RepositoryEvent::PublishFailed {
                toast_id,
                message: error.to_string(),
            });
        }
    }
}

fn apply_publish_plan(event_sender: &RuntimeEventSender, path: PathBuf, toast_id: Option<u64>) {
    tracing::debug!(path = %path.display(), ?toast_id, "vcs: publish-plan requested");
    let result = discovery::open_repository(&path).and_then(|mut repo| repo.publish_plan());

    match result {
        Ok(plan) => {
            tracing::debug!(path = %path.display(), "vcs: publish-plan ready");
            event_sender.send(RepositoryEvent::PublishPlanReady {
                toast_id,
                path,
                plan,
            });
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "vcs: publish-plan failed");
            event_sender.send(RepositoryEvent::PublishPlanFailed {
                toast_id,
                message: error.to_string(),
            });
        }
    }
}

fn apply_pull_ff(
    state: &mut VcsWorkerState,
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
        "vcs: pull-ff requested",
    );
    let result = discovery::open_repository(&path)
        .and_then(|mut repo| repo.pull_fast_forward(&remote, &branch));

    match result {
        Ok(outcome) => {
            let (already_up_to_date, behind) = match outcome {
                PullFastForwardOutcome::AlreadyUpToDate => (true, 0),
                PullFastForwardOutcome::FastForwarded { behind } => (false, behind),
            };
            tracing::debug!(
                path = %path.display(),
                %remote,
                %branch,
                already_up_to_date,
                behind,
                "vcs: pull-ff complete",
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
                "vcs: pull-ff failed",
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
    state: &mut VcsWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    reason: RepositorySyncReason,
) {
    sync_repository_inner(state, event_sender, path, reason, true, None, None);
}

fn sync_repository(
    state: &mut VcsWorkerState,
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
    state: &mut VcsWorkerState,
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
    let location = match discovery::discover_repository(&path) {
        Ok(Some(location)) => location,
        Ok(None) => {
            event_sender.send(RepositoryEvent::RepositorySnapshotFailed {
                path: path.clone(),
                reason,
                message: format!("{} is not a supported repository", path.display()),
            });
            return;
        }
        Err(error) => {
            event_sender.send(RepositoryEvent::RepositorySnapshotFailed {
                path,
                reason,
                message: error.to_string(),
            });
            return;
        }
    };
    if state.active_path.as_ref() != Some(&path) {
        state.vcs_repo = None;
        state.last_snapshot = None;
        state.active_path = Some(path.clone());
    }
    sync_vcs_repository(
        state,
        event_sender,
        path,
        reason,
        force_emit,
        dirty_hint,
        reporter_ref,
        location,
    );
}

fn sync_vcs_repository(
    state: &mut VcsWorkerState,
    event_sender: &RuntimeEventSender,
    path: PathBuf,
    reason: RepositorySyncReason,
    force_emit: bool,
    dirty_hint: Option<RepositoryChangeKind>,
    reporter: Option<&dyn ProgressSink>,
    location: RepoLocation,
) {
    if state
        .vcs_repo
        .as_ref()
        .is_none_or(|repo| repo.location() != &location)
    {
        state.vcs_repo = match discovery::open_location(location) {
            Ok(repo) => Some(repo),
            Err(error) => {
                event_sender.send(RepositoryEvent::RepositorySnapshotFailed {
                    path,
                    reason,
                    message: error.to_string(),
                });
                return;
            }
        };
    }
    let Some(repo) = state.vcs_repo.as_mut() else {
        event_sender.send(RepositoryEvent::RepositorySnapshotFailed {
            path,
            reason,
            message: "repository backend is not open".to_owned(),
        });
        return;
    };
    let mut snapshot = match repo.snapshot(reason, reporter) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            event_sender.send(RepositoryEvent::RepositorySnapshotFailed {
                path,
                reason,
                message: error.to_string(),
            });
            return;
        }
    };
    let changed = state
        .last_snapshot
        .as_ref()
        .is_none_or(|previous| neutral_snapshot_changed(previous, &snapshot));
    let should_emit = force_emit || reason == RepositorySyncReason::Open || changed;
    tracing::debug!(
        path = %path.display(),
        ?reason,
        force_emit,
        changed,
        should_emit,
        "sync_repository: computed"
    );
    if should_emit {
        snapshot.change_kind = if reason == RepositorySyncReason::Open {
            None
        } else {
            dirty_hint.or(if force_emit {
                Some(RepositoryChangeKind::Worktree)
            } else {
                None
            })
        };
        event_sender.send(RepositoryEvent::RepositorySnapshotReady(
            RepositorySnapshot::from_vcs_snapshot(snapshot.clone()),
        ));
    }
    state.last_snapshot = Some(snapshot);
}

fn neutral_snapshot_changed(previous: &VcsSnapshot, next: &VcsSnapshot) -> bool {
    previous.location != next.location
        || previous.capabilities != next.capabilities
        || previous.refs != next.refs
        || previous.changes != next.changes
        || previous.file_changes != next.file_changes
}
