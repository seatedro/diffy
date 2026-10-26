use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::apprt::services::{GitHubBackend, ReviewServices};
use crate::apprt::transport::ScopedEventSender;
use crate::contracts::review::{
    GitHubRequest, GitHubResponse, PullRequestRef, PullWatch, ReviewEffect, ReviewEvent,
    ReviewScope, Secret,
};
use crate::core::forge::github::{PullRequestFingerprint, PullRequestInfo};
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

/// How often the open pull request is checked, after T3 Code's sync: a
/// pull request someone is working on every half minute, one that went
/// quiet every five, a closed one in case it reopens, a merged one not
/// at all. Signed in, a check is one small GraphQL read; signed out,
/// REST, at 60 requests an hour.
const WATCH_ACTIVE: Duration = Duration::from_secs(30);
const WATCH_QUIET: Duration = Duration::from_secs(5 * 60);
const WATCH_CLOSED: Duration = Duration::from_secs(15 * 60);
const WATCH_SIGNED_OUT: Duration = Duration::from_secs(5 * 60);
/// A pull request with nothing new for this long is quiet.
const WATCH_QUIET_AFTER: Duration = Duration::from_secs(30 * 60);
/// Coming back to the window checks at once when the last check is
/// older than this. Signed out, every request counts, however often the
/// window is left and come back to.
const WATCH_REFOCUS: Duration = Duration::from_secs(10);

struct Watch {
    scope: ReviewScope,
    pr: PullRequestRef,
    token: Option<Secret>,
    /// The head the review shows: how the pull request moved is always
    /// told against it, however many pushes were seen since.
    loaded_head: String,
    /// What the last check saw.
    known: PullRequestFingerprint,
    /// `known` has comments and checks to compare: the first check only
    /// learns them.
    marks_known: bool,
    events: ScopedEventSender,
    checked: Instant,
    /// When a check last found something new.
    changed: Instant,
}

impl Watch {
    fn new(
        scope: ReviewScope,
        pr: PullRequestRef,
        token: Option<Secret>,
        info: &PullRequestInfo,
        events: ScopedEventSender,
    ) -> Self {
        let now = Instant::now();
        Self {
            scope,
            pr,
            token,
            loaded_head: info.head_sha.clone(),
            known: PullRequestFingerprint {
                title: info.title.clone(),
                state: info.state.clone(),
                merged: info.merged,
                draft: info.draft,
                head_sha: info.head_sha.clone(),
                base_branch: info.base_branch.clone(),
                ..PullRequestFingerprint::default()
            },
            marks_known: false,
            events,
            checked: now,
            changed: now,
        }
    }

    fn signed_in(&self) -> bool {
        self.token.as_ref().is_some_and(|token| !token.is_empty())
    }

    /// How long until the next check, or `None` for a merged pull
    /// request, which won't change again.
    fn interval(&self) -> Option<Duration> {
        if self.known.merged {
            return None;
        }
        Some(if !self.signed_in() {
            WATCH_SIGNED_OUT
        } else if self.known.state == "closed" {
            WATCH_CLOSED
        } else if self.changed.elapsed() >= WATCH_QUIET_AFTER {
            WATCH_QUIET
        } else {
            WATCH_ACTIVE
        })
    }

    fn refocus(&self) -> Option<Duration> {
        let interval = self.interval()?;
        Some(if self.signed_in() {
            WATCH_REFOCUS
        } else {
            interval
        })
    }

    /// Asks GitHub about the pull request, and reports what changed
    /// since the last check: its head, state or title, its comments, or
    /// its checks.
    fn check(&mut self, github: &dyn GitHubBackend) {
        self.checked = Instant::now();
        let request = GitHubRequest::CheckPullRequest {
            pr: self.pr.clone(),
            token: self.token.clone(),
            head_sha: self.loaded_head.clone(),
        };
        github.execute(request, &mut |response| match response {
            GitHubResponse::PullRequestChecked {
                pr, now, change, ..
            } => {
                let moved = moved(&self.known, &now);
                let remarks_changed = self.marks_known && now.remarks != self.known.remarks;
                let checks_changed = self.marks_known && now.checks != self.known.checks;
                self.marks_known = true;
                self.known = (*now).clone();
                if moved || remarks_changed || checks_changed {
                    self.changed = Instant::now();
                    self.events.send(ReviewEvent::GitHub {
                        scope: self.scope,
                        response: Box::new(GitHubResponse::PullRequestChecked {
                            pr,
                            now,
                            change,
                            remarks_changed,
                            checks_changed,
                        }),
                    });
                }
            }
            GitHubResponse::PullRequestCheckFailed { message, .. } => {
                tracing::debug!("pull request check failed: {message}");
            }
            _ => {}
        });
    }
}

fn moved(known: &PullRequestFingerprint, now: &PullRequestFingerprint) -> bool {
    known.head_sha != now.head_sha
        || known.base_branch != now.base_branch
        || known.state != now.state
        || known.merged != now.merged
        || known.draft != now.draft
        || known.title != now.title
}

struct PullWatcher {
    sender: Option<Sender<(PullWatch, ScopedEventSender)>>,
    worker: Option<JoinHandle<()>>,
}

impl PullWatcher {
    fn new(github: Arc<dyn GitHubBackend>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("diffy-review-watch".to_owned())
            .spawn(move || watch_loop(github.as_ref(), &receiver))
            .ok();
        Self {
            sender: Some(sender),
            worker,
        }
    }

    fn send(&self, command: PullWatch, events: ScopedEventSender) {
        if let Some(sender) = &self.sender {
            let _ = sender.send((command, events));
        }
    }
}

impl Drop for PullWatcher {
    fn drop(&mut self) {
        // Not joined: a check in flight can take a network timeout, and
        // nothing it would report matters once the app is closing.
        self.sender = None;
        self.worker = None;
    }
}

fn watch_loop(github: &dyn GitHubBackend, receiver: &Receiver<(PullWatch, ScopedEventSender)>) {
    let mut watch: Option<Watch> = None;
    let mut focused = true;
    loop {
        let wait = watch
            .as_ref()
            .filter(|_| focused)
            .and_then(|watch| Some(watch.interval()?.saturating_sub(watch.checked.elapsed())));
        let next = match wait {
            Some(wait) => receiver.recv_timeout(wait),
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match next {
            Ok((
                PullWatch::Start {
                    scope,
                    pr,
                    token,
                    info,
                },
                events,
            )) => watch = Some(Watch::new(scope, pr, token, &info, events)),
            Ok((PullWatch::Stop, _)) => watch = None,
            Ok((PullWatch::WindowFocused(now), _)) => {
                focused = now;
                if let Some(watch) = watch.as_mut().filter(|w| {
                    now && w
                        .refocus()
                        .is_some_and(|refocus| w.checked.elapsed() >= refocus)
                }) {
                    watch.check(github);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(watch) = watch.as_mut() {
                    watch.check(github);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

pub struct ReviewRuntime {
    services: ReviewServices,
    token_store: GitHubTokenStore,
    drafts: DraftSaver,
    watcher: PullWatcher,
}

impl ReviewRuntime {
    pub fn new(services: ReviewServices, token_store: GitHubTokenStore) -> Self {
        Self {
            drafts: DraftSaver::new(services.clone()),
            watcher: PullWatcher::new(Arc::clone(services.github())),
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
            ReviewEffect::WatchPullRequest(command) => self.watcher.send(command, events),
            ReviewEffect::LoadSession {
                scope,
                target,
                pull_request,
                repo_path,
                merge_base,
                carry,
            } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let loaded = services.load_session(
                        target.clone(),
                        *pull_request,
                        repo_path.as_deref(),
                        merge_base.as_deref(),
                        carry.map(|carry| *carry),
                    );
                    let event = match loaded {
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
            ReviewEffect::ResolveClones { job, current } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let clones = services.resolve_clones(current);
                    events.send(ReviewEvent::ClonesResolved { job, clones });
                });
            }
            ReviewEffect::PrepareReviewCache { repo } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let result = services
                        .prepare_review_cache(&repo)
                        .map_err(|error| error.user_message());
                    events.send(ReviewEvent::ReviewCacheReady { repo, result });
                });
            }
            ReviewEffect::ValidateClone { repo, path } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let result = services
                        .validate_clone(&repo, &path)
                        .map_err(|error| error.user_message());
                    events.send(ReviewEvent::CloneValidated { repo, result });
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

    /// Answers every check with the pull request as it is set to.
    struct Pushes(std::sync::Mutex<PullRequestFingerprint>);

    impl Pushes {
        fn set(&self, change: impl FnOnce(&mut PullRequestFingerprint)) {
            change(&mut self.0.lock().unwrap());
        }
    }

    impl GitHubBackend for Pushes {
        fn execute(&self, request: GitHubRequest, emit: &mut dyn FnMut(GitHubResponse)) {
            let GitHubRequest::CheckPullRequest { pr, head_sha, .. } = request else {
                return;
            };
            let now = self.0.lock().unwrap().clone();
            let change = if now.head_sha == head_sha {
                crate::contracts::review::HeadChange::Same
            } else {
                crate::contracts::review::HeadChange::Ahead(1)
            };
            emit(GitHubResponse::PullRequestChecked {
                pr,
                now: Box::new(now),
                change,
                remarks_changed: false,
                checks_changed: false,
            });
        }
    }

    #[test]
    fn a_watch_reports_each_change_once_and_stops_at_a_merge() {
        let (sender, messages) = mpsc::channel::<AppMessage>();
        let events = RuntimeEventSender::from_channel(sender);
        let open = PullRequestFingerprint {
            state: "open".to_owned(),
            head_sha: "one".to_owned(),
            remarks: "1 comment".to_owned(),
            ..PullRequestFingerprint::default()
        };
        let github = Pushes(std::sync::Mutex::new(open));
        let info = PullRequestInfo {
            state: "open".to_owned(),
            head_sha: "one".to_owned(),
            ..PullRequestInfo::default()
        };
        let pr = PullRequestRef {
            owner: "owner".to_owned(),
            repo: "repo".to_owned(),
            number: 42,
        };
        let mut watch = Watch::new(
            scope(1),
            pr,
            Some(Secret::new("token")),
            &info,
            events.scoped(WorkspaceEpoch(1)),
        );
        let reported = |messages: &mpsc::Receiver<AppMessage>| {
            messages
                .try_iter()
                .filter_map(|message| match message {
                    AppMessage::Runtime(envelope) => match envelope.event {
                        AppEvent::Review(ReviewEvent::GitHub { response, .. }) => match *response {
                            GitHubResponse::PullRequestChecked {
                                now,
                                remarks_changed,
                                ..
                            } => Some((now.head_sha, remarks_changed)),
                            _ => None,
                        },
                        _ => None,
                    },
                    AppMessage::SyntaxWake => None,
                })
                .collect::<Vec<_>>()
        };

        watch.check(&github);
        assert!(
            reported(&messages).is_empty(),
            "the first check only learns"
        );
        assert_eq!(watch.interval(), Some(WATCH_ACTIVE));

        github.set(|pr| pr.head_sha = "two".to_owned());
        watch.check(&github);
        watch.check(&github);
        assert_eq!(reported(&messages), vec![("two".to_owned(), false)]);

        github.set(|pr| pr.remarks = "2 comments".to_owned());
        watch.check(&github);
        assert_eq!(reported(&messages), vec![("two".to_owned(), true)]);

        github.set(|pr| pr.merged = true);
        watch.check(&github);
        assert_eq!(reported(&messages).len(), 1);
        assert_eq!(watch.interval(), None, "a merged pull request is done");
    }
}
