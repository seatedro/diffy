use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::apprt::services::ReviewServices;
use crate::apprt::transport::ScopedEventSender;
use crate::contracts::review::{ReviewEffect, ReviewEvent, ReviewScope};
use crate::core::review::ReviewSession;
use crate::platform::startup::GitHubTokenStore;

pub(crate) const DRAFT_SAVE_DEBOUNCE: Duration = Duration::from_millis(400);

struct DraftSave {
    scope: ReviewScope,
    session: Box<ReviewSession>,
    events: ScopedEventSender,
    immediate: bool,
}

struct DraftSaver {
    sender: Option<Sender<DraftSave>>,
    worker: Option<JoinHandle<()>>,
}

impl DraftSaver {
    fn new(services: ReviewServices) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("diffy-review-drafts".to_owned())
            .spawn(move || draft_save_loop(&services, &receiver))
            .ok();
        Self {
            sender: Some(sender),
            worker,
        }
    }

    fn save(&self, request: DraftSave) {
        let rejected = match &self.sender {
            Some(sender) => sender.send(request).err().map(|error| error.0),
            None => Some(request),
        };
        if let Some(request) = rejected {
            request.events.send(ReviewEvent::SessionSaveFailed {
                scope: request.scope,
                key: request.session.key(),
                message: "review draft writer stopped".to_owned(),
            });
        }
    }
}

impl Drop for DraftSaver {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn draft_save_loop(services: &ReviewServices, receiver: &Receiver<DraftSave>) {
    let mut pending: HashMap<String, DraftSave> = HashMap::new();
    loop {
        let next = if pending.is_empty() {
            receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            receiver.recv_timeout(DRAFT_SAVE_DEBOUNCE)
        };
        match next {
            Ok(request) => {
                let immediate = request.immediate;
                pending.insert(request.session.key().storage_key(), request);
                if immediate {
                    write_drafts(services, &mut pending);
                }
            }
            Err(RecvTimeoutError::Timeout) => write_drafts(services, &mut pending),
            Err(RecvTimeoutError::Disconnected) => {
                write_drafts(services, &mut pending);
                return;
            }
        }
    }
}

fn write_drafts(services: &ReviewServices, pending: &mut HashMap<String, DraftSave>) {
    for (_, request) in pending.drain() {
        let DraftSave {
            scope,
            session,
            events,
            ..
        } = request;
        let event = match services.save_session(&session) {
            Ok(key) => ReviewEvent::SessionSaved { scope, key },
            Err(error) => ReviewEvent::SessionSaveFailed {
                scope,
                key: session.key(),
                message: error.user_message(),
            },
        };
        events.send(event);
    }
}

pub struct ReviewRuntime {
    services: ReviewServices,
    token_store: GitHubTokenStore,
    drafts: DraftSaver,
}

impl ReviewRuntime {
    pub fn new(services: ReviewServices, token_store: GitHubTokenStore) -> Self {
        Self {
            drafts: DraftSaver::new(services.clone()),
            services,
            token_store,
        }
    }

    pub fn dispatch(&self, effect: ReviewEffect, events: ScopedEventSender) {
        match effect {
            ReviewEffect::GitHub { scope, request } => {
                let github = Arc::clone(self.services.github());
                thread::spawn(move || {
                    github.execute(*request, &mut |response| {
                        events.send(ReviewEvent::GitHub {
                            scope,
                            response: Box::new(response),
                        });
                    });
                });
            }
            ReviewEffect::StartDeviceFlow { flow, client_id } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let event = match services.start_device_flow(&client_id) {
                        Ok(state) => ReviewEvent::DeviceFlowStarted { flow, state },
                        Err(error) => ReviewEvent::DeviceFlowStartFailed {
                            flow,
                            message: error.user_message(),
                        },
                    };
                    events.send(event);
                });
            }
            ReviewEffect::PollDeviceFlow {
                flow,
                client_id,
                device_code,
                interval_seconds,
            } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let event =
                        match services.poll_device_flow(&client_id, &device_code, interval_seconds)
                        {
                            Ok(token) => ReviewEvent::DeviceFlowCompleted { flow, token },
                            Err(error) => ReviewEvent::DeviceFlowFailed {
                                flow,
                                message: error.user_message(),
                            },
                        };
                    events.send(event);
                });
            }
            ReviewEffect::LoadToken => {
                if self.token_store == GitHubTokenStore::Disabled {
                    events.send(ReviewEvent::TokenLoaded { token: None });
                    return;
                }
                let services = self.services.clone();
                let store = self.token_store;
                thread::spawn(move || {
                    let result = match store {
                        GitHubTokenStore::Keyring => services.load_keyring_token(),
                        GitHubTokenStore::DevFile => services.load_dev_token(),
                        GitHubTokenStore::Disabled => Ok(None),
                    };
                    let event = match result {
                        Ok(token) => ReviewEvent::TokenLoaded { token },
                        Err(error) => ReviewEvent::TokenLoadFailed {
                            message: error.user_message(),
                        },
                    };
                    events.send(event);
                });
            }
            ReviewEffect::SaveToken(token) => {
                if self.token_store == GitHubTokenStore::Disabled {
                    return;
                }
                let services = self.services.clone();
                let store = self.token_store;
                thread::spawn(move || {
                    let result = match store {
                        GitHubTokenStore::Keyring => services.save_keyring_token(&token),
                        GitHubTokenStore::DevFile => services.save_dev_token(&token),
                        GitHubTokenStore::Disabled => Ok(()),
                    };
                    if let Err(error) = result {
                        events.send(ReviewEvent::TokenSaveFailed {
                            message: error.user_message(),
                        });
                    }
                });
            }
            ReviewEffect::ClearToken => {
                if self.token_store == GitHubTokenStore::Disabled {
                    return;
                }
                let services = self.services.clone();
                let store = self.token_store;
                thread::spawn(move || {
                    let result = match store {
                        GitHubTokenStore::Keyring => services.clear_keyring_token(),
                        GitHubTokenStore::DevFile => services.clear_dev_token(),
                        GitHubTokenStore::Disabled => Ok(()),
                    };
                    if let Err(error) = result {
                        tracing::warn!("failed to clear GitHub token: {error}");
                    }
                });
            }
            ReviewEffect::LoadSession {
                scope,
                target,
                pull_request,
            } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let event = match services.load_session(target.clone(), *pull_request) {
                        Ok(session) => ReviewEvent::SessionLoaded {
                            scope,
                            target,
                            session: Box::new(session),
                        },
                        Err(error) => ReviewEvent::SessionLoadFailed {
                            scope,
                            target,
                            message: error.user_message(),
                        },
                    };
                    events.send(event);
                });
            }
            ReviewEffect::SaveSession { scope, session } => self.drafts.save(DraftSave {
                scope,
                session,
                events,
                immediate: true,
            }),
            ReviewEffect::SaveSessionDraft { scope, session } => self.drafts.save(DraftSave {
                scope,
                session,
                events,
                immediate: false,
            }),
            ReviewEffect::FetchAvatar { url } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let event = match services.fetch_avatar(&url) {
                        Ok(image) => ReviewEvent::AvatarFetched {
                            url,
                            image: Arc::new(image),
                        },
                        Err(error) => ReviewEvent::AvatarFetchFailed {
                            url,
                            message: error.user_message(),
                        },
                    };
                    events.send(event);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::apprt::services::LiveGitHub;
    use crate::apprt::transport::RuntimeEventSender;
    use crate::contracts::shared::{AccountEpoch, JobId, WorkspaceEpoch};
    use crate::contracts::shell::{AppEvent, AppMessage};
    use crate::core::forge::github::PullRequestInfo;
    use crate::core::review::{ReviewAnchor, ReviewLineRange, ReviewSide, ReviewTarget};
    use crate::platform::persistence::SettingsStore;
    use crate::platform::review_store::ReviewStore;

    fn session(body: &str) -> Box<ReviewSession> {
        let mut session = ReviewSession::new(
            ReviewTarget::github("owner", "repo", 42),
            PullRequestInfo {
                head_sha: "head".to_owned(),
                ..PullRequestInfo::default()
            },
        );
        session.create_inline_draft(
            ReviewAnchor::inline("a.rs", ReviewSide::New, ReviewLineRange::new(1, 1)),
            body,
        );
        Box::new(session)
    }

    fn runtime(dir: &std::path::Path) -> ReviewRuntime {
        let store = ReviewStore::open(dir.join("review-sessions.json")).unwrap();
        let services = ReviewServices::new(
            SettingsStore::new_in(dir.join("settings.json")),
            store.clone(),
            Arc::new(LiveGitHub::new(store)),
        );
        ReviewRuntime::new(services, GitHubTokenStore::Disabled)
    }

    fn saved_body(dir: &std::path::Path) -> Option<String> {
        let store = ReviewStore::open(dir.join("review-sessions.json")).unwrap();
        let session = store
            .load_session(&ReviewTarget::github("owner", "repo", 42), "head")
            .unwrap()?;
        session
            .drafts
            .values()
            .next()
            .map(|draft| draft.body.clone())
    }

    fn scope(request: u64) -> ReviewScope {
        ReviewScope {
            account: AccountEpoch(0),
            request: JobId(request),
        }
    }

    #[test]
    fn draft_saves_coalesce_and_flush_when_the_runtime_stops() {
        let dir = tempfile::tempdir().unwrap();
        let (sender, messages) = mpsc::channel::<AppMessage>();
        let events = RuntimeEventSender::from_channel(sender);
        let runtime = runtime(dir.path());
        for (n, body) in ["a", "ab", "abc"].into_iter().enumerate() {
            runtime.dispatch(
                ReviewEffect::SaveSessionDraft {
                    scope: scope(n as u64),
                    session: session(body),
                },
                events.scoped(WorkspaceEpoch(1)),
            );
        }
        assert_eq!(saved_body(dir.path()), None);

        drop(runtime);

        assert_eq!(saved_body(dir.path()).as_deref(), Some("abc"));
        let saved = messages
            .try_iter()
            .filter(|message| match message {
                AppMessage::Runtime(envelope) => matches!(
                    envelope.event,
                    AppEvent::Review(ReviewEvent::SessionSaved { .. })
                ),
                AppMessage::SyntaxWake => false,
            })
            .count();
        assert_eq!(saved, 1);
    }

    #[test]
    fn an_immediate_save_supersedes_a_pending_draft_save() {
        let dir = tempfile::tempdir().unwrap();
        let (sender, _messages) = mpsc::channel::<AppMessage>();
        let events = RuntimeEventSender::from_channel(sender);
        let runtime = runtime(dir.path());
        runtime.dispatch(
            ReviewEffect::SaveSessionDraft {
                scope: scope(1),
                session: session("typing"),
            },
            events.scoped(WorkspaceEpoch(1)),
        );
        runtime.dispatch(
            ReviewEffect::SaveSession {
                scope: scope(2),
                session: session("submitted"),
            },
            events.scoped(WorkspaceEpoch(1)),
        );
        thread::sleep(DRAFT_SAVE_DEBOUNCE * 2);

        assert_eq!(saved_body(dir.path()).as_deref(), Some("submitted"));
        drop(runtime);
        assert_eq!(saved_body(dir.path()).as_deref(), Some("submitted"));
    }
}
