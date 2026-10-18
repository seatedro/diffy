use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::contracts::mutations::{MutationSource, MutationSourceKey};
use crate::contracts::navigation::{
    RepositoryChangeKind, RepositorySnapshot, RepositorySyncReason,
};
use crate::contracts::shared::{JobId, WorkspaceEpoch};
use crate::core::compare::{ComparePhase, ProgressSink};
use crate::core::vcs::{
    backend::VcsRepository,
    discovery,
    model::{
        FileChange, FileOperation, PublishAction, PublishPlan, PullFastForwardOutcome,
        RepoLocation, VcsOperation, VcsSnapshot,
    },
    mutation::{self, PatchSelection},
};

const VCS_DIRTY_DEBOUNCE: Duration = Duration::from_millis(150);

pub(crate) trait VcsEvents: Send + Sync + 'static {
    fn emit(&self, workspace: WorkspaceEpoch, event: VcsWorkerEvent);
}

#[derive(Debug, Clone)]
pub(crate) enum VcsWorkerEvent {
    Progress {
        generation: u64,
        phase: ComparePhase,
    },
    SnapshotReady {
        revision: u64,
        snapshot: RepositorySnapshot,
    },
    SnapshotFailed {
        path: PathBuf,
        reason: RepositorySyncReason,
        message: String,
    },
    FileOperationApplied {
        job: JobId,
        path: PathBuf,
    },
    FileOperationFailed {
        job: JobId,
        path: PathBuf,
        message: String,
    },
    CommitCreated {
        job: JobId,
        path: PathBuf,
    },
    CommitFailed {
        job: JobId,
        path: PathBuf,
        message: String,
    },
    OperationComplete {
        job: JobId,
        path: PathBuf,
        operation: VcsOperation,
        message: String,
    },
    OperationFailed {
        job: JobId,
        operation: VcsOperation,
        message: String,
    },
    FetchProgress {
        job: JobId,
        received_objects: usize,
        total_objects: usize,
        received_bytes: usize,
    },
    FetchComplete {
        job: JobId,
        path: PathBuf,
        remote: String,
    },
    FetchFailed {
        job: JobId,
        remote: String,
        message: String,
    },
    PushProgress {
        job: JobId,
        current: usize,
        total: usize,
        bytes: usize,
    },
    PushComplete {
        job: JobId,
        path: PathBuf,
        remote: String,
        branch: String,
    },
    PushFailed {
        job: JobId,
        remote: String,
        message: String,
    },
    PublishComplete {
        job: JobId,
        path: PathBuf,
        label: String,
    },
    PublishFailed {
        job: JobId,
        message: String,
    },
    PublishPlanReady {
        job: Option<JobId>,
        path: PathBuf,
        plan: PublishPlan,
    },
    PublishPlanFailed {
        job: Option<JobId>,
        message: String,
    },
    PullComplete {
        job: JobId,
        path: PathBuf,
        remote: String,
        branch: String,
        already_up_to_date: bool,
        behind: usize,
    },
    PullFailed {
        job: JobId,
        remote: String,
        branch: String,
        message: String,
    },
    MutationSourceReady {
        source: Arc<MutationSource>,
    },
    MutationSourceFailed {
        key: MutationSourceKey,
        message: String,
    },
    /// The VCS worker thread is gone, so dispatched repository commands are
    /// being dropped. Surfaced so the user knows repo operations stopped.
    WorkerStopped,
}

#[derive(Debug, Clone)]
pub(crate) enum VcsCommand {
    Sync {
        path: PathBuf,
        reason: RepositorySyncReason,
        /// When `Some`, the worker emits `CompareProgressUpdate` events
        /// tagged with this generation so the loading panel follows the
        /// repo-open phases. `None` for background resyncs.
        progress: Option<u64>,
    },
    ApplyOperation {
        job: JobId,
        path: PathBuf,
        file_change: FileChange,
        operation: FileOperation,
    },
    ApplyBatchOperation {
        job: JobId,
        path: PathBuf,
        file_changes: Vec<FileChange>,
        operation: FileOperation,
    },
    LoadMutationSource {
        key: MutationSourceKey,
    },
    ApplyPatch {
        job: JobId,
        source: Arc<MutationSource>,
        selection: PatchSelection,
        operation: FileOperation,
    },
    Commit {
        job: JobId,
        path: PathBuf,
        message: String,
    },
    RunOperation {
        job: JobId,
        path: PathBuf,
        operation: VcsOperation,
    },
    Fetch {
        job: JobId,
        path: PathBuf,
        remote: String,
    },
    Push {
        job: JobId,
        path: PathBuf,
        remote: String,
        refspec: String,
        force_with_lease: bool,
    },
    Publish {
        job: JobId,
        path: PathBuf,
        action: Option<PublishAction>,
    },
    PublishPlan {
        job: Option<JobId>,
        path: PathBuf,
    },
    PullFf {
        job: JobId,
        path: PathBuf,
        remote: String,
        branch: String,
    },
    Dirty {
        path: PathBuf,
        change_hint: RepositoryChangeKind,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct VcsWorkerCommand {
    pub workspace: WorkspaceEpoch,
    pub command: VcsCommand,
}

#[derive(Clone)]
pub(crate) struct VcsWorker {
    sender: Sender<VcsWorkerCommand>,
    events: Arc<dyn VcsEvents>,
}

impl VcsWorker {
    pub fn new(events: Arc<dyn VcsEvents>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker_events = Arc::clone(&events);
        thread::spawn(move || vcs_worker_loop(worker_events, receiver));
        Self { sender, events }
    }

    /// Sends a command to the worker thread. If the thread died, every repo
    /// operation would otherwise silently evaporate, so report it through the
    /// runtime event loop where state turns it into a visible error.
    pub fn dispatch(&self, workspace: WorkspaceEpoch, command: VcsCommand) {
        if self
            .sender
            .send(VcsWorkerCommand { workspace, command })
            .is_err()
        {
            tracing::error!("VCS worker thread stopped; dropping repository command");
            self.events.emit(workspace, VcsWorkerEvent::WorkerStopped);
        }
    }

    pub fn sender(&self) -> Sender<VcsWorkerCommand> {
        self.sender.clone()
    }
}

#[derive(Clone)]
struct Emitter {
    events: Arc<dyn VcsEvents>,
    workspace: WorkspaceEpoch,
}

impl Emitter {
    fn emit(&self, event: VcsWorkerEvent) {
        self.events.emit(self.workspace, event);
    }
}

struct SyncProgress {
    emitter: Emitter,
    generation: u64,
}

impl ProgressSink for SyncProgress {
    fn phase(&self, phase: ComparePhase) {
        self.emitter.emit(VcsWorkerEvent::Progress {
            generation: self.generation,
            phase,
        });
    }
}

#[derive(Default)]
struct VcsWorkerState {
    vcs_repo: Option<Box<dyn VcsRepository>>,
    active_path: Option<PathBuf>,
    last_snapshot: Option<VcsSnapshot>,
    snapshot_revision: u64,
}

struct PendingDirty {
    workspace: WorkspaceEpoch,
    path: PathBuf,
    change_hint: RepositoryChangeKind,
}

fn vcs_worker_loop(events: Arc<dyn VcsEvents>, receiver: Receiver<VcsWorkerCommand>) {
    let mut state = VcsWorkerState::default();
    let mut pending_dirty: Option<PendingDirty> = None;

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

        let Some(VcsWorkerCommand { workspace, command }) = command else {
            let Some(dirty) = pending_dirty.take() else {
                continue;
            };
            let emitter = Emitter {
                events: Arc::clone(&events),
                workspace: dirty.workspace,
            };
            sync_repository(
                &mut state,
                &emitter,
                dirty.path,
                RepositorySyncReason::Dirty,
                Some(dirty.change_hint),
                None,
            );
            continue;
        };

        if let VcsCommand::Dirty { path, change_hint } = command {
            pending_dirty = Some(match pending_dirty.take() {
                Some(pending) if pending.path == path && pending.workspace == workspace => {
                    PendingDirty {
                        change_hint: pending.change_hint.merge(change_hint),
                        ..pending
                    }
                }
                _ => PendingDirty {
                    workspace,
                    path,
                    change_hint,
                },
            });
            continue;
        }

        pending_dirty = None;
        let emitter = Emitter {
            events: Arc::clone(&events),
            workspace,
        };
        run_command(&mut state, &emitter, command);
    }
}

fn run_command(state: &mut VcsWorkerState, emitter: &Emitter, command: VcsCommand) {
    match command {
        VcsCommand::Sync {
            path,
            reason,
            progress,
        } => sync_repository(state, emitter, path, reason, None, progress),
        VcsCommand::ApplyOperation {
            job,
            path,
            file_change,
            operation,
        } => {
            let result = discovery::open_repository(&path)
                .and_then(|mut repo| repo.apply_file_operation(&file_change, operation));
            finish_file_operation(state, emitter, job, path, result);
        }
        VcsCommand::ApplyBatchOperation {
            job,
            path,
            file_changes,
            operation,
        } => {
            let result = discovery::open_repository(&path)
                .and_then(|mut repo| repo.apply_batch_file_operation(&file_changes, operation));
            finish_file_operation(state, emitter, job, path, result);
        }
        VcsCommand::LoadMutationSource { key } => load_mutation_source(emitter, key),
        VcsCommand::ApplyPatch {
            job,
            source,
            selection,
            operation,
        } => apply_patch_operation(state, emitter, job, &source, &selection, operation),
        VcsCommand::Commit { job, path, message } => {
            apply_commit(state, emitter, job, path, &message)
        }
        VcsCommand::RunOperation {
            job,
            path,
            operation,
        } => apply_vcs_operation(state, emitter, job, path, operation),
        VcsCommand::Fetch { job, path, remote } => apply_fetch(state, emitter, job, path, remote),
        VcsCommand::Push {
            job,
            path,
            remote,
            refspec,
            force_with_lease,
        } => apply_push(state, emitter, job, path, remote, refspec, force_with_lease),
        VcsCommand::Publish { job, path, action } => {
            apply_publish(state, emitter, job, path, action)
        }
        VcsCommand::PublishPlan { job, path } => apply_publish_plan(emitter, job, path),
        VcsCommand::PullFf {
            job,
            path,
            remote,
            branch,
        } => apply_pull_ff(state, emitter, job, path, remote, branch),
        VcsCommand::Dirty { .. } => {}
    }
}

fn finish_file_operation(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
    job: JobId,
    path: PathBuf,
    result: crate::core::error::Result<()>,
) {
    if let Err(error) = result {
        emitter.emit(VcsWorkerEvent::FileOperationFailed {
            job,
            path,
            message: error.user_message(),
        });
        return;
    }

    emitter.emit(VcsWorkerEvent::FileOperationApplied {
        job,
        path: path.clone(),
    });
    sync_repository_forced(state, emitter, path, RepositorySyncReason::Dirty);
}

fn load_mutation_source(emitter: &Emitter, key: MutationSourceKey) {
    let result = discovery::open_repository(&key.repo_path)
        .and_then(|mut repo| repo.mutation_material(&key.change));
    match result {
        Ok(material) => emitter.emit(VcsWorkerEvent::MutationSourceReady {
            source: Arc::new(MutationSource::new(key, material)),
        }),
        Err(error) => emitter.emit(VcsWorkerEvent::MutationSourceFailed {
            key,
            message: error.user_message(),
        }),
    }
}

fn apply_patch_operation(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
    job: JobId,
    source: &MutationSource,
    selection: &PatchSelection,
    operation: FileOperation,
) {
    let path = source.repo_path().to_path_buf();
    let result = discovery::open_repository(&path).and_then(|mut repo| {
        mutation::apply_selection(
            repo.as_mut(),
            source.material(),
            source.change(),
            selection,
            operation,
        )
    });
    if let Err(error) = result {
        tracing::warn!(
            ?operation,
            path = %path.display(),
            file = %source.change().path,
            error = %error,
            "patch apply failed"
        );
        emitter.emit(VcsWorkerEvent::FileOperationFailed {
            job,
            path,
            message: format!(
                "Could not {} lines: {}",
                operation.label(),
                error.user_message()
            ),
        });
        return;
    }
    emitter.emit(VcsWorkerEvent::FileOperationApplied {
        job,
        path: path.clone(),
    });
    sync_repository_forced(state, emitter, path, RepositorySyncReason::Dirty);
}

fn apply_commit(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
    job: JobId,
    path: PathBuf,
    message: &str,
) {
    if let Err(error) =
        discovery::open_repository(&path).and_then(|mut repo| repo.create_commit(message))
    {
        emitter.emit(VcsWorkerEvent::CommitFailed {
            job,
            path,
            message: error.user_message(),
        });
        return;
    }

    emitter.emit(VcsWorkerEvent::CommitCreated {
        job,
        path: path.clone(),
    });
    sync_repository_forced(state, emitter, path, RepositorySyncReason::Dirty);
}

fn apply_vcs_operation(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
    job: JobId,
    path: PathBuf,
    operation: VcsOperation,
) {
    tracing::debug!(
        path = %path.display(),
        operation = %operation.label(),
        job = job.0,
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
            emitter.emit(VcsWorkerEvent::OperationComplete {
                job,
                path: path.clone(),
                operation,
                message,
            });
            sync_repository_forced(state, emitter, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                operation = %operation.label(),
                %error,
                "vcs: operation failed",
            );
            emitter.emit(VcsWorkerEvent::OperationFailed {
                job,
                operation,
                message: error.user_message(),
            });
        }
    }
}

fn apply_fetch(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
    job: JobId,
    path: PathBuf,
    remote: String,
) {
    tracing::debug!(path = %path.display(), %remote, job = job.0, "vcs: fetch requested");
    let result = discovery::open_repository(&path).and_then(|mut repo| {
        repo.fetch_remote(
            &remote,
            &mut |received_objects, total_objects, received_bytes| {
                emitter.emit(VcsWorkerEvent::FetchProgress {
                    job,
                    received_objects,
                    total_objects,
                    received_bytes,
                });
            },
        )
    });

    match result {
        Ok(()) => {
            tracing::debug!(path = %path.display(), %remote, "vcs: fetch complete");
            emitter.emit(VcsWorkerEvent::FetchComplete {
                job,
                path: path.clone(),
                remote,
            });
            sync_repository_forced(state, emitter, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %remote, %error, "vcs: fetch failed");
            emitter.emit(VcsWorkerEvent::FetchFailed {
                job,
                remote,
                message: error.user_message(),
            });
        }
    }
}

fn apply_push(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
    job: JobId,
    path: PathBuf,
    remote: String,
    refspec: String,
    force_with_lease: bool,
) {
    tracing::debug!(
        path = %path.display(),
        %remote,
        %refspec,
        force_with_lease,
        job = job.0,
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

    let result = discovery::open_repository(&path).and_then(|mut repo| {
        repo.push(
            &remote,
            &refspec,
            force_with_lease,
            &mut |current, total, bytes| {
                emitter.emit(VcsWorkerEvent::PushProgress {
                    job,
                    current,
                    total,
                    bytes,
                });
            },
        )
    });

    match result {
        Ok(()) => {
            tracing::debug!(path = %path.display(), %remote, %branch, "vcs: push complete");
            emitter.emit(VcsWorkerEvent::PushComplete {
                job,
                path: path.clone(),
                remote,
                branch,
            });
            sync_repository_forced(state, emitter, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %remote, %error, "vcs: push failed");
            emitter.emit(VcsWorkerEvent::PushFailed {
                job,
                remote,
                message: error.user_message(),
            });
        }
    }
}

fn apply_publish(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
    job: JobId,
    path: PathBuf,
    action: Option<PublishAction>,
) {
    tracing::debug!(path = %path.display(), job = job.0, "vcs: publish requested");
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
            emitter.emit(VcsWorkerEvent::PublishComplete {
                job,
                path: path.clone(),
                label: outcome.label,
            });
            sync_repository_forced(state, emitter, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "vcs: publish failed");
            emitter.emit(VcsWorkerEvent::PublishFailed {
                job,
                message: error.user_message(),
            });
        }
    }
}

fn apply_publish_plan(emitter: &Emitter, job: Option<JobId>, path: PathBuf) {
    tracing::debug!(path = %path.display(), ?job, "vcs: publish-plan requested");
    let result = discovery::open_repository(&path).and_then(|mut repo| repo.publish_plan());

    match result {
        Ok(plan) => {
            tracing::debug!(path = %path.display(), "vcs: publish-plan ready");
            emitter.emit(VcsWorkerEvent::PublishPlanReady { job, path, plan });
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "vcs: publish-plan failed");
            emitter.emit(VcsWorkerEvent::PublishPlanFailed {
                job,
                message: error.user_message(),
            });
        }
    }
}

fn apply_pull_ff(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
    job: JobId,
    path: PathBuf,
    remote: String,
    branch: String,
) {
    tracing::debug!(
        path = %path.display(),
        %remote,
        %branch,
        job = job.0,
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
            emitter.emit(VcsWorkerEvent::PullComplete {
                job,
                path: path.clone(),
                remote,
                branch,
                already_up_to_date,
                behind,
            });
            sync_repository_forced(state, emitter, path, RepositorySyncReason::Rescan);
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %remote,
                %branch,
                %error,
                "vcs: pull-ff failed",
            );
            emitter.emit(VcsWorkerEvent::PullFailed {
                job,
                remote,
                branch,
                message: error.user_message(),
            });
        }
    }
}

fn sync_repository_forced(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
    path: PathBuf,
    reason: RepositorySyncReason,
) {
    sync_repository_inner(state, emitter, path, reason, true, None, None);
}

fn sync_repository(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
    path: PathBuf,
    reason: RepositorySyncReason,
    dirty_hint: Option<RepositoryChangeKind>,
    progress: Option<u64>,
) {
    sync_repository_inner(state, emitter, path, reason, false, dirty_hint, progress);
}

fn sync_repository_inner(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
    path: PathBuf,
    reason: RepositorySyncReason,
    force_emit: bool,
    dirty_hint: Option<RepositoryChangeKind>,
    progress: Option<u64>,
) {
    let reporter = progress.map(|generation| SyncProgress {
        emitter: emitter.clone(),
        generation,
    });
    let reporter_ref: Option<&dyn ProgressSink> = reporter.as_ref().map(|r| r as &dyn ProgressSink);

    if let Some(r) = reporter_ref {
        r.phase(ComparePhase::OpeningRepo);
    }
    let location = match discovery::discover_repository(&path) {
        Ok(Some(location)) => location,
        Ok(None) => {
            emitter.emit(VcsWorkerEvent::SnapshotFailed {
                path: path.clone(),
                reason,
                message: format!("{} is not a supported repository", path.display()),
            });
            return;
        }
        Err(error) => {
            emitter.emit(VcsWorkerEvent::SnapshotFailed {
                path,
                reason,
                message: error.user_message(),
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
        emitter,
        path,
        reason,
        force_emit,
        dirty_hint,
        reporter_ref,
        location,
    );
}

#[allow(clippy::too_many_arguments)]
fn sync_vcs_repository(
    state: &mut VcsWorkerState,
    emitter: &Emitter,
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
                emitter.emit(VcsWorkerEvent::SnapshotFailed {
                    path,
                    reason,
                    message: error.user_message(),
                });
                return;
            }
        };
    }
    let Some(repo) = state.vcs_repo.as_mut() else {
        emitter.emit(VcsWorkerEvent::SnapshotFailed {
            path,
            reason,
            message: "repository backend is not open".to_owned(),
        });
        return;
    };
    let mut snapshot = match repo.snapshot(reason, reporter) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            emitter.emit(VcsWorkerEvent::SnapshotFailed {
                path,
                reason,
                message: error.user_message(),
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
        let mut payload = RepositorySnapshot::from_vcs_snapshot(snapshot.clone());
        payload.publish_plan = repo
            .publish_plan()
            .map_err(|error| {
                tracing::debug!(path = %path.display(), %error, "vcs: no publish plan for snapshot");
            })
            .ok();
        state.snapshot_revision = state.snapshot_revision.saturating_add(1);
        emitter.emit(VcsWorkerEvent::SnapshotReady {
            revision: state.snapshot_revision,
            snapshot: payload,
        });
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
