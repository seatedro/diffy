use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use quark_components::AvatarImage;
use quark_ui::FocusId;
use quark_ui::text_input::{InlineStyle, TextEditCommand, TextEditOutcome};

use super::composer::ComposerTarget;
use super::presentation::{CardHost, ReviewPresentation};
use crate::app::text_targets::{REVIEW_COMPOSER, TextCx, TextHost};
use crate::contracts::commands::{SettingsSnapshot, ToastKind, ToastUpdate};
use crate::contracts::review::{
    ComposerFormat, GitHubRequest, GitHubResponse, PullRequestComparison, PullRequestRef,
    ReviewAction, ReviewContext, ReviewEffect, ReviewEvent, ReviewScope, Secret,
};
use crate::contracts::shared::{AccountEpoch, JobId, SharedContext, SourceTarget};
use crate::contracts::shell::{Controller, Intent, LogicalFocus, Reduction, ShellEffect};
use crate::contracts::viewport::CompareIntent;
use crate::core::compare::RendererKind;
use crate::core::forge::github::{DeviceFlowState, GitHubUser, parse_pr_url};
use crate::core::review::{
    ReviewAnchor, ReviewDecision, ReviewDraftId, ReviewDraftKind, ReviewDraftState,
    ReviewResolution, ReviewSession, ReviewSessionStatus, ReviewTarget, ReviewThreadId,
};
use crate::core::vcs::model::{VcsCompareRequest, VcsCompareSpec};

const ACCOUNT_AVATAR_PX: u32 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewOverlay {
    Auth,
    AccountMenu,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AuthStatus {
    #[default]
    Idle,
    Starting,
    Awaiting(DeviceFlowState),
    Failed(String),
    Done,
}

#[derive(Debug, Default)]
pub struct Account {
    pub token: Option<Secret>,
    pub user: Option<GitHubUser>,
    pub avatar: Option<AvatarImage>,
    pub avatar_url: Option<String>,
    pub auth: AuthStatus,
    pub flow: Option<JobId>,
    pub token_loaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    LoadPullRequest,
    User,
    ReviewData,
    ReviewComments,
    Reply(ReviewDraftId),
    Mutation,
    SubmitDrafts,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InFlight {
    pr: Option<PullRequestRef>,
    kind: Pending,
}

#[derive(Debug)]
pub struct ActivePullRequest {
    pub pr: PullRequestRef,
    pub url: String,
    pub comparison: Option<PullRequestComparison>,
    pub session: Option<ReviewSession>,
    pub loading: bool,
    pub error: Option<String>,
    session_dirty: bool,
}

impl ActivePullRequest {
    fn target(&self) -> ReviewTarget {
        ReviewTarget::github(self.pr.owner.clone(), self.pr.repo.clone(), self.pr.number)
    }

    pub fn web_url(&self) -> String {
        if self.url.is_empty() {
            format!(
                "https://github.com/{}/{}/pull/{}",
                self.pr.owner, self.pr.repo, self.pr.number
            )
        } else {
            self.url.clone()
        }
    }
}

#[derive(Debug, Clone)]
pub struct Composer {
    pub target: ComposerTarget,
    pub preview: bool,
}

pub struct ReviewController {
    epoch: AccountEpoch,
    client_id: String,
    pub(super) account: Account,
    pub(super) overlay: Option<ReviewOverlay>,
    pending_url: Option<String>,
    repo_path: Option<PathBuf>,
    pub(super) pr: Option<ActivePullRequest>,
    next_job: u64,
    in_flight: HashMap<JobId, InFlight>,
    pub(super) expanded: HashMap<ReviewThreadId, bool>,
    pub(super) composer: Option<Composer>,
    focus_request: Option<LogicalFocus>,
    pub(super) host: CardHost,
    presentation: Rc<ReviewPresentation>,
}

impl Default for ReviewController {
    fn default() -> Self {
        let host = CardHost::default();
        Self {
            epoch: AccountEpoch::default(),
            client_id: String::new(),
            account: Account::default(),
            overlay: None,
            pending_url: None,
            repo_path: None,
            pr: None,
            next_job: 0,
            in_flight: HashMap::new(),
            expanded: HashMap::new(),
            composer: None,
            focus_request: None,
            presentation: Rc::new(ReviewPresentation::empty(host.clone())),
            host,
        }
    }
}

impl std::fmt::Debug for ReviewController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReviewController")
            .field("account", &self.account)
            .field("overlay", &self.overlay)
            .field("pr", &self.pr)
            .field("composer", &self.composer)
            .finish_non_exhaustive()
    }
}

fn toast(kind: ToastKind, message: impl Into<String>) -> Intent {
    Intent::Toast(ToastUpdate::Show {
        id: None,
        kind,
        message: message.into(),
        description: None,
    })
}

fn info(message: impl Into<String>) -> Intent {
    toast(ToastKind::Info, message)
}

fn error(message: impl Into<String>) -> Intent {
    toast(ToastKind::Error, message)
}

pub(super) fn avatar_url_sized(base: &str, size: u32) -> Option<String> {
    let base = base.trim();
    if base.is_empty() {
        return None;
    }
    let (path, query) = base.split_once('?').unwrap_or((base, ""));
    let mut parts: Vec<&str> = query
        .split('&')
        .filter(|part| !part.is_empty() && !part.starts_with("s="))
        .collect();
    let size = format!("s={size}");
    parts.push(&size);
    Some(format!("{path}?{}", parts.join("&")))
}

impl ReviewController {
    pub fn presentation(&self) -> &ReviewPresentation {
        &self.presentation
    }

    pub fn shared_presentation(&self) -> Rc<ReviewPresentation> {
        Rc::clone(&self.presentation)
    }

    pub fn set_syntax_wake(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.host.set_wake(wake);
    }

    pub fn account(&self) -> &Account {
        &self.account
    }

    pub fn active_pull_request(&self) -> Option<&ActivePullRequest> {
        self.pr.as_ref()
    }

    pub fn session(&self) -> Option<&ReviewSession> {
        self.pr.as_ref()?.session.as_ref()
    }

    pub fn review_counts(&self) -> Option<(usize, usize, usize)> {
        let session = self.session()?;
        let composing = self.composer.as_ref().and_then(|c| c.target.draft());
        let mut pending = 0;
        let mut failed = 0;
        for draft in session.drafts.values() {
            if Some(draft.id) == composing || draft.body.trim().is_empty() {
                continue;
            }
            match draft.state {
                ReviewDraftState::Pending => pending += 1,
                ReviewDraftState::Failed(_) => failed += 1,
                ReviewDraftState::Submitting | ReviewDraftState::Submitted { .. } => {}
            }
        }
        Some((session.unresolved_thread_count(), pending, failed))
    }

    pub fn overlay_open(&self) -> Option<ReviewOverlay> {
        self.overlay
    }

    pub fn composer(&self) -> Option<&Composer> {
        self.composer.as_ref()
    }

    pub(super) fn signed_in(&self) -> bool {
        self.account
            .token
            .as_ref()
            .is_some_and(|token| !token.is_empty())
    }

    fn token(&self) -> Option<Secret> {
        self.account.token.clone().filter(|token| !token.is_empty())
    }

    fn job(&mut self, pr: Option<PullRequestRef>, kind: Pending) -> ReviewScope {
        self.next_job += 1;
        let request = JobId(self.next_job);
        self.in_flight.insert(request, InFlight { pr, kind });
        ReviewScope {
            account: self.epoch,
            request,
        }
    }

    fn github(&mut self, kind: Pending, request: GitHubRequest) -> ReviewEffect {
        let pr = self.pr.as_ref().map(|pr| pr.pr.clone());
        let scope = self.job(pr, kind);
        ReviewEffect::GitHub {
            scope,
            request: Box::new(request),
        }
    }

    fn accept(&mut self, scope: ReviewScope, keep: bool) -> Option<Pending> {
        if scope.account != self.epoch {
            self.in_flight.remove(&scope.request);
            return None;
        }
        let entry = if keep {
            self.in_flight.get(&scope.request).cloned()
        } else {
            self.in_flight.remove(&scope.request)
        }?;
        match &entry.pr {
            Some(pr) if self.pr.as_ref().map(|active| &active.pr) != Some(pr) => None,
            _ => Some(entry.kind),
        }
    }

    fn require_token(&mut self, why: &str) -> Result<Secret, Reduction> {
        match self.token() {
            Some(token) => Ok(token),
            None => {
                self.overlay = Some(ReviewOverlay::Auth);
                Err(Reduction::changed().intent(info(why)))
            }
        }
    }

    fn bump_account(&mut self) -> Intent {
        self.epoch = AccountEpoch(self.epoch.0 + 1);
        self.in_flight.clear();
        Intent::AccountChanged
    }

    fn save_session(&mut self) -> Option<ReviewEffect> {
        let active = self.pr.as_mut()?;
        active.session_dirty = false;
        let session = Box::new(active.session.clone()?);
        let pr = Some(active.pr.clone());
        let scope = self.job(pr, Pending::Session);
        Some(ReviewEffect::SaveSession { scope, session })
    }

    fn flush_session(&mut self) -> Option<ReviewEffect> {
        if self.pr.as_ref().is_some_and(|pr| pr.session_dirty) {
            self.save_session()
        } else {
            None
        }
    }

    fn session_mut(&mut self) -> Option<&mut ReviewSession> {
        self.pr.as_mut()?.session.as_mut()
    }

    fn fetch_review(&mut self) -> Option<ReviewEffect> {
        let pr = self.pr.as_ref()?.pr.clone();
        let token = self.token();
        Some(match token {
            Some(token) => self.github(
                Pending::ReviewData,
                GitHubRequest::FetchReviewData {
                    pr,
                    token: Some(token),
                },
            ),
            None => self.github(
                Pending::ReviewComments,
                GitHubRequest::FetchReviewComments { pr, token: None },
            ),
        })
    }

    fn startup(&mut self, startup: &crate::contracts::shell::AppStartup) -> Reduction {
        self.client_id = startup.options.github_client_id.clone();
        self.account.user = startup.settings.github_user.clone();
        self.pending_url = startup.options.args.open_pr.clone();
        if self.repo_path.is_none() {
            self.repo_path = startup.options.args.repo.clone();
        }
        let mut reduction = Reduction::changed();
        match startup.options.github_token.clone() {
            Some(token) => {
                self.account.token = Some(Secret::new(token));
                self.account.token_loaded = true;
                reduction.merge(self.fetch_user());
                reduction.merge(self.load_pending());
            }
            None => reduction = reduction.effect(ReviewEffect::LoadToken),
        }
        reduction
    }

    fn fetch_user(&mut self) -> Reduction {
        let Some(token) = self.token() else {
            return Reduction::default();
        };
        let scope = self.job(None, Pending::User);
        Reduction::default().effect(ReviewEffect::GitHub {
            scope,
            request: Box::new(GitHubRequest::FetchUser { token }),
        })
    }

    fn load_pending(&mut self) -> Reduction {
        if !self.account.token_loaded {
            return Reduction::default();
        }
        let (Some(url), Some(repo_path)) = (self.pending_url.clone(), self.repo_path.clone())
        else {
            return Reduction::default();
        };
        let Some(parsed) = parse_pr_url(&url) else {
            self.pending_url = None;
            return Reduction::changed().intent(error("Not a valid GitHub pull request URL."));
        };
        self.pending_url = None;
        let pr = PullRequestRef {
            owner: parsed.owner,
            repo: parsed.repo,
            number: parsed.number,
        };
        let mut reduction = Reduction::changed();
        if self.pr.as_ref().is_some_and(|active| active.pr != pr) {
            reduction.merge(self.close_pull_request());
        }
        if self.pr.is_none() {
            self.pr = Some(ActivePullRequest {
                pr,
                url: url.clone(),
                comparison: None,
                session: None,
                loading: true,
                error: None,
                session_dirty: false,
            });
        } else if let Some(active) = self.pr.as_mut() {
            active.loading = true;
            active.error = None;
            active.url = url.clone();
        }
        let token = self.token();
        reduction.effect(self.github(
            Pending::LoadPullRequest,
            GitHubRequest::LoadPullRequest {
                url,
                repo_path,
                token,
            },
        ))
    }

    fn close_pull_request(&mut self) -> Reduction {
        let mut reduction = Reduction::changed();
        if let Some(effect) = self.flush_session() {
            reduction = reduction.effect(effect);
        }
        self.pr = None;
        self.composer = None;
        self.expanded.clear();
        self.in_flight.retain(|_, entry| entry.pr.is_none());
        reduction.intent(Intent::SetReviewTarget(None))
    }

    fn publish(&self, cx: &SharedContext, compare: bool) -> Reduction {
        let Some(active) = self.pr.as_ref() else {
            return Reduction::default();
        };
        let Some(comparison) = active.comparison.as_ref() else {
            return Reduction::default();
        };
        let mut reduction =
            Reduction::changed().intent(Intent::SetReviewTarget(Some(Rc::new(ReviewContext {
                account: self.epoch,
                target: active.target(),
                pull_request: Arc::new(comparison.info.clone()),
                left_ref: comparison.left_ref.clone(),
                right_ref: comparison.right_ref.clone(),
            }))));
        if compare && let Some(repo_path) = self.repo_path.clone() {
            reduction = reduction.intent(Intent::RequestCompare(CompareIntent {
                repo_path,
                request: VcsCompareRequest {
                    spec: VcsCompareSpec::MergeBaseRange {
                        base: comparison.left_ref.clone(),
                        head: comparison.right_ref.clone(),
                    },
                    layout: cx.settings.settings.viewport.layout,
                    renderer: RendererKind::default(),
                },
            }));
        }
        reduction
    }

    fn set_comparison(
        &mut self,
        comparison: PullRequestComparison,
        cx: &SharedContext,
    ) -> Reduction {
        let Some(active) = self.pr.as_mut() else {
            return Reduction::default();
        };
        let refs_changed = active.comparison.as_ref().is_none_or(|current| {
            current.left_ref != comparison.left_ref || current.right_ref != comparison.right_ref
        });
        active.comparison = Some(comparison);
        self.publish(cx, refs_changed)
    }

    fn pull_request_loaded(
        &mut self,
        comparison: PullRequestComparison,
        cx: &SharedContext,
    ) -> Reduction {
        let info = comparison.info.clone();
        let mut reduction = self.set_comparison(comparison, cx);
        let Some(active) = self.pr.as_mut() else {
            return reduction;
        };
        active.loading = false;
        active.error = None;
        let target = active.target();
        match active.session.as_mut() {
            Some(session) => {
                session.pull_request = info.clone();
                session.status = ReviewSessionStatus::Loading;
            }
            None => {
                let mut session = ReviewSession::new(target.clone(), info.clone());
                session.status = ReviewSessionStatus::Loading;
                active.session = Some(session);
            }
        }
        let pr = Some(active.pr.clone());
        let scope = self.job(pr, Pending::Session);
        reduction = reduction.effect(ReviewEffect::LoadSession {
            scope,
            target,
            pull_request: Box::new(info),
        });
        if let Some(effect) = self.fetch_review() {
            reduction = reduction.effect(effect);
        }
        reduction
    }

    fn sign_out(&mut self, cx: &SharedContext) -> Reduction {
        let mut reduction = Reduction::changed();
        if self.pr.is_some() {
            reduction.merge(self.close_pull_request());
        }
        self.account = Account {
            token_loaded: true,
            ..Account::default()
        };
        self.overlay = None;
        reduction = reduction
            .effect(ReviewEffect::ClearToken)
            .intent(self.bump_account())
            .intent(info("Signed out of GitHub."));
        reduction.merge(self.publish_user(cx, None));
        reduction
    }

    fn publish_user(&self, cx: &SharedContext, user: Option<GitHubUser>) -> Reduction {
        if cx.settings.settings.github_user == user {
            return Reduction::default();
        }
        let mut settings = cx.settings.settings.clone();
        settings.github_user = user;
        Reduction::default().intent(Intent::SettingsChanged(Rc::new(SettingsSnapshot {
            revision: cx.settings.revision + 1,
            settings,
        })))
    }

    fn open_composer(&mut self, target: ComposerTarget, body: &str) -> Reduction {
        self.composer = Some(Composer {
            target,
            preview: false,
        });
        self.host.reset_composer(body);
        self.focus_request = Some(LogicalFocus::ReviewComposer);
        Reduction::changed()
    }

    fn close_composer(&mut self) {
        self.composer = None;
        self.host.clear_composer();
        self.focus_request = Some(LogicalFocus::Viewport);
    }

    fn open_composer_at(&mut self, target: SourceTarget, cx: &SharedContext) -> Reduction {
        if let Err(reduction) = self.require_token("Sign in to add review comments.") {
            return reduction;
        }
        if self.session().is_none() {
            return Reduction::changed().intent(error("Open a pull request to comment on it."));
        }
        let path = cx.manifest.as_ref().and_then(|manifest| {
            let file = manifest.files.iter().find(|file| file.key == target.file)?;
            let path = match target.side {
                crate::contracts::shared::Side::Old => file.paths.old_path(),
                crate::contracts::shared::Side::New => file.paths.new_path(),
            }
            .or_else(|| Some(file.paths.display_path()))?;
            Some(path.into_owned())
        });
        let Some(path) = path.filter(|path| !path.is_empty()) else {
            return Reduction::changed().intent(error("Select a line in the diff to comment on."));
        };
        self.open_composer(ComposerTarget::Inline { path, target }, "")
    }

    fn reply_to(&mut self, thread: ReviewThreadId) -> Reduction {
        if let Err(reduction) = self.require_token("Sign in to reply to review threads.") {
            return reduction;
        }
        let Some(session) = self.session_mut() else {
            return Reduction::default();
        };
        let existing = session.drafts.values().find(|draft| {
            matches!(&draft.kind, ReviewDraftKind::Reply { thread_id } if *thread_id == thread)
                && !matches!(draft.state, ReviewDraftState::Submitting)
        });
        let (draft, body) = match existing {
            Some(draft) => (draft.id, draft.body.clone()),
            None => (
                session.create_reply_draft(thread.clone(), ""),
                String::new(),
            ),
        };
        self.expanded.insert(thread.clone(), true);
        self.open_composer(ComposerTarget::Reply { thread, draft }, &body)
    }

    fn edit_comment(&mut self, comment_node_id: String) -> Reduction {
        if let Err(reduction) = self.require_token("Sign in to edit review comments.") {
            return reduction;
        }
        let Some(session) = self.session() else {
            return Reduction::default();
        };
        let found = session.threads.iter().find_map(|thread| {
            let comment = thread
                .comments
                .iter()
                .find(|c| c.backend_node_id.as_deref() == Some(comment_node_id.as_str()))?;
            Some((thread.id.clone(), comment.body.clone()))
        });
        let Some((thread, body)) = found else {
            return Reduction::changed().intent(error("This comment can no longer be edited."));
        };
        self.expanded.insert(thread.clone(), true);
        self.open_composer(
            ComposerTarget::Edit {
                thread,
                comment_node_id,
            },
            &body,
        )
    }

    fn edit_draft(&mut self, id: ReviewDraftId) -> Reduction {
        let Some(draft) = self.session().and_then(|session| session.drafts.get(&id)) else {
            return Reduction::default();
        };
        let body = draft.body.clone();
        let target = match &draft.kind {
            ReviewDraftKind::Reply { thread_id } => ComposerTarget::Reply {
                thread: thread_id.clone(),
                draft: id,
            },
            ReviewDraftKind::InlineComment | ReviewDraftKind::General => ComposerTarget::Draft(id),
        };
        self.open_composer(target, &body)
    }

    fn delete_draft(&mut self, id: ReviewDraftId) -> Reduction {
        let Some(session) = self.session_mut() else {
            return Reduction::default();
        };
        if session.remove_draft(id).is_none() {
            return Reduction::default();
        }
        if self
            .composer
            .as_ref()
            .is_some_and(|c| c.target.draft() == Some(id))
        {
            self.close_composer();
        }
        let mut reduction = Reduction::changed();
        if let Some(effect) = self.save_session() {
            reduction = reduction.effect(effect);
        }
        reduction
    }

    fn cancel_comment(&mut self) -> Reduction {
        let Some(target) = self.composer.as_ref().map(|c| c.target.clone()) else {
            return Reduction::default();
        };
        self.close_composer();
        let mut reduction = Reduction::changed();
        if let ComposerTarget::Reply { draft, .. } = target
            && let Some(session) = self.session_mut()
            && session
                .drafts
                .get(&draft)
                .is_some_and(|d| matches!(d.state, ReviewDraftState::Pending))
        {
            session.remove_draft(draft);
            if let Some(effect) = self.save_session() {
                reduction = reduction.effect(effect);
            }
        }
        reduction
    }

    fn submit_comment(&mut self) -> Reduction {
        let Some(composer) = self.composer.clone() else {
            return Reduction::default();
        };
        let body = self.host.composer_text().trim().to_owned();
        if body.is_empty() {
            return Reduction::changed().intent(error("Write a comment before submitting."));
        }
        let token = match self.require_token("Sign in to add review comments.") {
            Ok(token) => token,
            Err(reduction) => return reduction,
        };
        let Some(pr) = self.pr.as_ref().map(|active| active.pr.clone()) else {
            return Reduction::default();
        };
        let mut reduction = Reduction::changed();
        match composer.target {
            ComposerTarget::Inline { path, target } => {
                let anchor = ReviewAnchor::from_source_target(path, &target);
                if let Some(session) = self.session_mut() {
                    session.create_inline_draft(anchor, body);
                }
                reduction = reduction.intent(info("Review comment staged."));
            }
            ComposerTarget::General => {
                if let Some(session) = self.session_mut() {
                    session.create_general_draft(body);
                }
                reduction = reduction.intent(info("Review comment staged."));
            }
            ComposerTarget::Draft(id) => {
                if let Some(session) = self.session_mut() {
                    session.update_draft_body(id, body);
                }
            }
            ComposerTarget::Reply { thread, draft } => {
                let Some(node) = self.session().and_then(|s| s.thread_node_id(&thread)) else {
                    return reduction.intent(error("This review thread cannot be replied to."));
                };
                if let Some(session) = self.session_mut() {
                    session.update_draft_body(draft, body.clone());
                    session.mark_draft_submitting(draft);
                }
                reduction = reduction.effect(self.github(
                    Pending::Reply(draft),
                    GitHubRequest::AddThreadReply {
                        pr,
                        thread_node_id: node,
                        review_node_id: None,
                        token: Some(token),
                        body,
                    },
                ));
            }
            ComposerTarget::Edit {
                comment_node_id, ..
            } => {
                reduction = reduction.effect(self.github(
                    Pending::Mutation,
                    GitHubRequest::UpdateCommentGraphql {
                        pr,
                        comment_node_id,
                        token: Some(token),
                        body,
                    },
                ));
            }
        }
        self.close_composer();
        if let Some(effect) = self.save_session() {
            reduction = reduction.effect(effect);
        }
        reduction
    }

    fn submit_review(&mut self, decision: ReviewDecision) -> Reduction {
        let token = match self.require_token("Sign in to submit a review.") {
            Ok(token) => token,
            Err(reduction) => return reduction,
        };
        let Some(pr) = self.pr.as_ref().map(|active| active.pr.clone()) else {
            return Reduction::default();
        };
        let Some(session) = self.session_mut() else {
            return Reduction::default();
        };
        let failed: Vec<ReviewDraftId> = session
            .drafts
            .values()
            .filter(|draft| matches!(draft.state, ReviewDraftState::Failed(_)))
            .map(|draft| draft.id)
            .collect();
        for id in failed {
            if let Some(draft) = session.drafts.get_mut(&id) {
                draft.state = ReviewDraftState::Pending;
            }
        }
        let pending: Vec<(ReviewDraftId, ReviewDraftKind, String)> = session
            .pending_drafts()
            .filter(|draft| !draft.body.trim().is_empty())
            .map(|draft| (draft.id, draft.kind.clone(), draft.body.clone()))
            .collect();
        let batch: Vec<ReviewDraftId> = pending
            .iter()
            .filter(|(_, kind, _)| !matches!(kind, ReviewDraftKind::Reply { .. }))
            .map(|(id, _, _)| *id)
            .collect();
        let replies: Vec<(ReviewDraftId, ReviewThreadId, String)> = pending
            .iter()
            .filter_map(|(id, kind, body)| match kind {
                ReviewDraftKind::Reply { thread_id } => {
                    Some((*id, thread_id.clone(), body.clone()))
                }
                _ => None,
            })
            .collect();
        if batch.is_empty() && replies.is_empty() && decision == ReviewDecision::Comment {
            return Reduction::changed().intent(info("There are no pending comments to submit."));
        }
        let mut submission = session.clone();
        submission.drafts.retain(|id, _| batch.contains(id));
        for id in batch.iter().chain(replies.iter().map(|(id, _, _)| id)) {
            session.mark_draft_submitting(*id);
        }
        session.status = ReviewSessionStatus::Loading;
        session.status_message = None;
        let nodes: Vec<Option<String>> = replies
            .iter()
            .map(|(_, thread, _)| session.thread_node_id(thread))
            .collect();
        let mut reduction = Reduction::changed();
        if !batch.is_empty() || replies.is_empty() {
            reduction = reduction.effect(self.github(
                Pending::SubmitDrafts,
                GitHubRequest::SubmitSessionDrafts {
                    session: Box::new(submission),
                    decision,
                    body: None,
                    token: Some(token.clone()),
                },
            ));
        }
        for ((draft, _, body), node) in replies.into_iter().zip(nodes) {
            match node {
                Some(thread_node_id) => {
                    reduction = reduction.effect(self.github(
                        Pending::Reply(draft),
                        GitHubRequest::AddThreadReply {
                            pr: pr.clone(),
                            thread_node_id,
                            review_node_id: None,
                            token: Some(token.clone()),
                            body,
                        },
                    ));
                }
                None => {
                    if let Some(session) = self.session_mut() {
                        session
                            .mark_draft_failed(draft, "This review thread cannot be replied to.");
                    }
                }
            }
        }
        if let Some(effect) = self.save_session() {
            reduction = reduction.effect(effect);
        }
        reduction
    }

    fn set_thread_resolved(&mut self, id: ReviewThreadId, resolved: bool) -> Reduction {
        let token = match self.require_token("Sign in to update review threads.") {
            Ok(token) => token,
            Err(reduction) => return reduction,
        };
        let Some(pr) = self.pr.as_ref().map(|active| active.pr.clone()) else {
            return Reduction::default();
        };
        let Some(node) = self.session().and_then(|s| s.thread_node_id(&id)) else {
            return Reduction::changed().intent(error("This review thread cannot be updated."));
        };
        Reduction::default().effect(self.github(
            Pending::Mutation,
            GitHubRequest::SetThreadResolution {
                pr,
                thread_node_id: node,
                token: Some(token),
                resolved,
            },
        ))
    }

    fn delete_comment(&mut self, comment_node_id: String) -> Reduction {
        let token = match self.require_token("Sign in to delete review comments.") {
            Ok(token) => token,
            Err(reduction) => return reduction,
        };
        let Some(pr) = self.pr.as_ref().map(|active| active.pr.clone()) else {
            return Reduction::default();
        };
        Reduction::default().effect(self.github(
            Pending::Mutation,
            GitHubRequest::DeleteCommentGraphql {
                pr,
                comment_node_id,
                token: Some(token),
            },
        ))
    }

    fn discard_drafts(&mut self) -> Reduction {
        let Some(session) = self.session_mut() else {
            return Reduction::default();
        };
        let ids: Vec<ReviewDraftId> = session
            .drafts
            .values()
            .filter(|draft| !matches!(draft.state, ReviewDraftState::Submitting))
            .map(|draft| draft.id)
            .collect();
        for id in &ids {
            session.remove_draft(*id);
        }
        if self
            .composer
            .as_ref()
            .and_then(|c| c.target.draft())
            .is_some_and(|draft| ids.contains(&draft))
        {
            self.close_composer();
        }
        let mut reduction = Reduction::changed();
        if let Some(effect) = self.save_session() {
            reduction = reduction.effect(effect);
        }
        reduction
    }

    fn toggle_thread(&mut self, id: ReviewThreadId) -> Reduction {
        let expanded = self.thread_expanded(&id);
        self.expanded.insert(id, !expanded);
        Reduction::changed()
    }

    pub(super) fn thread_expanded(&self, id: &ReviewThreadId) -> bool {
        if let Some(expanded) = self.expanded.get(id) {
            return *expanded;
        }
        self.session()
            .and_then(|session| session.threads.iter().find(|thread| &thread.id == id))
            .is_none_or(|thread| thread.status.resolution != ReviewResolution::Resolved)
    }

    fn format(&mut self, format: ComposerFormat) -> Reduction {
        if self.composer.as_ref().is_none_or(|c| c.preview) {
            return Reduction::default();
        }
        self.host.format_composer(format);
        self.sync_composer_draft();
        self.focus_request = Some(LogicalFocus::ReviewComposer);
        Reduction::changed()
    }

    fn sync_composer_draft(&mut self) {
        let Some(draft) = self.composer.as_ref().and_then(|c| match c.target {
            ComposerTarget::Reply { draft, .. } => Some(draft),
            _ => None,
        }) else {
            return;
        };
        let body = self.host.composer_text();
        if let Some(active) = self.pr.as_mut()
            && let Some(session) = active.session.as_mut()
            && let Some(entry) = session.drafts.get_mut(&draft)
            && entry.body != body
        {
            entry.body = body;
            active.session_dirty = true;
        }
    }

    fn device_flow(&mut self) -> Reduction {
        self.next_job += 1;
        let flow = JobId(self.next_job);
        self.account.flow = Some(flow);
        self.account.auth = AuthStatus::Starting;
        self.overlay = Some(ReviewOverlay::Auth);
        Reduction::changed().effect(ReviewEffect::StartDeviceFlow {
            flow,
            client_id: self.client_id.clone(),
        })
    }

    fn reduce_github(
        &mut self,
        scope: ReviewScope,
        response: GitHubResponse,
        cx: &SharedContext,
    ) -> Reduction {
        let keep = matches!(response, GitHubResponse::PullRequestCached { .. });
        let Some(kind) = self.accept(scope, keep) else {
            return Reduction::default();
        };
        match response {
            GitHubResponse::PullRequestCached { comparison, .. } => {
                if self.pr.as_ref().is_some_and(|pr| pr.comparison.is_none()) {
                    self.set_comparison(comparison, cx)
                } else {
                    Reduction::default()
                }
            }
            GitHubResponse::PullRequestLoaded { comparison, .. } => {
                self.pull_request_loaded(comparison, cx)
            }
            GitHubResponse::PullRequestLoadFailed { message, .. } => {
                if let Some(active) = self.pr.as_mut() {
                    active.loading = false;
                    active.error = Some(message.clone());
                }
                Reduction::changed().intent(error(message))
            }
            GitHubResponse::UserFetched(user) => {
                let previous = self.account.user.as_ref().map(|u| u.login.clone());
                let mut reduction = Reduction::changed();
                if previous.is_some_and(|login| login != user.login) && self.pr.is_some() {
                    reduction.merge(self.close_pull_request());
                    reduction = reduction.intent(self.bump_account());
                }
                if let Some(url) = avatar_url_sized(&user.avatar_url, ACCOUNT_AVATAR_PX)
                    && self.account.avatar_url.as_deref() != Some(url.as_str())
                {
                    self.account.avatar_url = Some(url.clone());
                    self.account.avatar = None;
                    reduction = reduction.effect(ReviewEffect::FetchAvatar { url });
                }
                reduction.merge(self.publish_user(cx, Some(user.clone())));
                self.account.user = Some(user);
                reduction
            }
            GitHubResponse::UserFetchFailed { message } => {
                tracing::warn!("failed to fetch GitHub user: {message}");
                Reduction::default()
            }
            GitHubResponse::ReviewDataLoaded { data, .. } => {
                if let Some(session) = self.session_mut() {
                    session.apply_github_review_data(*data);
                }
                let mut reduction = Reduction::changed();
                if let Some(effect) = self.save_session() {
                    reduction = reduction.effect(effect);
                }
                reduction
            }
            GitHubResponse::ReviewDataLoadFailed { pr, message } => {
                tracing::warn!("failed to fetch pull request review data: {message}");
                if let Some(session) = self.session_mut() {
                    session.status_message = Some(message);
                }
                Reduction::changed().effect(self.github(
                    Pending::ReviewComments,
                    GitHubRequest::FetchReviewComments {
                        pr,
                        token: self.token(),
                    },
                ))
            }
            GitHubResponse::ReviewCommentsLoaded { comments, .. } => {
                let mut reduction = Reduction::changed();
                if let Some(session) = self.session_mut()
                    && session.threads.iter().all(|t| t.backend_node_id.is_none())
                {
                    session.replace_github_comments(comments);
                    if let Some(effect) = self.save_session() {
                        reduction = reduction.effect(effect);
                    }
                }
                reduction
            }
            GitHubResponse::ReviewCommentsLoadFailed { message, .. } => {
                if let Some(session) = self.session_mut() {
                    session.status = ReviewSessionStatus::Failed;
                    session.status_message = Some(message);
                }
                Reduction::changed()
            }
            GitHubResponse::ThreadReplyAdded { .. } => {
                if let Pending::Reply(draft) = kind
                    && let Some(session) = self.session_mut()
                {
                    session.remove_draft(draft);
                    if session
                        .drafts
                        .values()
                        .all(|d| !matches!(d.state, ReviewDraftState::Submitting))
                    {
                        session.status = ReviewSessionStatus::Ready;
                    }
                }
                self.after_mutation(true)
            }
            GitHubResponse::ThreadReplyAddFailed { message, .. } => {
                if let Pending::Reply(draft) = kind
                    && let Some(session) = self.session_mut()
                {
                    session.mark_draft_failed(draft, message.clone());
                    session.status = ReviewSessionStatus::Failed;
                    session.status_message = Some(message.clone());
                }
                let mut reduction = self.after_mutation(false);
                reduction = reduction.intent(error(message));
                reduction
            }
            GitHubResponse::DraftsSubmitted { draft_ids, .. } => {
                if let Some(session) = self.session_mut() {
                    for id in &draft_ids {
                        session.remove_draft(*id);
                    }
                    if session
                        .drafts
                        .values()
                        .all(|d| !matches!(d.state, ReviewDraftState::Submitting))
                    {
                        session.status = ReviewSessionStatus::Ready;
                        session.status_message = None;
                    }
                }
                self.after_mutation(true).intent(info("Review submitted."))
            }
            GitHubResponse::DraftsSubmitFailed {
                draft_ids, message, ..
            } => {
                if let Some(session) = self.session_mut() {
                    for id in &draft_ids {
                        session.mark_draft_failed(*id, message.clone());
                    }
                    session.status = ReviewSessionStatus::Failed;
                    session.status_message = Some(message.clone());
                }
                self.after_mutation(false).intent(error(message))
            }
            GitHubResponse::ThreadResolutionChanged { resolution, .. } => {
                let id = ReviewThreadId::github_node(resolution.thread_node_id);
                if let Some(session) = self.session_mut() {
                    session.mark_thread_resolution(
                        &id,
                        if resolution.is_resolved {
                            ReviewResolution::Resolved
                        } else {
                            ReviewResolution::Unresolved
                        },
                    );
                }
                self.expanded.remove(&id);
                self.after_mutation(false)
            }
            GitHubResponse::CommentGraphqlUpdated { .. }
            | GitHubResponse::CommentGraphqlDeleted { .. }
            | GitHubResponse::ReviewCommentCreated { .. }
            | GitHubResponse::ReviewCommentReplied { .. }
            | GitHubResponse::ReviewCommentUpdated { .. }
            | GitHubResponse::ReviewCommentDeleted { .. }
            | GitHubResponse::ReviewCreated { .. }
            | GitHubResponse::ReviewSubmitted { .. } => self.after_mutation(true),
            GitHubResponse::CommentGraphqlUpdateFailed { message, .. }
            | GitHubResponse::CommentGraphqlDeleteFailed { message, .. }
            | GitHubResponse::ThreadResolutionChangeFailed { message, .. }
            | GitHubResponse::ReviewCommentCreateFailed { message, .. }
            | GitHubResponse::ReviewCommentReplyFailed { message, .. }
            | GitHubResponse::ReviewCommentUpdateFailed { message, .. }
            | GitHubResponse::ReviewCommentDeleteFailed { message, .. }
            | GitHubResponse::ReviewCreateFailed { message, .. }
            | GitHubResponse::ReviewSubmitFailed { message, .. } => {
                Reduction::changed().intent(error(message))
            }
            GitHubResponse::PullRequestPeeked { .. }
            | GitHubResponse::PullRequestPeekFailed { .. } => Reduction::default(),
        }
    }

    fn after_mutation(&mut self, refetch: bool) -> Reduction {
        let mut reduction = Reduction::changed();
        if let Some(effect) = self.save_session() {
            reduction = reduction.effect(effect);
        }
        if refetch && let Some(effect) = self.fetch_review() {
            reduction = reduction.effect(effect);
        }
        reduction
    }

    fn reduce(&mut self, action: ReviewAction, cx: &SharedContext) -> Reduction {
        match action {
            ReviewAction::OpenAuthModal => {
                self.overlay = Some(ReviewOverlay::Auth);
                if self.account.auth == AuthStatus::Done && !self.signed_in() {
                    self.account.auth = AuthStatus::Idle;
                }
                Reduction::changed()
            }
            ReviewAction::CloseOverlay => {
                self.overlay = None;
                Reduction::changed()
            }
            ReviewAction::StartDeviceFlow => self.device_flow(),
            ReviewAction::OpenDeviceFlowBrowser => match &self.account.auth {
                AuthStatus::Awaiting(flow) => {
                    Reduction::default().effect(ShellEffect::OpenBrowser {
                        url: flow.verification_uri.clone(),
                    })
                }
                _ => Reduction::default(),
            },
            ReviewAction::OpenAccountMenu => {
                self.overlay = Some(ReviewOverlay::AccountMenu);
                Reduction::changed()
            }
            ReviewAction::SignOut => self.sign_out(cx),
            ReviewAction::LoadPullRequest { url } => {
                self.pending_url = Some(url);
                let reduction = self.load_pending();
                if self.pending_url.is_some() && self.repo_path.is_none() {
                    return reduction.intent(info("Open a repository to load the pull request."));
                }
                reduction
            }
            ReviewAction::PreviewPullRequest => Reduction::default(),
            ReviewAction::OpenPullRequestInBrowser => match self.pr.as_ref() {
                Some(active) => Reduction::default().effect(ShellEffect::OpenBrowser {
                    url: active.web_url(),
                }),
                None => Reduction::default(),
            },
            ReviewAction::OpenComposer => {
                if let Err(reduction) = self.require_token("Sign in to add review comments.") {
                    return reduction;
                }
                if self.session().is_none() {
                    return Reduction::default();
                }
                self.open_composer(ComposerTarget::General, "")
            }
            ReviewAction::OpenComposerAt(target) => self.open_composer_at(target, cx),
            ReviewAction::SubmitComment => self.submit_comment(),
            ReviewAction::CancelComment => self.cancel_comment(),
            ReviewAction::FormatComment(format) => self.format(format),
            ReviewAction::SetComposerPreview(preview) => match self.composer.as_mut() {
                Some(composer) if composer.preview != preview => {
                    composer.preview = preview;
                    if !preview {
                        self.focus_request = Some(LogicalFocus::ReviewComposer);
                    }
                    Reduction::changed()
                }
                _ => Reduction::default(),
            },
            ReviewAction::ReplyToThread(thread) => self.reply_to(thread),
            ReviewAction::EditComment { comment_node_id } => self.edit_comment(comment_node_id),
            ReviewAction::DeleteComment { comment_node_id } => self.delete_comment(comment_node_id),
            ReviewAction::SubmitReview { decision } => self.submit_review(decision),
            ReviewAction::DiscardDrafts => self.discard_drafts(),
            ReviewAction::ToggleThread(id) => self.toggle_thread(id),
            ReviewAction::SetThreadResolved { id, resolved } => {
                self.set_thread_resolved(id, resolved)
            }
            ReviewAction::EditDraft(id) => self.edit_draft(id),
            ReviewAction::DeleteDraft(id) => self.delete_draft(id),
            ReviewAction::CardDocument { card, event } => {
                if self.host.handle_document(&card, event) {
                    Reduction::changed()
                } else {
                    Reduction::default()
                }
            }
            ReviewAction::CopyCardSelection(card) => match self.host.selected_text(&card) {
                Some(text) => Reduction::default().effect(ShellEffect::SetClipboard(text)),
                None => Reduction::default(),
            },
            ReviewAction::ScrollComposer(lines) => {
                self.host.scroll_composer(lines);
                Reduction::changed()
            }
        }
    }

    fn event(&mut self, event: ReviewEvent, cx: &SharedContext) -> Reduction {
        match event {
            ReviewEvent::GitHub { scope, response } => self.reduce_github(scope, *response, cx),
            ReviewEvent::DeviceFlowStarted { flow, state } => {
                if self.account.flow != Some(flow) {
                    return Reduction::default();
                }
                let url = state.verification_uri.clone();
                let effect = ReviewEffect::PollDeviceFlow {
                    flow,
                    client_id: self.client_id.clone(),
                    device_code: state.device_code.clone(),
                    interval_seconds: state.interval,
                };
                self.account.auth = AuthStatus::Awaiting(state);
                Reduction::changed()
                    .effect(ShellEffect::OpenBrowser { url })
                    .effect(effect)
            }
            ReviewEvent::DeviceFlowStartFailed { flow, message }
            | ReviewEvent::DeviceFlowFailed { flow, message } => {
                if self.account.flow != Some(flow) {
                    return Reduction::default();
                }
                self.account.flow = None;
                self.account.auth = AuthStatus::Failed(message.clone());
                Reduction::changed().intent(error(message))
            }
            ReviewEvent::DeviceFlowCompleted { flow, token } => {
                if self.account.flow != Some(flow) {
                    return Reduction::default();
                }
                self.account.flow = None;
                self.account.auth = AuthStatus::Done;
                self.account.token = Some(token.clone());
                self.account.token_loaded = true;
                if self.overlay == Some(ReviewOverlay::Auth) {
                    self.overlay = None;
                }
                let mut reduction = Reduction::changed()
                    .effect(ReviewEffect::SaveToken(token))
                    .intent(self.bump_account())
                    .intent(info("GitHub authentication completed."));
                reduction.merge(self.fetch_user());
                reduction.merge(self.load_pending());
                if self.pr.as_ref().is_some_and(|pr| pr.comparison.is_some())
                    && let Some(effect) = self.fetch_review()
                {
                    reduction = reduction.effect(effect);
                }
                reduction
            }
            ReviewEvent::TokenLoaded { token } => {
                self.account.token_loaded = true;
                self.account.token = token.filter(|token| !token.is_empty());
                let mut reduction = Reduction::changed();
                reduction.merge(self.fetch_user());
                reduction.merge(self.load_pending());
                reduction
            }
            ReviewEvent::TokenLoadFailed { message } => {
                self.account.token_loaded = true;
                self.account.token = None;
                let mut reduction =
                    Reduction::changed().intent(error(format!("Keyring unavailable: {message}")));
                reduction.merge(self.load_pending());
                reduction
            }
            ReviewEvent::TokenSaveFailed { message } => Reduction::changed().intent(error(
                format!("Couldn't save GitHub token to keyring: {message}"),
            )),
            ReviewEvent::SessionLoaded {
                scope,
                target,
                session,
            } => {
                if self.accept(scope, false).is_none() {
                    return Reduction::default();
                }
                let Some(active) = self.pr.as_mut() else {
                    return Reduction::default();
                };
                if active.target() != target {
                    return Reduction::default();
                }
                match active.session.as_mut() {
                    Some(current) => current.merge_persisted_state(*session),
                    None => active.session = Some(*session),
                }
                Reduction::changed()
            }
            ReviewEvent::SessionLoadFailed { scope, message, .. } => {
                if self.accept(scope, false).is_some() {
                    tracing::warn!("failed to load review session: {message}");
                }
                Reduction::default()
            }
            ReviewEvent::SessionSaved { scope, .. } => {
                self.in_flight.remove(&scope.request);
                Reduction::default()
            }
            ReviewEvent::SessionSaveFailed { scope, message, .. } => {
                self.in_flight.remove(&scope.request);
                Reduction::default()
                    .intent(error(format!("Couldn't save review drafts: {message}")))
            }
            ReviewEvent::AvatarFetched { url, image } => {
                if self.account.avatar_url.as_deref() != Some(url.as_str()) {
                    return Reduction::default();
                }
                self.account.avatar = Some(AvatarImage {
                    rgba: Arc::from(image.rgba.as_slice()),
                    width: image.width,
                    height: image.height,
                    cache_key: avatar_cache_key(&url),
                });
                Reduction::changed()
            }
            ReviewEvent::AvatarFetchFailed { url, message } => {
                tracing::warn!("failed to fetch avatar {url}: {message}");
                Reduction::default()
            }
        }
    }

    fn intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        match intent {
            Intent::Startup(startup) => {
                self.epoch = cx.account;
                self.startup(startup)
            }
            Intent::RepositoryChanged(repository) => {
                let path = repository.snapshot.path.clone();
                if self.repo_path.as_ref() == Some(&path) && self.pending_url.is_none() {
                    return Reduction::default();
                }
                self.repo_path = Some(path);
                self.load_pending()
            }
            Intent::ActivateWorkspace { path } => {
                self.repo_path = Some(path.clone());
                if self.pr.is_some() {
                    let mut reduction = self.close_pull_request();
                    reduction
                        .intents
                        .retain(|i| !matches!(i, Intent::SetReviewTarget(_)));
                    return reduction;
                }
                Reduction::default()
            }
            Intent::AccountChanged => {
                self.epoch = cx.account;
                Reduction::default()
            }
            Intent::PublishManifest(_) => Reduction::changed(),
            _ => Reduction::default(),
        }
    }

    fn finish(&mut self, mut reduction: Reduction, cx: &SharedContext) -> Reduction {
        if let Some(focus) = self.focus_request.take() {
            reduction
                .effects
                .push(ShellEffect::Focus(Some(focus)).into());
        }
        if let Some(effect) = self.flush_session() {
            reduction.effects.push(effect.into());
        }
        if reduction.changed {
            self.refresh(cx);
        }
        reduction
    }

    pub(super) fn refresh(&mut self, cx: &SharedContext) {
        self.presentation = Rc::new(ReviewPresentation::build(self, cx));
    }

    pub(super) fn bump_presentation(&mut self) {
        self.presentation = Rc::new(self.presentation.bumped());
    }
}

pub(super) fn avatar_cache_key(url: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    "avatar".hash(&mut h);
    url.hash(&mut h);
    h.finish()
}

impl Controller for ReviewController {
    type Action = ReviewAction;
    type Event = ReviewEvent;

    fn reduce_action(&mut self, action: Self::Action, cx: &SharedContext) -> Reduction {
        let reduction = self.reduce(action, cx);
        self.finish(reduction, cx)
    }

    fn reduce_event(&mut self, event: Self::Event, cx: &SharedContext) -> Reduction {
        let reduction = self.event(event, cx);
        self.finish(reduction, cx)
    }

    fn apply_intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        let reduction = self.intent(intent, cx);
        self.finish(reduction, cx)
    }
}

impl TextHost for ReviewController {
    fn owns_text(&self, target: FocusId) -> bool {
        target == REVIEW_COMPOSER && self.composer.as_ref().is_some_and(|c| !c.preview)
    }

    fn edit_text(
        &mut self,
        target: FocusId,
        command: TextEditCommand,
        cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        if !self.owns_text(target) {
            return None;
        }
        let outcome = match command {
            TextEditCommand::ToggleStyle(style) => {
                let format = if style == InlineStyle::BOLD {
                    ComposerFormat::Bold
                } else if style == InlineStyle::ITALIC {
                    ComposerFormat::Italic
                } else if style == InlineStyle::CODE {
                    ComposerFormat::Code
                } else {
                    return Some(TextEditOutcome::default());
                };
                self.host.format_composer(format)
            }
            command => self.host.edit_composer(command, cx.now_ms),
        };
        if outcome.text_changed {
            self.sync_composer_draft();
        }
        if outcome.text_changed || outcome.selection_changed || outcome.preedit_changed {
            self.bump_presentation();
        }
        Some(outcome)
    }

    fn set_preedit(
        &mut self,
        target: FocusId,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> bool {
        if !self.owns_text(target) {
            return false;
        }
        self.host.composer_preedit(text, cursor);
        self.bump_presentation();
        true
    }

    fn set_text_value(
        &mut self,
        target: FocusId,
        value: String,
        _cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        if !self.owns_text(target) {
            return None;
        }
        self.host.reset_composer(&value);
        self.sync_composer_draft();
        self.bump_presentation();
        Some(TextEditOutcome {
            text_changed: true,
            ..TextEditOutcome::default()
        })
    }
}
