use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use diffy_ui::diff_view::decorator::DiffDecorator;
use quark::view;
use quark_app::testing::{By, UiTestHarness};
use quark_app::{UiApp, UiContext, ViewContext};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole as Role;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome};

use super::view::default_card_width;
use super::{AuthStatus, ReviewController, ReviewOverlay, overlay, view as title_view};
use crate::app::shell::Slot;
use crate::app::text_targets::{REVIEW_COMPOSER, TextCx, TextHost, focus_id};
use crate::apprt::services::{GitHubBackend, ReviewServices};
use crate::contracts::commands::ToastUpdate;
use crate::contracts::navigation::{
    RepositoryContext, RepositoryRevision, RepositorySnapshot, RepositorySyncReason,
};
use crate::contracts::review::{
    AvatarImage, CardKey, GitHubRequest, GitHubResponse, PullRequestComparison, PullRequestRef,
    ReviewAction, ReviewEffect, ReviewEvent, Secret,
};
use crate::contracts::shared::{
    AccountEpoch, CompareGeneration, FileKey, FileRevision, SharedContext, Side, SourceTarget,
};
use crate::contracts::shell::{
    Action, AppStartup, Controller, Effect, Intent, LogicalFocus, Reduction, ShellAction,
    ShellEffect,
};
use crate::contracts::viewport::CompareManifestSnapshot;
use crate::contracts::viewport::pipeline::{FileStats, ManifestFile};
use crate::core::compare::stats::CompareFilePaths;
use crate::core::forge::github::{
    DeviceFlowState, GitHubPullRequestReviewData, GitHubPullRequestReviewThread,
    GitHubPullRequestReviewThreadComment, GitHubReviewSide, GitHubReviewThreadResolution,
    GitHubUser, PullRequestInfo, PullRequestReview,
};
use crate::core::review::{
    ReviewAnchor, ReviewComment, ReviewCommentId, ReviewDecision, ReviewDraftKind,
    ReviewDraftState, ReviewLineRange, ReviewResolution, ReviewSession, ReviewSide, ReviewThread,
    ReviewThreadId, ReviewThreadPermissions, ReviewThreadStatus,
};
use crate::platform::persistence::SettingsStore;
use crate::platform::review_store::ReviewStore;
use crate::platform::startup::{Args, StartupOptions};

const URL: &str = "https://github.com/owner/repo/pull/42";
const OTHER_URL: &str = "https://github.com/owner/repo/pull/7";
const THREAD_NODE: &str = "thread-1";

fn info(number: i32) -> PullRequestInfo {
    PullRequestInfo {
        title: "Tidy the widget".to_owned(),
        state: "open".to_owned(),
        author_login: "author".to_owned(),
        number,
        additions: 3,
        deletions: 1,
        changed_files: 1,
        base_branch: "main".to_owned(),
        head_branch: "feature".to_owned(),
        base_sha: "base".to_owned(),
        head_sha: format!("head{number}"),
        base_repo_url: String::new(),
        head_repo_url: String::new(),
    }
}

fn comparison(number: i32) -> PullRequestComparison {
    PullRequestComparison {
        info: info(number),
        left_ref: format!("refs/diffy/pr/{number}/base"),
        right_ref: format!("refs/diffy/pr/{number}/head"),
    }
}

fn graphql_thread() -> GitHubPullRequestReviewThread {
    GitHubPullRequestReviewThread {
        node_id: THREAD_NODE.to_owned(),
        path: "src/widget.rs".to_owned(),
        line: Some(12),
        diff_side: Some(GitHubReviewSide::Right),
        subject_type: "LINE".to_owned(),
        viewer_can_reply: true,
        viewer_can_resolve: true,
        comments: vec![GitHubPullRequestReviewThreadComment {
            node_id: "comment-1".to_owned(),
            database_id: Some(11),
            author_login: "alpha".to_owned(),
            body: "Consider caching this.".to_owned(),
            path: "src/widget.rs".to_owned(),
            line: Some(12),
            viewer_can_update: true,
            ..GitHubPullRequestReviewThreadComment::default()
        }],
        ..GitHubPullRequestReviewThread::default()
    }
}

fn fixture_comment(thread: &ReviewThreadId, id: i64, author: &str, body: &str) -> ReviewComment {
    ReviewComment {
        id: ReviewCommentId::github(id),
        backend_id: Some(id),
        backend_node_id: Some(format!("node-{id}")),
        thread_id: thread.clone(),
        in_reply_to: None,
        in_reply_to_node_id: None,
        author_login: Some(author.to_owned()),
        author_avatar_url: None,
        body: body.to_owned(),
        anchor: None,
        html_url: None,
        created_at: None,
        updated_at: None,
        outdated: false,
        state: None,
        viewer_can_update: false,
        viewer_can_delete: false,
        reactions: Vec::new(),
    }
}

fn sample_review_thread() -> ReviewThread {
    let id = ReviewThreadId::github_node("harness-thread-1");
    ReviewThread {
        comments: vec![
            fixture_comment(
                &id,
                1001,
                "alpha",
                "This recomputes the layout every frame even when nothing changed.",
            ),
            fixture_comment(
                &id,
                1002,
                "bravo",
                "`memoize_height()` over recompute keeps it **stable** while *pinned*.",
            ),
            fixture_comment(
                &id,
                1003,
                "alpha",
                "Cache it behind a dirty flag:\n```rust\nfn cached_height(w: u32) -> u16 {\n    measure(w)\n}\n```",
            ),
        ],
        id,
        backend_node_id: Some("harness-thread-1".to_owned()),
        anchor: Some(ReviewAnchor::inline(
            "src/widget.rs",
            ReviewSide::New,
            ReviewLineRange::new(120, 1),
        )),
        status: ReviewThreadStatus {
            resolution: ReviewResolution::Unresolved,
            outdated: false,
            collapsed: false,
        },
        permissions: ReviewThreadPermissions {
            can_reply: true,
            can_resolve: true,
            can_unresolve: false,
        },
    }
}

fn manifest() -> Rc<CompareManifestSnapshot> {
    Rc::new(CompareManifestSnapshot {
        generation: CompareGeneration(1),
        resolved_left: "base".to_owned(),
        resolved_right: "head".to_owned(),
        files: Arc::from([ManifestFile {
            key: FileKey(5),
            revision: FileRevision(2),
            paths: CompareFilePaths::from_paths(Some("src/widget.rs"), Some("src/widget.rs")),
            status: diffy_diff::FileStatus::Modified,
            binary: false,
            stats: FileStats::Pending,
        }]),
    })
}

fn target(lines: std::ops::Range<u32>) -> SourceTarget {
    SourceTarget {
        file: FileKey(5),
        revision: FileRevision(2),
        side: Side::New,
        lines,
    }
}

#[derive(Default)]
struct Script {
    data: Vec<GitHubPullRequestReviewThread>,
    cached: bool,
    fail_replies: bool,
    fail_submit: bool,
    requests: Vec<GitHubRequest>,
}

#[derive(Clone, Default)]
struct FakeGitHub(Arc<Mutex<Script>>);

impl FakeGitHub {
    fn with<R>(&self, f: impl FnOnce(&mut Script) -> R) -> R {
        let mut script = self.0.lock().unwrap();
        f(&mut script)
    }
}

impl GitHubBackend for FakeGitHub {
    fn execute(&self, request: GitHubRequest, emit: &mut dyn FnMut(GitHubResponse)) {
        let (data, cached, fail_replies, fail_submit) = self.with(|s| {
            s.requests.push(request.clone());
            (s.data.clone(), s.cached, s.fail_replies, s.fail_submit)
        });
        let pr_number = |url: &str| if url == OTHER_URL { 7 } else { 42 };
        match request {
            GitHubRequest::LoadPullRequest { url, .. } => {
                let number = pr_number(&url);
                if cached {
                    emit(GitHubResponse::PullRequestCached {
                        url: url.clone(),
                        comparison: comparison(number),
                    });
                }
                emit(GitHubResponse::PullRequestLoaded {
                    url,
                    comparison: comparison(number),
                });
            }
            GitHubRequest::FetchUser { .. } => emit(GitHubResponse::UserFetched(GitHubUser {
                login: "reviewer".to_owned(),
                name: "Rae Viewer".to_owned(),
                avatar_url: "https://avatars.test/u/1".to_owned(),
            })),
            GitHubRequest::FetchReviewData { pr, .. } => emit(GitHubResponse::ReviewDataLoaded {
                pr,
                data: Box::new(GitHubPullRequestReviewData {
                    threads: data,
                    ..GitHubPullRequestReviewData::default()
                }),
            }),
            GitHubRequest::FetchReviewComments { pr, .. } => {
                emit(GitHubResponse::ReviewCommentsLoaded {
                    pr,
                    comments: Vec::new(),
                })
            }
            GitHubRequest::AddThreadReply {
                pr, thread_node_id, ..
            } => emit(if fail_replies {
                GitHubResponse::ThreadReplyAddFailed {
                    pr,
                    thread_node_id,
                    message: "reply rejected".to_owned(),
                }
            } else {
                GitHubResponse::ThreadReplyAdded {
                    pr,
                    thread_node_id,
                    comment: GitHubPullRequestReviewThreadComment::default(),
                }
            }),
            GitHubRequest::SubmitSessionDrafts { session, .. } => {
                let pr = PullRequestRef {
                    owner: session.target.owner.clone(),
                    repo: session.target.repo.clone(),
                    number: session.target.number,
                };
                let draft_ids = session.pending_drafts().map(|d| d.id).collect();
                emit(if fail_submit {
                    GitHubResponse::DraftsSubmitFailed {
                        pr,
                        draft_ids,
                        message: "review rejected".to_owned(),
                    }
                } else {
                    GitHubResponse::DraftsSubmitted {
                        pr,
                        review: PullRequestReview::default(),
                        draft_ids,
                    }
                })
            }
            GitHubRequest::SetThreadResolution {
                pr,
                thread_node_id,
                resolved,
                ..
            } => emit(GitHubResponse::ThreadResolutionChanged {
                pr,
                resolution: GitHubReviewThreadResolution {
                    thread_node_id,
                    is_resolved: resolved,
                },
            }),
            GitHubRequest::UpdateCommentGraphql { pr, .. } => {
                emit(GitHubResponse::CommentGraphqlUpdated {
                    pr,
                    comment: GitHubPullRequestReviewThreadComment::default(),
                })
            }
            GitHubRequest::DeleteCommentGraphql {
                pr,
                comment_node_id,
                ..
            } => emit(GitHubResponse::CommentGraphqlDeleted {
                pr,
                comment_node_id,
                comment: None,
            }),
            other => panic!("unexpected request {other:?}"),
        }
    }
}

struct Fake {
    github: FakeGitHub,
    services: ReviewServices,
    hold: bool,
    held: Vec<ReviewEffect>,
    effects: Vec<ReviewEffect>,
    _dir: Option<tempfile::TempDir>,
}

impl Fake {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut fake = Self::at(dir.path());
        fake._dir = Some(dir);
        fake
    }

    fn at(dir: &Path) -> Self {
        let github = FakeGitHub::default();
        github.with(|s| s.data = vec![graphql_thread()]);
        let settings = SettingsStore::new_in(dir.join("settings.json"));
        let store = ReviewStore::open(dir.join("review-sessions.json")).unwrap();
        Self {
            services: ReviewServices::new(settings, store, Arc::new(github.clone())),
            github,
            hold: false,
            held: Vec::new(),
            effects: Vec::new(),
            _dir: None,
        }
    }

    fn run(&mut self, effect: ReviewEffect) -> Vec<ReviewEvent> {
        self.effects.push(effect.clone());
        if self.hold {
            self.held.push(effect);
            return Vec::new();
        }
        let mut events = Vec::new();
        match effect {
            ReviewEffect::GitHub { scope, request } => {
                self.services.github().execute(*request, &mut |response| {
                    events.push(ReviewEvent::GitHub {
                        scope,
                        response: Box::new(response),
                    })
                });
            }
            ReviewEffect::StartDeviceFlow { flow, .. } => {
                events.push(ReviewEvent::DeviceFlowStarted {
                    flow,
                    state: DeviceFlowState {
                        device_code: "device".to_owned(),
                        user_code: "ABCD-1234".to_owned(),
                        verification_uri: "https://github.com/login/device".to_owned(),
                        interval: 5,
                    },
                })
            }
            ReviewEffect::PollDeviceFlow { flow, .. } => {
                events.push(ReviewEvent::DeviceFlowCompleted {
                    flow,
                    token: Secret::new("device-token"),
                })
            }
            ReviewEffect::LoadToken => events.push(ReviewEvent::TokenLoaded {
                token: Some(Secret::new("stored-token")),
            }),
            ReviewEffect::SaveToken(_) | ReviewEffect::ClearToken => {}
            ReviewEffect::LoadSession {
                scope,
                target,
                pull_request,
            } => events.push(
                match self.services.load_session(target.clone(), *pull_request) {
                    Ok(session) => ReviewEvent::SessionLoaded {
                        scope,
                        target,
                        session: Box::new(session),
                    },
                    Err(error) => ReviewEvent::SessionLoadFailed {
                        scope,
                        target,
                        message: error.to_string(),
                    },
                },
            ),
            ReviewEffect::SaveSession { scope, session } => {
                let key = self.services.save_session(&session).unwrap();
                events.push(ReviewEvent::SessionSaved { scope, key });
            }
            ReviewEffect::FetchAvatar { url } => events.push(ReviewEvent::AvatarFetched {
                url,
                image: Arc::new(AvatarImage {
                    rgba: vec![255; 16],
                    width: 2,
                    height: 2,
                }),
            }),
        }
        events
    }

    fn requests(&self) -> Vec<GitHubRequest> {
        self.github.with(|s| s.requests.clone())
    }
}

struct Driver {
    review: ReviewController,
    shared: SharedContext,
    fake: Fake,
    focus: Vec<Option<LogicalFocus>>,
    clipboard: Vec<String>,
    browser: Vec<String>,
    toasts: Vec<ToastUpdate>,
    compares: usize,
}

impl Driver {
    fn new(fake: Fake) -> Self {
        Self {
            review: ReviewController::with_grammars(crate::core::syntax::grammar::Grammars::none()),
            shared: SharedContext::default(),
            fake,
            focus: Vec::new(),
            clipboard: Vec::new(),
            browser: Vec::new(),
            toasts: Vec::new(),
            compares: 0,
        }
    }

    fn act(&mut self, action: ReviewAction) {
        let reduction = self.review.reduce_action(action, &self.shared);
        self.drive(reduction);
    }

    fn deliver(&mut self, event: ReviewEvent) {
        let reduction = self.review.reduce_event(event, &self.shared);
        self.drive(reduction);
    }

    fn intent(&mut self, intent: Intent) {
        self.drive(Reduction::default().intent(intent));
    }

    fn release(&mut self) {
        self.fake.hold = false;
        for effect in std::mem::take(&mut self.fake.held) {
            for event in self.fake.run(effect) {
                self.deliver(event);
            }
        }
    }

    fn drive(&mut self, reduction: Reduction) {
        let mut events = VecDeque::new();
        for effect in reduction.effects {
            match effect {
                Effect::Review(effect) => events.extend(self.fake.run(effect)),
                Effect::Shell(ShellEffect::Focus(focus)) => self.focus.push(focus),
                Effect::Shell(ShellEffect::SetClipboard(text)) => self.clipboard.push(text),
                Effect::Shell(ShellEffect::OpenBrowser { url }) => self.browser.push(url),
                other => panic!("unexpected effect {other:?}"),
            }
        }
        for intent in reduction.intents {
            match &intent {
                Intent::AccountChanged => {
                    self.shared.account = AccountEpoch(self.shared.account.0 + 1)
                }
                Intent::SetReviewTarget(review) => self.shared.review = review.clone(),
                Intent::SettingsChanged(settings) => self.shared.settings = settings.clone(),
                Intent::PublishManifest(manifest) => self.shared.manifest = Some(manifest.clone()),
                Intent::RepositoryChanged(repository) => {
                    self.shared.repository = Some(repository.clone())
                }
                Intent::Toast(toast) => self.toasts.push(toast.clone()),
                Intent::RequestCompare(_) => self.compares += 1,
                _ => {}
            }
            let follow = self.review.apply_intent(&intent, &self.shared);
            self.drive(follow);
        }
        while let Some(event) = events.pop_front() {
            self.deliver(event);
        }
    }

    fn startup(&mut self, open_pr: Option<&str>) {
        let args = Args {
            repo: None,
            left: None,
            right: None,
            compare_mode: None,
            layout: None,
            renderer: None,
            file_index: None,
            file_path: None,
            open_pr: open_pr.map(str::to_owned),
        };
        let options =
            StartupOptions::from_parts_with_keyring(args, None, "client".to_owned(), false, true);
        self.intent(Intent::Startup(Rc::new(AppStartup {
            options,
            settings: crate::platform::persistence::Settings::default(),
        })));
    }

    fn open_repository(&mut self) {
        self.intent(Intent::RepositoryChanged(Rc::new(RepositoryContext {
            revision: RepositoryRevision(1),
            snapshot: Arc::new(repository_snapshot(PathBuf::from("/tmp/repo"))),
        })));
        self.intent(Intent::PublishManifest(manifest()));
    }

    fn signed_in_with_pr() -> Self {
        let mut driver = Self::new(Fake::new());
        driver.startup(Some(URL));
        driver.open_repository();
        driver
    }

    fn session(&self) -> &ReviewSession {
        self.review.session().unwrap()
    }

    fn type_text(&mut self, text: &str) {
        let cx = TextCx {
            shared: &self.shared,
            now_ms: 0,
        };
        self.review.edit_text(
            REVIEW_COMPOSER,
            TextEditCommand::InsertText(text.to_owned()),
            &cx,
        );
    }

    fn thread(&self) -> ReviewThreadId {
        ReviewThreadId::github_node(THREAD_NODE)
    }

    fn errors(&self) -> Vec<String> {
        self.toasts
            .iter()
            .filter_map(|toast| match toast {
                ToastUpdate::Show {
                    kind: crate::contracts::commands::ToastKind::Error,
                    message,
                    ..
                } => Some(message.clone()),
                _ => None,
            })
            .collect()
    }
}

fn repository_snapshot(path: PathBuf) -> RepositorySnapshot {
    RepositorySnapshot {
        location: crate::core::vcs::model::RepoLocation {
            kind: crate::core::vcs::model::VcsKind::GIT,
            profile: crate::core::vcs::model::VCS_PROFILE_GIT,
            workspace_root: path.clone(),
            store_root: None,
        },
        path,
        reason: RepositorySyncReason::Open,
        change_kind: None,
        capabilities: crate::core::vcs::model::RepoCapabilities::git(),
        refs: Vec::new(),
        changes: Vec::new(),
        operation_log: Vec::new(),
        file_changes: Vec::new(),
        publish_plan: None,
    }
}

#[test]
fn startup_loads_the_token_then_the_handed_off_pull_request() {
    let mut driver = Driver::new(Fake::new());
    driver.startup(Some(URL));

    assert!(driver.review.account().token.is_some());
    assert!(driver.review.active_pull_request().is_none());

    driver.open_repository();

    let active = driver.review.active_pull_request().unwrap();
    assert_eq!(active.pr.number, 42);
    assert_eq!(active.comparison, Some(comparison(42)));
    assert_eq!(
        driver.shared.review.as_ref().unwrap().pull_request.number,
        42
    );
    assert_eq!(driver.compares, 1);
    assert_eq!(driver.session().threads.len(), 1);
    assert_eq!(
        driver
            .review
            .account()
            .user
            .as_ref()
            .map(|u| u.login.as_str()),
        Some("reviewer")
    );
    assert!(driver.review.account().avatar.is_some());
    assert!(matches!(
        driver.fake.requests().as_slice(),
        [
            GitHubRequest::FetchUser { .. },
            GitHubRequest::LoadPullRequest { .. },
            GitHubRequest::FetchReviewData { .. }
        ]
    ));
}

#[test]
fn cached_comparison_publishes_before_the_fresh_load_without_a_second_compare() {
    let fake = Fake::new();
    fake.github.with(|s| s.cached = true);
    let mut driver = Driver::new(fake);
    driver.startup(Some(URL));
    driver.open_repository();

    assert_eq!(driver.compares, 1);
    assert_eq!(
        driver.review.active_pull_request().unwrap().comparison,
        Some(comparison(42))
    );
}

#[test]
fn review_data_is_anchored_to_manifest_source_targets() {
    let driver = Driver::signed_in_with_pr();

    let presentation = driver.review.presentation();
    let annotation = &presentation.annotations()[0];

    assert_eq!(annotation.card, CardKey::Thread(driver.thread()));
    assert_eq!(annotation.target, target(11..12));
}

#[test]
fn stale_results_are_rejected_after_an_account_switch() {
    let mut driver = Driver::signed_in_with_pr();
    driver.fake.hold = true;
    driver.act(ReviewAction::SetThreadResolved {
        id: driver.thread(),
        resolved: true,
    });
    let held = std::mem::take(&mut driver.fake.held);
    driver.fake.hold = false;
    driver.act(ReviewAction::SignOut);
    driver.act(ReviewAction::StartDeviceFlow);
    driver.act(ReviewAction::LoadPullRequest {
        url: URL.to_owned(),
    });
    assert!(driver.review.account().token.is_some());
    driver.fake.held = held;
    driver.release();

    let thread = &driver.session().threads[0];
    assert_eq!(thread.status.resolution, ReviewResolution::Unresolved);
}

#[test]
fn stale_results_are_rejected_after_a_pull_request_switch() {
    let mut driver = Driver::signed_in_with_pr();
    driver.fake.hold = true;
    driver.act(ReviewAction::SetThreadResolved {
        id: driver.thread(),
        resolved: true,
    });
    let held = std::mem::take(&mut driver.fake.held);
    driver.fake.hold = false;
    driver.act(ReviewAction::LoadPullRequest {
        url: OTHER_URL.to_owned(),
    });
    assert_eq!(driver.review.active_pull_request().unwrap().pr.number, 7);
    let resolved_before = driver.session().threads[0].status.resolution;
    driver.fake.held = held;
    driver.release();

    assert_eq!(driver.review.active_pull_request().unwrap().pr.number, 7);
    assert_eq!(
        driver.session().threads[0].status.resolution,
        resolved_before
    );
    assert_eq!(resolved_before, ReviewResolution::Unresolved);
}

#[test]
fn device_flow_signs_in_and_changes_the_account() {
    let mut driver = Driver::new(Fake::new());
    driver.startup(None);
    driver.act(ReviewAction::SignOut);
    let epoch = driver.shared.account;
    driver.toasts.clear();

    driver.act(ReviewAction::OpenAuthModal);
    assert_eq!(driver.review.overlay_open(), Some(ReviewOverlay::Auth));
    driver.fake.hold = true;
    driver.act(ReviewAction::StartDeviceFlow);
    assert_eq!(driver.review.account().auth, AuthStatus::Starting);
    driver.release();

    assert_eq!(driver.review.account().auth, AuthStatus::Done);
    assert_eq!(
        driver.review.account().token.as_ref().map(Secret::expose),
        Some("device-token")
    );
    assert_eq!(driver.review.overlay_open(), None);
    assert_eq!(driver.shared.account.0, epoch.0 + 1);
    assert_eq!(driver.browser, ["https://github.com/login/device"]);
}

#[test]
fn sign_out_clears_the_account_and_review_target() {
    let mut driver = Driver::signed_in_with_pr();
    let epoch = driver.shared.account;

    driver.act(ReviewAction::OpenAccountMenu);
    driver.act(ReviewAction::SignOut);

    assert!(driver.review.account().token.is_none());
    assert!(driver.review.account().user.is_none());
    assert!(driver.review.active_pull_request().is_none());
    assert!(driver.shared.review.is_none());
    assert_eq!(driver.shared.account.0, epoch.0 + 1);
    assert!(driver.fake.effects.contains(&ReviewEffect::ClearToken));
}

#[test]
fn inline_comment_stages_a_durable_draft_and_returns_focus() {
    let mut driver = Driver::signed_in_with_pr();

    driver.act(ReviewAction::OpenComposerAt(target(19..21)));
    assert_eq!(
        driver.focus.last(),
        Some(&Some(LogicalFocus::ReviewComposer))
    );
    driver.type_text("Name this constant.");
    driver.act(ReviewAction::SubmitComment);

    let draft = driver.session().drafts.values().next().unwrap();
    assert_eq!(draft.kind, ReviewDraftKind::InlineComment);
    assert_eq!(draft.body, "Name this constant.");
    let anchor = draft.anchor.as_ref().unwrap();
    assert_eq!(anchor.path, "src/widget.rs");
    assert_eq!(anchor.line_range, Some(ReviewLineRange::new(20, 2)));
    assert_eq!(driver.focus.last(), Some(&Some(LogicalFocus::Viewport)));
    assert!(driver.review.composer().is_none());
    assert!(matches!(
        driver.fake.effects.last(),
        Some(ReviewEffect::SaveSession { .. })
    ));
}

#[test]
fn drafts_persist_across_a_simulated_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut driver = Driver::new(Fake::at(dir.path()));
        driver.startup(Some(URL));
        driver.open_repository();
        driver.act(ReviewAction::OpenComposerAt(target(4..5)));
        driver.type_text("Keep this after restart.");
        driver.act(ReviewAction::SubmitComment);
        driver.act(ReviewAction::ReplyToThread(driver.thread()));
        driver.type_text("Half-written reply");
        driver.act(ReviewAction::CloseOverlay);
    }

    let mut driver = Driver::new(Fake::at(dir.path()));
    driver.startup(Some(URL));
    driver.open_repository();

    let drafts: Vec<_> = driver.session().drafts.values().collect();
    assert_eq!(drafts.len(), 2);
    assert!(
        drafts
            .iter()
            .any(|d| d.body == "Keep this after restart."
                && d.kind == ReviewDraftKind::InlineComment)
    );
    let reply = drafts
        .iter()
        .find(|d| matches!(d.kind, ReviewDraftKind::Reply { .. }))
        .unwrap();
    assert_eq!(reply.body, "Half-written reply");
    let presentation = driver.review.presentation();
    assert!(
        presentation
            .cards()
            .iter()
            .any(|card| matches!(card.key, CardKey::Draft(_)) && card.target == Some(target(4..5)))
    );

    driver.act(ReviewAction::ReplyToThread(driver.thread()));
    assert_eq!(driver.review.host.composer_text(), "Half-written reply");
}

#[test]
fn reply_failure_keeps_the_draft_for_retry() {
    let mut driver = Driver::signed_in_with_pr();
    driver.fake.github.with(|s| s.fail_replies = true);

    driver.act(ReviewAction::ReplyToThread(driver.thread()));
    driver.type_text("Agreed.");
    driver.act(ReviewAction::SubmitComment);

    let draft = driver.session().drafts.values().next().unwrap();
    assert_eq!(
        draft.state,
        ReviewDraftState::Failed("reply rejected".to_owned())
    );
    assert_eq!(driver.errors(), ["reply rejected"]);
    let card = driver
        .review
        .presentation()
        .card(&CardKey::Thread(driver.thread()))
        .cloned()
        .unwrap();
    assert_eq!(card.retry_draft, Some(draft.id));

    driver.fake.github.with(|s| s.fail_replies = false);
    driver.act(ReviewAction::EditDraft(draft.id));
    assert_eq!(driver.review.host.composer_text(), "Agreed.");
    driver.act(ReviewAction::SubmitComment);
    assert!(driver.session().drafts.is_empty());
}

#[test]
fn submit_review_recovers_from_partial_failure() {
    let mut driver = Driver::signed_in_with_pr();
    driver.act(ReviewAction::OpenComposerAt(target(2..3)));
    driver.type_text("Inline note");
    driver.act(ReviewAction::SubmitComment);
    driver.fake.github.with(|s| s.fail_replies = true);
    driver.act(ReviewAction::ReplyToThread(driver.thread()));
    driver.type_text("Queued reply");
    driver.act(ReviewAction::SubmitComment);
    driver.fake.github.with(|s| s.fail_replies = true);

    driver.act(ReviewAction::SubmitReview {
        decision: ReviewDecision::Comment,
    });

    let drafts: Vec<_> = driver.session().drafts.values().cloned().collect();
    assert_eq!(drafts.len(), 1, "the inline draft was submitted");
    assert!(matches!(drafts[0].kind, ReviewDraftKind::Reply { .. }));
    assert!(matches!(drafts[0].state, ReviewDraftState::Failed(_)));
    assert_eq!(driver.session().metrics().failed_drafts, 1);
    let submitted = driver
        .fake
        .requests()
        .into_iter()
        .filter_map(|request| match request {
            GitHubRequest::SubmitSessionDrafts { session, .. } => Some(session.drafts.len()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(submitted, [1]);

    driver.fake.github.with(|s| s.fail_replies = false);
    driver.act(ReviewAction::SubmitReview {
        decision: ReviewDecision::Comment,
    });

    assert!(driver.session().drafts.is_empty());
    let submits = driver
        .fake
        .requests()
        .iter()
        .filter(|request| matches!(request, GitHubRequest::SubmitSessionDrafts { .. }))
        .count();
    assert_eq!(submits, 1, "only the failed reply is resent");
}

#[test]
fn submit_review_failure_marks_the_batch_failed() {
    let mut driver = Driver::signed_in_with_pr();
    driver.fake.github.with(|s| s.fail_submit = true);
    driver.act(ReviewAction::OpenComposerAt(target(2..3)));
    driver.type_text("Inline note");
    driver.act(ReviewAction::SubmitComment);

    driver.act(ReviewAction::SubmitReview {
        decision: ReviewDecision::RequestChanges,
    });

    let draft = driver.session().drafts.values().next().unwrap();
    assert_eq!(
        draft.state,
        ReviewDraftState::Failed("review rejected".to_owned())
    );
    assert_eq!(driver.errors(), ["review rejected"]);
}

#[test]
fn resolve_and_reopen_update_the_thread() {
    let mut driver = Driver::signed_in_with_pr();

    driver.act(ReviewAction::SetThreadResolved {
        id: driver.thread(),
        resolved: true,
    });
    assert_eq!(
        driver.session().threads[0].status.resolution,
        ReviewResolution::Resolved
    );
    assert!(!driver.review.thread_expanded(&driver.thread()));

    driver.act(ReviewAction::SetThreadResolved {
        id: driver.thread(),
        resolved: false,
    });
    assert_eq!(
        driver.session().threads[0].status.resolution,
        ReviewResolution::Unresolved
    );
}

#[test]
fn edit_and_delete_go_through_graphql_and_refetch() {
    let mut driver = Driver::signed_in_with_pr();

    driver.act(ReviewAction::EditComment {
        comment_node_id: "comment-1".to_owned(),
    });
    assert_eq!(driver.review.host.composer_text(), "Consider caching this.");
    driver.type_text(" Please.");
    driver.act(ReviewAction::SubmitComment);
    driver.act(ReviewAction::DeleteComment {
        comment_node_id: "comment-1".to_owned(),
    });

    let requests = driver.fake.requests();
    assert!(requests.iter().any(|r| matches!(r,
        GitHubRequest::UpdateCommentGraphql { body, .. } if body == "Consider caching this. Please.")));
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, GitHubRequest::DeleteCommentGraphql { .. }))
    );
    let refetches = requests
        .iter()
        .filter(|r| matches!(r, GitHubRequest::FetchReviewData { .. }))
        .count();
    assert_eq!(refetches, 3);
}

#[test]
fn commenting_signed_out_opens_sign_in() {
    let mut driver = Driver::signed_in_with_pr();
    driver.act(ReviewAction::SignOut);
    driver.act(ReviewAction::ReplyToThread(driver.thread()));

    assert_eq!(driver.review.overlay_open(), Some(ReviewOverlay::Auth));
    assert!(driver.review.composer().is_none());
}

#[test]
fn formatting_shortcuts_write_markdown() {
    let mut driver = Driver::signed_in_with_pr();
    driver.act(ReviewAction::OpenComposer);
    driver.type_text("fast");
    let cx = TextCx {
        shared: &driver.shared,
        now_ms: 0,
    };
    driver
        .review
        .edit_text(REVIEW_COMPOSER, TextEditCommand::SelectAll, &cx);
    driver.review.edit_text(
        REVIEW_COMPOSER,
        TextEditCommand::ToggleStyle(quark_ui::text_input::InlineStyle::BOLD),
        &cx,
    );
    assert_eq!(driver.review.host.composer_text(), "**fast**");

    driver.act(ReviewAction::FormatComment(
        crate::contracts::review::ComposerFormat::Code,
    ));
    assert_eq!(driver.review.host.composer_text(), "**fast**`code`");
}

struct App {
    driver: Driver,
}

impl App {
    fn new(driver: Driver) -> Self {
        Self { driver }
    }

    fn apply_focus(&mut self, cx: &mut UiContext) {
        for focus in self.driver.focus.drain(..) {
            cx.set_focus(focus.map(focus_id));
        }
        for text in self.driver.clipboard.drain(..) {
            cx.window.set_clipboard_text(&text);
        }
    }
}

impl UiApp for App {
    type Action = Action;
    type Message = ReviewEvent;

    fn message(&mut self, event: ReviewEvent, cx: &mut UiContext) {
        self.driver.deliver(event);
        self.apply_focus(cx);
        cx.window.request_redraw();
    }

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        let (width, height) = cx.frame.size();
        let title_height = cx.theme.metrics.title_bar_height;
        let shared = self.driver.shared.clone();
        let slot = Slot {
            shared: &shared,
            width,
            height: title_height,
        };
        let title = title_view(&mut self.driver.review, &slot, cx);
        let presentation = self.driver.review.presentation();
        let card_width = default_card_width(width, cx.theme);
        let cards: Vec<AnyElement> = presentation
            .cards()
            .iter()
            .filter_map(|card| presentation.card_element(&card.key, card_width))
            .collect();
        let full = Slot {
            shared: &shared,
            width,
            height,
        };
        let overlay = overlay(&self.driver.review, &full, cx);
        view! {
            <div w={width} h={height} class="flex-col relative overflow-hidden">
                <div w={width} h={title_height} class="flex-row items-center">{title}</div>
                <div class="flex-col gap-2 p-2">{...cards}</div>
                {?overlay}
            </div>
        }
    }

    fn update(&mut self, action: Action, cx: &mut UiContext) {
        match action {
            Action::Review(action) => self.driver.act(action),
            Action::Shell(ShellAction::CopyText(text)) => cx.window.set_clipboard_text(&text),
            _ => {}
        }
        self.apply_focus(cx);
    }

    fn edit_text(&mut self, target: FocusId, command: TextEditCommand) -> TextEditOutcome {
        let cx = TextCx {
            shared: &self.driver.shared,
            now_ms: 0,
        };
        self.driver
            .review
            .edit_text(target, command, &cx)
            .unwrap_or_default()
    }

    fn set_preedit(&mut self, target: FocusId, text: String, cursor: Option<(usize, usize)>) {
        self.driver.review.set_preedit(target, text, cursor);
    }
}

fn harness(driver: Driver) -> UiTestHarness<App> {
    UiTestHarness::new(App::new(driver), (1100.0, 900.0), 2.0)
}

fn sample_driver() -> Driver {
    let mut driver = Driver::new(Fake::new());
    driver.fake.github.with(|s| s.data.clear());
    driver.startup(Some(URL));
    driver.open_repository();
    let thread = sample_review_thread();
    let session = driver.review.pr.as_mut().unwrap().session.as_mut().unwrap();
    session.threads = vec![thread];
    driver.review.refresh(&driver.shared);
    driver
}

#[test]
fn signed_out_title_offers_sign_in_and_the_device_code() {
    let mut driver = Driver::new(Fake::new());
    driver.startup(None);
    driver.act(ReviewAction::SignOut);
    driver.fake.hold = true;
    let mut ui = harness(driver);

    ui.click_node(By::role_name(Role::Button, "Sign in to GitHub"));
    let dialog = ui.find(By::role_name(Role::Dialog, "Sign in to GitHub"));
    assert!(dialog.bounds.width > 0.0);
    ui.click_node(By::role_name(Role::Button, "Continue with GitHub"));
    ui.app_mut().driver.fake.hold = false;
    let held = std::mem::take(&mut ui.app_mut().driver.fake.held);
    let start = held.into_iter().next().unwrap();
    let events = ui.app_mut().driver.fake.run(start);
    ui.app_mut().driver.fake.hold = true;
    for event in events {
        ui.send_message(event);
    }

    assert!(ui.painted_text().contains("ABCD-1234"));
    ui.click_node(By::role_name(Role::Button, "Copy code"));
    assert_eq!(ui.clipboard_text().as_deref(), Some("ABCD-1234"));
}

#[test]
fn review_card_fixture_content() {
    let ui = harness(sample_driver());
    let tree = ui.accessibility_tree();
    let painted = ui.painted_text();

    let card = ui.find(By::test_id(
        "review.thread.github-thread-node:harness-thread-1",
    ));
    assert!(
        card.name
            .as_deref()
            .unwrap_or_default()
            .contains("Comment on line R120"),
        "{tree}"
    );
    assert!(painted.contains("Comment on line R120"), "{painted}");
    assert!(painted.contains("Unresolved"));
    ui.find(By::role_name(Role::Button, "Resolve thread"));
    ui.find(By::role_name(Role::Button, "Reply"));
    assert!(painted.contains("memoize_height()"), "{painted}");
    assert!(
        !painted.contains('`'),
        "markdown markers are not painted: {painted}"
    );
    assert!(painted.contains("stable"));
    let author = painted.find("@alpha").expect("author header");
    let body = painted.find("This recomputes").expect("first body");
    assert!(author < body, "header precedes body: {painted}");
    assert!(painted.contains("@bravo replied"));
    assert!(painted.contains("fn cached_height"));
    assert_eq!(tree, harness(sample_driver()).accessibility_tree());
}

#[test]
fn grammar_availability_rebuilds_card_documents() {
    let mut ui = harness(sample_driver());
    let before = ui.app().driver.review.presentation().revision();

    ui.app_mut()
        .driver
        .intent(Intent::GrammarAvailable(Default::default()));
    ui.send_message(ReviewEvent::TokenSaveFailed {
        message: "redraw".to_owned(),
    });

    assert!(ui.app().driver.review.presentation().revision() > before);
    assert!(ui.painted_text().contains("fn cached_height"));
}

#[test]
fn collapse_toggles_the_thread_body() {
    let mut ui = harness(sample_driver());
    ui.click_node(By::test_id(
        "review.toggle.github-thread-node:harness-thread-1",
    ));

    let painted = ui.painted_text();
    assert!(painted.contains("@alpha: This recomputes"), "{painted}");
    assert!(ui.try_find(By::role_name(Role::Button, "Reply")).is_none());

    ui.click_node(By::test_id(
        "review.toggle.github-thread-node:harness-thread-1",
    ));
    ui.find(By::role_name(Role::Button, "Reply"));
}

#[test]
fn reply_composer_takes_focus_and_returns_it_on_send() {
    let mut ui = harness(Driver::signed_in_with_pr());
    ui.click_node(By::role_name(Role::Button, "Reply"));

    let composer = ui.find(By::test_id("review.composer.draft.1"));
    assert!(composer.bounds.height > 0.0);
    assert_eq!(ui.focus(), Some(REVIEW_COMPOSER));
    ui.find(By::role_name(Role::MultilineTextInput, "Review comment"));
    ui.type_text("Done in the next push.");
    ui.key("mod+enter");

    assert_eq!(ui.focus(), Some(focus_id(LogicalFocus::Viewport)));
    assert!(
        ui.try_find(By::test_id("review.composer.draft.1"))
            .is_none()
    );
    let requests = ui.app().driver.fake.requests();
    assert!(requests.iter().any(|r| matches!(r,
        GitHubRequest::AddThreadReply { body, thread_node_id, .. }
            if body == "Done in the next push." && thread_node_id == THREAD_NODE)));
}

#[test]
fn escape_cancels_the_composer_and_discards_the_reply_draft() {
    let mut ui = harness(Driver::signed_in_with_pr());
    ui.click_node(By::role_name(Role::Button, "Reply"));
    ui.type_text("never mind");
    ui.key("escape");

    assert!(
        ui.try_find(By::test_id("review.composer.draft.1"))
            .is_none()
    );
    assert!(ui.app().driver.session().drafts.is_empty());
    assert_eq!(ui.focus(), Some(focus_id(LogicalFocus::Viewport)));
}

#[test]
fn composer_preview_renders_markdown() {
    let mut ui = harness(Driver::signed_in_with_pr());
    ui.click_node(By::role_name(Role::Button, "Reply"));
    ui.type_text("Use `cache()` here");
    ui.click_node(By::role_name(Role::Button, "Preview"));

    let painted = ui.painted_text();
    assert!(painted.contains("cache()"), "{painted}");
    assert!(!painted.contains("`cache()`"), "{painted}");
    assert!(
        ui.try_find(By::role_name(Role::MultilineTextInput, "Review comment"))
            .is_none()
    );

    ui.click_node(By::role_name(Role::Button, "Write"));
    ui.find(By::role_name(Role::MultilineTextInput, "Review comment"));
}

#[test]
fn staged_drafts_render_cards_and_submit_review() {
    let mut driver = Driver::signed_in_with_pr();
    driver.act(ReviewAction::OpenComposerAt(target(2..3)));
    driver.type_text("Inline note");
    driver.act(ReviewAction::SubmitComment);
    let mut ui = harness(driver);

    let card = ui.find(By::test_id("review.draft.1"));
    assert!(card.bounds.height > 0.0);
    assert!(ui.painted_text().contains("Pending"));
    ui.click_node(By::role_name(Role::Button, "Submit review"));

    assert!(ui.try_find(By::test_id("review.draft.1")).is_none());
    assert!(
        ui.try_find(By::role_name(Role::Button, "Submit review"))
            .is_none()
    );
}

#[test]
fn resolve_button_resolves_the_thread() {
    let mut ui = harness(Driver::signed_in_with_pr());
    ui.click_node(By::role_name(Role::Button, "Resolve thread"));

    assert_eq!(
        ui.app().driver.session().threads[0].status.resolution,
        ReviewResolution::Resolved
    );
    assert!(ui.painted_text().contains("Resolved"));
}

#[test]
fn account_menu_signs_out() {
    let mut ui = harness(Driver::signed_in_with_pr());
    ui.click_node(By::test_id("review.account"));
    ui.find(By::test_id("review.account_menu"));
    assert!(ui.painted_text().contains("Rae Viewer"));
    ui.click_node(By::role_name(Role::Button, "Sign out"));

    ui.find(By::role_name(Role::Button, "Sign in to GitHub"));
    assert!(ui.try_find(By::test_id("review.account_menu")).is_none());
}

#[test]
fn copy_shortcut_copies_the_card_selection() {
    let mut ui = harness(sample_driver());
    let card = CardKey::Thread(ReviewThreadId::github_node("harness-thread-1"));
    let selected = {
        let driver = &mut ui.app_mut().driver;
        driver.review.host.select_all_for_test(&card);
        driver.review.host.selected_text(&card)
    };
    let selected = selected.expect("selection");
    assert!(selected.contains("memoize_height()"));
    assert!(!selected.contains('`'));

    ui.app_mut()
        .driver
        .act(ReviewAction::CopyCardSelection(card));
    assert_eq!(ui.app().driver.clipboard.last(), Some(&selected));
}
