use std::thread;

use crate::apprt::services::NavigationServices;
use crate::apprt::transport::ScopedEventSender;
use crate::apprt::vcs_worker::{VcsCommand, VcsWorker};
use crate::apprt::watcher::RepoWatchWorker;
use crate::contracts::navigation::{NavigationEffect, NavigationEvent};

pub struct NavigationRuntime {
    services: NavigationServices,
    vcs: VcsWorker,
    watcher: RepoWatchWorker,
}

impl NavigationRuntime {
    pub(crate) fn new(
        services: NavigationServices,
        vcs: VcsWorker,
        watcher: RepoWatchWorker,
    ) -> Self {
        Self {
            services,
            vcs,
            watcher,
        }
    }

    pub fn dispatch(&self, effect: NavigationEffect, events: ScopedEventSender) {
        match effect {
            NavigationEffect::WatchRepository { path } => {
                self.watcher.dispatch(events.workspace(), path);
            }
            NavigationEffect::SyncRepository(request) => {
                self.vcs.dispatch(
                    events.workspace(),
                    VcsCommand::Sync {
                        path: request.path,
                        reason: request.reason,
                        progress: request.progress.map(|generation| generation.0),
                    },
                );
            }
            NavigationEffect::LoadHistory(request) => {
                let services = self.services;
                thread::spawn(move || {
                    let event = match services.load_history(&request) {
                        Ok(commits) => NavigationEvent::HistoryReady {
                            generation: request.generation,
                            commits,
                        },
                        Err(error) => NavigationEvent::HistoryFailed {
                            generation: request.generation,
                            message: error.user_message(),
                        },
                    };
                    events.send(event);
                });
            }
            NavigationEffect::ResolveRef(request) => {
                let services = self.services;
                thread::spawn(move || {
                    let event = match services.resolve_ref(&request.repo_path, &request.query) {
                        Ok((short_oid, summary)) => NavigationEvent::RefResolved {
                            request: request.request,
                            field: request.field,
                            query: request.query,
                            short_oid,
                            summary,
                        },
                        Err(_) => NavigationEvent::RefResolveFailed {
                            request: request.request,
                            field: request.field,
                            query: request.query,
                        },
                    };
                    events.send(event);
                });
            }
        }
    }
}
