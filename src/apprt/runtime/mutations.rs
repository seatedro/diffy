use std::sync::Arc;
use std::thread;
use std::time::Instant;

use crate::apprt::services::MutationsServices;
use crate::apprt::transport::{ProgressGate, ProgressKey, RuntimeEventSender, ScopedEventSender};
use crate::apprt::vcs_worker::{VcsCommand, VcsEvents, VcsWorker, VcsWorkerEvent};
use crate::contracts::mutations::{MutationsEffect, MutationsEvent};
use crate::contracts::navigation::{NavigationEvent, RepositoryRevision};
use crate::contracts::shared::{CompareGeneration, WorkspaceEpoch};
use crate::contracts::viewport::pipeline::PipelineEvent;

pub struct MutationsRuntime {
    services: MutationsServices,
    vcs: VcsWorker,
    keyring_enabled: bool,
}

impl MutationsRuntime {
    pub(crate) fn new(services: MutationsServices, vcs: VcsWorker, keyring_enabled: bool) -> Self {
        Self {
            services,
            vcs,
            keyring_enabled,
        }
    }

    pub fn dispatch(&self, effect: MutationsEffect, events: ScopedEventSender) {
        let workspace = events.workspace();
        match effect {
            MutationsEffect::ApplyFileOperation(request) => {
                let command = match <[_; 1]>::try_from(request.file_changes) {
                    Ok([file_change]) => VcsCommand::ApplyOperation {
                        job: request.job,
                        path: request.repo_path,
                        file_change,
                        operation: request.operation,
                    },
                    Err(file_changes) => VcsCommand::ApplyBatchOperation {
                        job: request.job,
                        path: request.repo_path,
                        file_changes,
                        operation: request.operation,
                    },
                };
                self.vcs.dispatch(workspace, command);
            }
            MutationsEffect::LoadMutationSource(key) => {
                self.vcs
                    .dispatch(workspace, VcsCommand::LoadMutationSource { key });
            }
            MutationsEffect::ApplyPatch(request) => {
                let selection = request.selection.patch_selection();
                self.vcs.dispatch(
                    workspace,
                    VcsCommand::ApplyPatch {
                        job: request.job,
                        source: request.source,
                        selection,
                        operation: request.operation,
                    },
                );
            }
            MutationsEffect::CreateCommit {
                job,
                repo_path,
                message,
                amend,
            } => self.vcs.dispatch(
                workspace,
                VcsCommand::Commit {
                    job,
                    path: repo_path,
                    message,
                    amend,
                },
            ),
            MutationsEffect::LoadLastCommit { repo_path } => self
                .vcs
                .dispatch(workspace, VcsCommand::LastCommit { path: repo_path }),
            MutationsEffect::UndoCommit {
                job,
                repo_path,
                sha,
            } => self.vcs.dispatch(
                workspace,
                VcsCommand::UndoCommit {
                    job,
                    path: repo_path,
                    sha,
                },
            ),
            MutationsEffect::RestoreDiscard { job, snapshot } => self
                .vcs
                .dispatch(workspace, VcsCommand::RestoreDiscard { job, snapshot }),
            MutationsEffect::RunOperation {
                job,
                repo_path,
                operation,
            } => self.vcs.dispatch(
                workspace,
                VcsCommand::RunOperation {
                    job,
                    path: repo_path,
                    operation,
                },
            ),
            MutationsEffect::FetchRemote {
                job,
                repo_path,
                remote,
            } => self.vcs.dispatch(
                workspace,
                VcsCommand::Fetch {
                    job,
                    path: repo_path,
                    remote,
                },
            ),
            MutationsEffect::Push {
                job,
                repo_path,
                remote,
                refspec,
                force_with_lease,
            } => self.vcs.dispatch(
                workspace,
                VcsCommand::Push {
                    job,
                    path: repo_path,
                    remote,
                    refspec,
                    force_with_lease,
                },
            ),
            MutationsEffect::Publish {
                job,
                repo_path,
                action,
            } => self.vcs.dispatch(
                workspace,
                VcsCommand::Publish {
                    job,
                    path: repo_path,
                    action: action.map(|action| *action),
                },
            ),
            MutationsEffect::LoadPublishPlan { job, repo_path } => self.vcs.dispatch(
                workspace,
                VcsCommand::PublishPlan {
                    job,
                    path: repo_path,
                },
            ),
            MutationsEffect::PullFastForward {
                job,
                repo_path,
                remote,
                branch,
            } => self.vcs.dispatch(
                workspace,
                VcsCommand::PullFf {
                    job,
                    path: repo_path,
                    remote,
                    branch,
                },
            ),
            MutationsEffect::LoadAiKeys => {
                if !self.keyring_enabled {
                    events.send(MutationsEvent::AiKeysLoaded {
                        openai: None,
                        anthropic: None,
                    });
                    return;
                }
                let services = self.services;
                thread::spawn(move || {
                    let event = match services.load_ai_keys() {
                        Ok((openai, anthropic)) => {
                            MutationsEvent::AiKeysLoaded { openai, anthropic }
                        }
                        Err(error) => MutationsEvent::AiKeysLoadFailed {
                            message: error.user_message(),
                        },
                    };
                    events.send(event);
                });
            }
            MutationsEffect::SaveAiKey { kind, value } => {
                if !self.keyring_enabled {
                    return;
                }
                let services = self.services;
                thread::spawn(move || {
                    if let Err(error) = services.save_ai_key(kind, &value) {
                        events.send(MutationsEvent::AiKeySaveFailed {
                            message: error.user_message(),
                        });
                    }
                });
            }
            MutationsEffect::ClearAiKey { kind } => {
                if !self.keyring_enabled {
                    return;
                }
                let services = self.services;
                thread::spawn(move || {
                    if let Err(error) = services.clear_ai_key(kind) {
                        tracing::warn!("failed to clear AI key from keyring: {error}");
                    }
                });
            }
            MutationsEffect::CheckStagedDiff {
                generation,
                repo_path,
                has_staged,
            } => {
                let services = self.services;
                thread::spawn(move || {
                    let staged = services.staged_digest(&repo_path, has_staged).ok();
                    events.send(MutationsEvent::StagedDiffChecked { generation, staged });
                });
            }
            MutationsEffect::GenerateCommitMessage(request) => {
                let services = self.services;
                thread::spawn(move || {
                    services.generate_commit_message(*request, &mut |event| events.send(event));
                });
            }
        }
    }
}

pub(crate) struct VcsEventRouter {
    events: RuntimeEventSender,
    progress: ProgressGate,
}

impl VcsEventRouter {
    pub(crate) fn new(events: RuntimeEventSender) -> Self {
        Self {
            events,
            progress: ProgressGate::default(),
        }
    }

    fn admit(&self, key: ProgressKey) -> bool {
        self.progress.admit(key, Instant::now())
    }
}

impl VcsEvents for VcsEventRouter {
    fn emit(&self, workspace: WorkspaceEpoch, event: VcsWorkerEvent) {
        let events = self.events.scoped(workspace);
        match event {
            VcsWorkerEvent::Progress { generation, phase } => {
                if self.admit(ProgressKey::compare(generation, &phase)) {
                    events.send(PipelineEvent::CompareProgress {
                        generation: CompareGeneration(generation),
                        phase,
                    });
                }
            }
            VcsWorkerEvent::SnapshotReady { revision, snapshot } => {
                events.send(NavigationEvent::SnapshotReady {
                    revision: RepositoryRevision(revision),
                    snapshot: Arc::new(snapshot),
                });
            }
            VcsWorkerEvent::SnapshotFailed {
                path,
                reason,
                message,
            } => {
                events.send(NavigationEvent::SnapshotFailed {
                    path,
                    reason,
                    message,
                });
            }
            VcsWorkerEvent::FileOperationApplied { job, path, undo } => {
                events.send(MutationsEvent::FileOperationApplied { job, path, undo });
            }
            VcsWorkerEvent::FileOperationFailed { job, path, message } => {
                events.send(MutationsEvent::FileOperationFailed { job, path, message });
            }
            VcsWorkerEvent::LastCommitLoaded { path, commit } => {
                events.send(MutationsEvent::LastCommitLoaded { path, commit });
            }
            VcsWorkerEvent::LastCommitFailed { message } => {
                events.send(MutationsEvent::LastCommitFailed { message });
            }
            VcsWorkerEvent::CommitUndone { job, path } => {
                events.send(MutationsEvent::CommitUndone { job, path });
            }
            VcsWorkerEvent::CommitUndoFailed { job, message } => {
                events.send(MutationsEvent::CommitUndoFailed { job, message });
            }
            VcsWorkerEvent::DiscardRestored { job, path } => {
                events.send(MutationsEvent::DiscardRestored { job, path });
            }
            VcsWorkerEvent::DiscardRestoreFailed { job, message } => {
                events.send(MutationsEvent::DiscardRestoreFailed { job, message });
            }
            VcsWorkerEvent::CommitCreated { job, path, commit } => {
                events.send(MutationsEvent::CommitCreated { job, path, commit });
            }
            VcsWorkerEvent::CommitFailed { job, path, message } => {
                events.send(MutationsEvent::CommitFailed { job, path, message });
            }
            VcsWorkerEvent::OperationComplete {
                job,
                path,
                operation,
                message,
            } => {
                events.send(MutationsEvent::OperationComplete {
                    job,
                    path,
                    operation,
                    message,
                });
            }
            VcsWorkerEvent::OperationFailed {
                job,
                operation,
                message,
            } => {
                events.send(MutationsEvent::OperationFailed {
                    job,
                    operation,
                    message,
                });
            }
            VcsWorkerEvent::FetchProgress {
                job,
                received_objects,
                total_objects,
                received_bytes,
            } => {
                if self.admit(ProgressKey::Fetch(job)) {
                    events.send(MutationsEvent::FetchProgress {
                        job,
                        received_objects,
                        total_objects,
                        received_bytes,
                    });
                }
            }
            VcsWorkerEvent::FetchComplete { job, path, remote } => {
                self.progress.finish(ProgressKey::Fetch(job));
                events.send(MutationsEvent::FetchComplete { job, path, remote });
            }
            VcsWorkerEvent::FetchFailed {
                job,
                remote,
                message,
            } => {
                self.progress.finish(ProgressKey::Fetch(job));
                events.send(MutationsEvent::FetchFailed {
                    job,
                    remote,
                    message,
                });
            }
            VcsWorkerEvent::PushProgress {
                job,
                current,
                total,
                bytes,
            } => {
                if self.admit(ProgressKey::Push(job)) {
                    events.send(MutationsEvent::PushProgress {
                        job,
                        current,
                        total,
                        bytes,
                    });
                }
            }
            VcsWorkerEvent::PushComplete {
                job,
                path,
                remote,
                branch,
            } => {
                self.progress.finish(ProgressKey::Push(job));
                events.send(MutationsEvent::PushComplete {
                    job,
                    path,
                    remote,
                    branch,
                });
            }
            VcsWorkerEvent::PushFailed {
                job,
                remote,
                message,
            } => {
                self.progress.finish(ProgressKey::Push(job));
                events.send(MutationsEvent::PushFailed {
                    job,
                    remote,
                    message,
                });
            }
            VcsWorkerEvent::PublishComplete { job, path, label } => {
                events.send(MutationsEvent::PublishComplete { job, path, label });
            }
            VcsWorkerEvent::PublishFailed { job, message } => {
                events.send(MutationsEvent::PublishFailed { job, message });
            }
            VcsWorkerEvent::PublishPlanReady { job, path, plan } => {
                events.send(MutationsEvent::PublishPlanReady {
                    job,
                    path,
                    plan: Box::new(plan),
                });
            }
            VcsWorkerEvent::PublishPlanFailed { job, message } => {
                events.send(MutationsEvent::PublishPlanFailed { job, message });
            }
            VcsWorkerEvent::PullComplete {
                job,
                path,
                remote,
                branch,
                already_up_to_date,
                behind,
            } => {
                events.send(MutationsEvent::PullComplete {
                    job,
                    path,
                    remote,
                    branch,
                    already_up_to_date,
                    behind,
                });
            }
            VcsWorkerEvent::PullFailed {
                job,
                remote,
                branch,
                message,
            } => {
                events.send(MutationsEvent::PullFailed {
                    job,
                    remote,
                    branch,
                    message,
                });
            }
            VcsWorkerEvent::MutationSourceReady { source } => {
                events.send(MutationsEvent::MutationSourceReady { source });
            }
            VcsWorkerEvent::MutationSourceFailed { key, message } => {
                events.send(MutationsEvent::MutationSourceFailed { key, message });
            }
            VcsWorkerEvent::WorkerStopped => {
                events.send(MutationsEvent::WorkerStopped);
            }
        }
    }
}
