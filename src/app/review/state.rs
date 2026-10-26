use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use quark_components::AvatarImage;
use quark_ui::FocusId;
use quark_ui::element::ScrollHandle;
use quark_ui::text_input::{InlineStyle, TextEditCommand, TextEditOutcome};

use super::composer::ComposerTarget;
use super::presentation::{CardHost, ReviewPresentation};
use super::pulls::PullsPage;
use crate::app::text_targets::{REVIEW_COMPOSER, REVIEW_PULLS_FILTER, TextCx, TextHost};
use crate::contracts::commands::{
    CommandDescriptor, CommandSource, CommandsAction, SettingsSnapshot, ToastKind, ToastUpdate,
};
use crate::contracts::navigation::SelectionOrigin;
use crate::contracts::review::{
    CardKey, ComposerFormat, DraftCarry, GitHubRequest, GitHubResponse, HeadChange,
    PullRequestComparison, PullRequestRef, PullWatch, PullsAction, ReviewAction, ReviewContext,
    ReviewEffect, ReviewEvent, ReviewScope, Secret,
};
use crate::contracts::shared::{AccountEpoch, JobId, SharedContext, SourceTarget};
use crate::contracts::shell::{Controller, Intent, LogicalFocus, Reduction, ShellEffect};
use crate::contracts::viewport::CompareIntent;
use crate::core::compare::RendererKind;
use crate::core::forge::github::{DeviceFlowState, GitHubUser, parse_pr_url};
use crate::core::forge::github::{PullRequestFingerprint, PullRequestInfo, PullRequestOverview};
use crate::core::review::{
    ReviewAnchor, ReviewDecision, ReviewDraftId, ReviewDraftKind, ReviewDraftState,
    ReviewResolution, ReviewSession, ReviewSessionStatus, ReviewTarget, ReviewThread,
    ReviewThreadId,
};
use crate::core::syntax::grammar::Grammars;
use crate::core::vcs::git::WORKDIR_REF;
use crate::core::vcs::model::{RefKind, VcsCompareRequest, VcsCompareSpec, VcsKind, VcsRef};

const ACCOUNT_AVATAR_PX: u32 = 128;
pub(super) const REVIEW_AVATAR_PX: u32 = 64;
pub(super) const AVATAR_CACHE_LIMIT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewOverlay {
    Auth,
    AccountMenu,
    SubmitReview,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeekState {
    Loading,
    Ready(Box<PullRequestInfo>),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AvatarState {
    Fetching,
    Ready,
    Failed,
}

#[derive(Debug, Default)]
struct AvatarCache {
    entries: HashMap<String, (String, AvatarState)>,
    order: VecDeque<String>,
}

impl AvatarCache {
    fn contains(&self, url: &str) -> bool {
        self.entries.contains_key(url)
    }

    fn insert(&mut self, url: String, login: String) -> Vec<String> {
        self.order.push_back(url.clone());
        self.entries.insert(url, (login, AvatarState::Fetching));
        let mut evicted = Vec::new();
        while self.order.len() > AVATAR_CACHE_LIMIT {
            if let Some(old) = self.order.pop_front()
                && let Some((login, _)) = self.entries.remove(&old)
            {
                evicted.push(login);
            }
        }
        evicted
    }

    fn settle(&mut self, url: &str, ready: bool) -> Option<String> {
        let (login, state) = self.entries.get_mut(url)?;
        *state = if ready {
            AvatarState::Ready
        } else {
            AvatarState::Failed
        };
        Some(login.clone())
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
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
pub(super) enum Pending {
    LoadPullRequest,
    User,
    ReviewData,
    ReviewComments,
    Reply(ReviewDraftId),
    Mutation,
    SubmitDrafts,
    Session,
    Peek,
    Overview,
    Comment,
    Pulls,
    Watch,
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
    review_refetch: bool,
    /// A push the watch noticed that isn't loaded yet.
    pub update: Option<HeadChange>,
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

#[derive(Debug, Default)]
pub struct OverviewState {
    pub open: bool,
    pub data: Option<Rc<PullRequestOverview>>,
    pub loading: bool,
    pub error: Option<String>,
    pub checks_expanded: bool,
    pub scroll: ScrollHandle,
    request: Option<JobId>,
}

#[derive(Debug, Clone)]
pub struct Composer {
    pub target: ComposerTarget,
    pub preview: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteTarget {
    Comment(String),
    Draft(ReviewDraftId),
}

pub struct ReviewController {
    pub(super) confirm_delete: Option<DeleteTarget>,
    pub(super) comment_menu: Option<String>,
    epoch: AccountEpoch,
    client_id: String,
    pub(super) account: Account,
    pub(super) overlay: Option<ReviewOverlay>,
    pending_url: Option<String>,
    pub(super) repo_path: Option<PathBuf>,
    pub(super) pr: Option<ActivePullRequest>,
    next_job: u64,
    in_flight: HashMap<JobId, InFlight>,
    pub(super) expanded: HashMap<ReviewThreadId, bool>,
    pub(super) composer: Option<Composer>,
    pub(super) focus_request: Option<LogicalFocus>,
    pub(super) focused_card: Option<CardKey>,
    avatars: AvatarCache,
    peeks: HashMap<PullRequestRef, PeekState>,
    pub(super) file_comments: Option<String>,
    pub(super) overview: OverviewState,
    pub(super) pulls: PullsPage,
    pub(super) host: CardHost,
    presentation: Rc<ReviewPresentation>,
}

impl Default for ReviewController {
    fn default() -> Self {
        let host = CardHost::default();
        Self {
            confirm_delete: None,
            comment_menu: None,
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
            focused_card: None,
            avatars: AvatarCache::default(),
            peeks: HashMap::new(),
            file_comments: None,
            overview: OverviewState::default(),
            pulls: PullsPage::default(),
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

pub(super) fn info(message: impl Into<String>) -> Intent {
    toast(ToastKind::Info, message)
}

pub(super) fn error(message: impl Into<String>) -> Intent {
    toast(ToastKind::Error, message)
}

const NOREPLY_DOMAIN: &str = "@users.noreply.github.com";

fn author_face_key(email: &str) -> Option<String> {
    let email = email.trim().to_ascii_lowercase();
    let (local, _) = email.split_once('@')?;
    if local.is_empty() {
        return None;
    }
    Some(match email.strip_suffix(NOREPLY_DOMAIN) {
        Some(local) => local.rsplit('+').next().unwrap_or(local).to_owned(),
        None => format!("email:{email}"),
    })
}

fn saved_renderer(cx: &SharedContext) -> RendererKind {
    cx.settings
        .settings
        .last_compare
        .as_ref()
        .map(|compare| compare.renderer)
        .unwrap_or_default()
}

fn author_avatar_url(key: &str) -> Option<String> {
    let base = match key.strip_prefix("email:") {
        Some(email) => format!(
            "https://avatars.githubusercontent.com/u/e?email={}",
            percent_encode(email)
        ),
        None => format!("https://avatars.githubusercontent.com/{key}"),
    };
    avatar_url_sized(&base, REVIEW_AVATAR_PX)
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
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
    pub fn presentation(&self) -> Rc<ReviewPresentation> {
        Rc::clone(&self.presentation)
    }

    pub fn with_grammars(grammars: Grammars) -> Self {
        let controller = Self::default();
        controller.host.set_grammars(Arc::new(grammars));
        controller
    }

    pub fn set_syntax_wake(&mut self, wake: Option<Arc<dyn Fn() + Send + Sync>>) {
        if let Some(wake) = wake {
            self.host.set_wake(wake);
        }
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

    pub fn summary_staged(&self) -> bool {
        self.session().is_some_and(|session| {
            session.drafts.values().any(|draft| {
                matches!(draft.kind, ReviewDraftKind::General)
                    && matches!(draft.state, ReviewDraftState::Pending)
                    && !draft.body.trim().is_empty()
            })
        })
    }

    pub fn viewer_is_author(&self) -> bool {
        let user = self.account.user.as_ref();
        let author = self
            .pr
            .as_ref()
            .and_then(|pr| pr.comparison.as_ref())
            .map(|comparison| comparison.info.author_login.as_str());
        user.zip(author)
            .is_some_and(|(user, author)| user.login.eq_ignore_ascii_case(author))
    }

    pub fn peek(&self, pr: &PullRequestRef) -> Option<&PeekState> {
        self.peeks.get(pr)
    }

    pub fn author_face(&self, email: &str) -> Option<AvatarImage> {
        self.host.face(&author_face_key(email)?)
    }

    fn request_author_avatars(&mut self, emails: &[String]) -> Reduction {
        if !self.signed_in() {
            return Reduction::default();
        }
        let mut reduction = Reduction::default();
        for key in emails.iter().filter_map(|email| author_face_key(email)) {
            let Some(url) = author_avatar_url(&key) else {
                continue;
            };
            if self.avatars.contains(&url) || self.host.face(&key).is_some() {
                continue;
            }
            for evicted in self.avatars.insert(url.clone(), key) {
                self.host.remove_face(&evicted);
            }
            reduction = reduction.effect(ReviewEffect::FetchAvatar { url });
        }
        reduction
    }

    pub fn avatar_cache_len(&self) -> usize {
        self.avatars.len()
    }

    pub(super) fn open_overlay(&mut self, overlay: ReviewOverlay) {
        self.overlay = Some(overlay);
        self.focus_request = Some(LogicalFocus::ReviewDialog);
    }

    fn close_overlay(&mut self) {
        if let Some(closed) = self.overlay.take() {
            self.focus_request = Some(match closed {
                ReviewOverlay::SubmitReview => LogicalFocus::Viewport,
                ReviewOverlay::Auth | ReviewOverlay::AccountMenu => LogicalFocus::ReviewAccount,
            });
        }
    }

    pub fn is_modal_open(&self) -> bool {
        self.overlay.is_some()
    }

    pub fn overlay_open(&self) -> Option<ReviewOverlay> {
        self.overlay
    }

    pub fn composer(&self) -> Option<&Composer> {
        self.composer.as_ref()
    }

    pub fn overview(&self) -> &OverviewState {
        &self.overview
    }

    pub fn overview_open(&self) -> bool {
        self.overview.open && self.pr.is_some()
    }

    fn fetch_overview(&mut self) -> Option<ReviewEffect> {
        let pr = self.pr.as_ref()?.pr.clone();
        let token = self.token();
        let scope = self.job(Some(pr.clone()), Pending::Overview);
        self.overview.loading = true;
        self.overview.error = None;
        self.overview.request = Some(scope.request);
        Some(ReviewEffect::GitHub {
            scope,
            request: Box::new(GitHubRequest::FetchPullRequestOverview { pr, token }),
        })
    }

    fn refresh_overview(&mut self) -> Reduction {
        match self.fetch_overview() {
            Some(effect) => Reduction::changed().effect(effect),
            None => Reduction::default(),
        }
    }

    fn open_overview(&mut self) -> Reduction {
        if self.pr.is_none() {
            return Reduction::default();
        }
        if !self.overview.open {
            self.overview.open = true;
            self.overview.scroll.set_offset(0.0, 0.0);
        }
        self.focus_request = Some(LogicalFocus::ReviewOverview);
        if self.overview.loading || self.overview.data.is_some() {
            return Reduction::changed();
        }
        self.refresh_overview()
    }

    fn close_overview(&mut self) -> Reduction {
        if !self.overview.open {
            return Reduction::default();
        }
        self.overview.open = false;
        if self
            .composer
            .as_ref()
            .is_some_and(|c| c.target == ComposerTarget::Conversation)
        {
            self.composer = None;
            self.host.clear_composer();
        }
        self.focus_request = Some(LogicalFocus::Viewport);
        Reduction::changed()
    }

    fn open_conversation_composer(&mut self) -> Reduction {
        if let Err(reduction) = self.require_token("Sign in to comment on this pull request.") {
            return reduction;
        }
        if !self.overview_open() {
            return Reduction::default();
        }
        if self
            .composer
            .as_ref()
            .is_some_and(|c| c.target == ComposerTarget::Conversation)
        {
            self.focus_request = Some(LogicalFocus::ReviewComposer);
            return Reduction::changed();
        }
        self.overview.scroll.set_offset(0.0, f64::MAX);
        self.open_composer(ComposerTarget::Conversation, "")
    }

    pub fn signed_in(&self) -> bool {
        self.account
            .token
            .as_ref()
            .is_some_and(|token| !token.is_empty())
    }

    pub(super) fn token(&self) -> Option<Secret> {
        self.account.token.clone().filter(|token| !token.is_empty())
    }

    pub(super) fn next_job_id(&mut self) -> JobId {
        self.next_job += 1;
        JobId(self.next_job)
    }

    pub(super) fn job(&mut self, pr: Option<PullRequestRef>, kind: Pending) -> ReviewScope {
        let request = self.next_job_id();
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
                self.open_overlay(ReviewOverlay::Auth);
                Err(Reduction::changed().intent(info(why)))
            }
        }
    }

    fn bump_account(&mut self) -> Intent {
        self.epoch = AccountEpoch(self.epoch.0 + 1);
        self.in_flight.clear();
        self.overview.loading = false;
        self.overview.request = None;
        self.pulls.reset();
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
        let in_flight = self.in_flight.values().any(|entry| {
            matches!(entry.kind, Pending::ReviewData | Pending::ReviewComments)
                && entry.pr.as_ref() == Some(&pr)
        });
        let active = self.pr.as_mut()?;
        active.review_refetch = in_flight;
        if in_flight {
            return None;
        }
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

    fn queued_review_fetch(&mut self) -> Option<ReviewEffect> {
        if self.pr.as_ref()?.review_refetch {
            self.fetch_review()
        } else {
            None
        }
    }

    fn startup(&mut self, startup: &crate::contracts::shell::AppStartup) -> Reduction {
        self.client_id = startup.options.github_client_id.clone();
        self.account.user = startup.settings.github_user.clone();
        // `--repo` arrives as the workspace navigation activates; a pull
        // request loaded before that would answer a stale workspace.
        self.pending_url = startup.options.args.open_pr.clone();
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
                review_refetch: false,
                update: None,
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
        self.overview = OverviewState::default();
        self.expanded.clear();
        self.in_flight.retain(|_, entry| entry.pr.is_none());
        reduction
            .effect(ReviewEffect::WatchPullRequest(PullWatch::Stop))
            .intent(Intent::SetReviewTarget(None))
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
                left_is_merge_base: comparison.merge_base.is_some(),
            }))));
        if compare && let Some(repo_path) = self.repo_path.clone() {
            reduction = reduction.intent(Intent::RequestCompare(CompareIntent {
                repo_path,
                request: VcsCompareRequest {
                    spec: comparison.spec(),
                    layout: cx.settings.settings.viewport.layout,
                    renderer: saved_renderer(cx),
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
        // The refs keep their names across pushes, so a new head is a
        // change too. A base branch that moved on doesn't change a merge
        // base diff.
        let refs_changed = active.comparison.as_ref().is_none_or(|current| {
            current.left_ref != comparison.left_ref
                || current.right_ref != comparison.right_ref
                || current.info.head_sha != comparison.info.head_sha
                || current.merge_base != comparison.merge_base
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
        let merge_base = comparison.merge_base.clone();
        let mut from_merge_base = None;
        if let Some(active) = self.pr.as_ref() {
            self.peeks
                .insert(active.pr.clone(), PeekState::Ready(Box::new(info.clone())));
            from_merge_base = active
                .comparison
                .as_ref()
                .and_then(|loaded| loaded.merge_base.clone());
            let moved = active
                .session
                .as_ref()
                .is_some_and(|session| session.pull_request.head_sha != info.head_sha);
            // The comment being written is on the old head's lines: keep
            // it as a draft, so it follows them with the rest.
            if moved
                && self
                    .composer
                    .as_ref()
                    .is_some_and(|c| c.target != ComposerTarget::Conversation)
            {
                self.stage_composer();
                self.close_composer();
            }
        }
        let mut reduction = self.set_comparison(comparison, cx);
        let Some(active) = self.pr.as_mut() else {
            return reduction;
        };
        active.loading = false;
        active.error = None;
        active.update = None;
        let target = active.target();
        let mut carry = None;
        match active.session.as_mut() {
            Some(session) => {
                if session.pull_request.head_sha != info.head_sha {
                    let drafts = session.take_unposted_drafts();
                    if !drafts.is_empty() {
                        carry = Some(Box::new(DraftCarry {
                            from: session.pull_request.clone(),
                            from_merge_base,
                            drafts,
                        }));
                    }
                }
                session.pull_request = info.clone();
                session.status = ReviewSessionStatus::Loading;
            }
            None => {
                let mut session = ReviewSession::new(target.clone(), info.clone());
                session.status = ReviewSessionStatus::Loading;
                active.session = Some(session);
            }
        }
        let pr = active.pr.clone();
        let scope = self.job(Some(pr.clone()), Pending::Session);
        reduction = reduction.effect(ReviewEffect::LoadSession {
            scope,
            target,
            pull_request: Box::new(info.clone()),
            repo_path: self.repo_path.clone(),
            merge_base,
            carry,
        });
        reduction = reduction.effect(self.watch(pr, info));
        if let Some(effect) = self.fetch_review() {
            reduction = reduction.effect(effect);
        }
        if !self.overview.loading
            && let Some(effect) = self.fetch_overview()
        {
            reduction = reduction.effect(effect);
        }
        reduction
    }

    /// Starts noticing pushes to `pr`, which is loaded at `info`.
    fn watch(&mut self, pr: PullRequestRef, info: PullRequestInfo) -> ReviewEffect {
        self.in_flight
            .retain(|_, entry| !matches!(entry.kind, Pending::Watch));
        let token = self.token();
        let scope = self.job(Some(pr.clone()), Pending::Watch);
        ReviewEffect::WatchPullRequest(PullWatch::Start {
            scope,
            pr,
            token,
            info: Box::new(info),
        })
    }

    /// The watch saw the pull request change. A new head waits for the
    /// reviewer to load it, so the diff never shifts under them; state
    /// and title update in place, and new comments and checks are read
    /// at once, since they move nothing.
    fn pull_request_checked(
        &mut self,
        now: PullRequestFingerprint,
        change: HeadChange,
        remarks_changed: bool,
        checks_changed: bool,
    ) -> Reduction {
        let Some(active) = self.pr.as_mut() else {
            return Reduction::default();
        };
        let Some(comparison) = active.comparison.as_mut() else {
            return Reduction::default();
        };
        let loaded = &mut comparison.info;
        active.update = (loaded.head_sha != now.head_sha || loaded.base_branch != now.base_branch)
            .then_some(change);
        let state_changed =
            loaded.state != now.state || loaded.merged != now.merged || loaded.draft != now.draft;
        loaded.title = now.title;
        loaded.state = now.state;
        loaded.merged = now.merged;
        loaded.draft = now.draft;
        let mut reduction = Reduction::changed();
        if remarks_changed && let Some(effect) = self.fetch_review() {
            reduction = reduction.effect(effect);
        }
        if (checks_changed || remarks_changed || state_changed)
            && !self.overview.loading
            && let Some(effect) = self.fetch_overview()
        {
            reduction = reduction.effect(effect);
        }
        reduction
    }

    /// Loads the pull request again: a new head, comments and checks.
    fn refresh_pull_request(&mut self) -> Reduction {
        let Some(active) = self.pr.as_ref() else {
            return Reduction::default();
        };
        if active.loading {
            return Reduction::default();
        }
        self.pending_url = Some(active.url.clone());
        self.load_pending()
            .intent(info("Refreshing the pull request\u{2026}"))
    }

    fn update_pull_request(&mut self) -> Reduction {
        let Some(active) = self.pr.as_ref() else {
            return Reduction::default();
        };
        if active.update.is_none() || active.loading {
            return Reduction::default();
        }
        self.pending_url = Some(active.url.clone());
        self.load_pending()
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
        self.close_overlay();
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

    /// Keeps the open composer's text as a pending draft.
    fn stage_composer(&mut self) {
        let Some(composer) = self.composer.clone() else {
            return;
        };
        let body = self.host.composer_text().trim().to_owned();
        let Some(session) = self.session_mut() else {
            return;
        };
        match composer.target {
            ComposerTarget::Inline {
                draft: Some(id), ..
            }
            | ComposerTarget::General { draft: Some(id) }
            | ComposerTarget::Draft(id)
            | ComposerTarget::Reply { draft: id, .. } => {
                session.update_draft_body(id, body);
            }
            ComposerTarget::Inline {
                path,
                target,
                draft: None,
            } if !body.is_empty() => {
                session.create_inline_draft(ReviewAnchor::from_source_target(path, &target), body);
            }
            ComposerTarget::General { draft: None } if !body.is_empty() => {
                session.create_general_draft(body);
            }
            _ => {}
        }
    }

    fn close_composer(&mut self) {
        self.composer = None;
        self.host.clear_composer();
        self.focus_request = Some(if self.overview_open() {
            LogicalFocus::ReviewOverview
        } else {
            LogicalFocus::Viewport
        });
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
        self.open_composer(
            ComposerTarget::Inline {
                path,
                target,
                draft: None,
            },
            "",
        )
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
        let fresh = match &target {
            ComposerTarget::Reply { draft, .. } => Some(*draft),
            ComposerTarget::Inline { draft, .. } | ComposerTarget::General { draft } => *draft,
            ComposerTarget::Draft(_)
            | ComposerTarget::Edit { .. }
            | ComposerTarget::Conversation => None,
        };
        if let Some(draft) = fresh
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
            ComposerTarget::Inline {
                path,
                target,
                draft,
            } => {
                if let Some(session) = self.session_mut() {
                    match draft {
                        Some(id) => {
                            session.update_draft_body(id, body);
                        }
                        None => {
                            let anchor = ReviewAnchor::from_source_target(path, &target);
                            session.create_inline_draft(anchor, body);
                        }
                    }
                }
                reduction = reduction.intent(info("Review comment staged."));
            }
            ComposerTarget::General { draft } => {
                if let Some(session) = self.session_mut() {
                    match draft {
                        Some(id) => {
                            session.update_draft_body(id, body);
                        }
                        None => {
                            session.create_general_draft(body);
                        }
                    }
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
            ComposerTarget::Conversation => {
                reduction = reduction.effect(self.github(
                    Pending::Comment,
                    GitHubRequest::AddPullRequestComment {
                        pr,
                        token: Some(token),
                        body,
                    },
                ));
                self.close_composer();
                return reduction;
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
        self.close_overlay();
        if self
            .composer
            .as_ref()
            .is_some_and(|c| matches!(c.target, ComposerTarget::General { .. }))
        {
            self.sync_composer_draft();
            self.close_composer();
        }
        let composing = self.composer.as_ref().and_then(|c| c.target.draft());
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
            .filter(|draft| !draft.body.trim().is_empty() && Some(draft.id) != composing)
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
        let body = self.host.composer_text();
        let Some(composer) = self.composer.as_mut() else {
            return;
        };
        let draft = match &mut composer.target {
            ComposerTarget::Reply { draft, .. } => *draft,
            ComposerTarget::Inline {
                path,
                target,
                draft,
            } => match draft {
                Some(id) => *id,
                None if body.trim().is_empty() => return,
                None => {
                    let anchor = ReviewAnchor::from_source_target(path.clone(), target);
                    let Some(session) = self.pr.as_mut().and_then(|pr| pr.session.as_mut()) else {
                        return;
                    };
                    *draft.insert(session.create_inline_draft(anchor, ""))
                }
            },
            ComposerTarget::General { draft } => match draft {
                Some(id) => *id,
                None if body.trim().is_empty() => return,
                None => {
                    let Some(session) = self.pr.as_mut().and_then(|pr| pr.session.as_mut()) else {
                        return;
                    };
                    *draft.insert(session.create_general_draft(""))
                }
            },
            ComposerTarget::Draft(_)
            | ComposerTarget::Edit { .. }
            | ComposerTarget::Conversation => return,
        };
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
        let flow = self.next_job_id();
        self.account.flow = Some(flow);
        self.account.auth = AuthStatus::Starting;
        self.open_overlay(ReviewOverlay::Auth);
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
        let keep = matches!(
            response,
            GitHubResponse::PullRequestCached { .. }
                | GitHubResponse::PullRequestChecked { .. }
                | GitHubResponse::PullRequestCheckFailed { .. }
        );
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
            GitHubResponse::PullRequestChecked {
                now,
                change,
                remarks_changed,
                checks_changed,
                ..
            } => self.pull_request_checked(*now, change, remarks_changed, checks_changed),
            GitHubResponse::PullRequestCheckFailed { .. } => Reduction::default(),
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
                if let Some(effect) = self.queued_review_fetch() {
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
                if let Some(effect) = self.queued_review_fetch() {
                    reduction = reduction.effect(effect);
                }
                reduction
            }
            GitHubResponse::ReviewCommentsLoadFailed { message, .. } => {
                if let Some(session) = self.session_mut() {
                    session.status = ReviewSessionStatus::Failed;
                    session.status_message = Some(message);
                }
                let mut reduction = Reduction::changed();
                if let Some(effect) = self.queued_review_fetch() {
                    reduction = reduction.effect(effect);
                }
                reduction
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
            GitHubResponse::ReviewCommentReplied { comment, .. } => {
                if let Some(session) = self.session_mut() {
                    session.append_reply(&comment);
                }
                self.after_mutation(true)
            }
            GitHubResponse::CommentGraphqlUpdated { .. }
            | GitHubResponse::CommentGraphqlDeleted { .. }
            | GitHubResponse::ReviewCommentCreated { .. }
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
            GitHubResponse::PullRequestPeeked { pr, info } => {
                self.peeks.insert(pr, PeekState::Ready(Box::new(info)));
                Reduction::changed()
            }
            GitHubResponse::PullRequestPeekFailed { pr, message } => {
                self.peeks.insert(pr, PeekState::Failed(message));
                Reduction::changed()
            }
            GitHubResponse::PullRequestOverviewLoaded { overview, .. } => {
                if self.overview.request != Some(scope.request) {
                    return Reduction::default();
                }
                self.overview.request = None;
                self.overview.loading = false;
                self.overview.error = None;
                self.overview.data = Some(Rc::new(*overview));
                Reduction::changed()
            }
            GitHubResponse::PullRequestOverviewLoadFailed { message, .. } => {
                if self.overview.request != Some(scope.request) {
                    return Reduction::default();
                }
                tracing::warn!("failed to fetch pull request overview: {message}");
                self.overview.request = None;
                self.overview.loading = false;
                self.overview.error = Some(message);
                Reduction::changed()
            }
            GitHubResponse::MyPullRequestsLoaded(pulls) => self.pulls_loaded(scope.request, pulls),
            GitHubResponse::MyPullRequestsLoadFailed { message } => {
                self.pulls_failed(scope.request, message)
            }
            GitHubResponse::PullRequestCommentAdded { .. } => {
                let mut reduction = Reduction::changed().intent(info("Comment posted."));
                if let Some(effect) = self.fetch_overview() {
                    reduction = reduction.effect(effect);
                }
                reduction
            }
            GitHubResponse::PullRequestCommentAddFailed { body, message, .. } => {
                let mut reduction = Reduction::changed().intent(error(message));
                if self.overview_open() && self.composer.is_none() {
                    reduction.merge(self.open_composer(ComposerTarget::Conversation, &body));
                }
                reduction
            }
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
        if refetch
            && !self.overview.loading
            && let Some(effect) = self.fetch_overview()
        {
            reduction = reduction.effect(effect);
        }
        reduction
    }

    fn reduce(&mut self, action: ReviewAction, cx: &SharedContext) -> Reduction {
        let confirming = match &action {
            ReviewAction::CardDocument { .. }
            | ReviewAction::ScrollComposer(_)
            | ReviewAction::CopyCardSelection(_) => None,
            _ => self.confirm_delete.take(),
        };
        let keeps_menu = matches!(
            &action,
            ReviewAction::ToggleCommentMenu { .. }
                | ReviewAction::DeleteComment { .. }
                | ReviewAction::CardDocument { .. }
                | ReviewAction::ScrollComposer(_)
                | ReviewAction::CopyCardSelection(_)
        );
        let cleared = confirming.is_some() || (!keeps_menu && self.comment_menu.take().is_some());
        let mut reduction = match action {
            ReviewAction::ToggleCommentMenu { comment_node_id } => {
                self.comment_menu =
                    if self.comment_menu.as_deref() == Some(comment_node_id.as_str()) {
                        None
                    } else {
                        Some(comment_node_id)
                    };
                Reduction::changed()
            }
            ReviewAction::DeleteComment { comment_node_id } => {
                if matches!(&confirming, Some(DeleteTarget::Comment(node)) if *node == comment_node_id)
                {
                    self.comment_menu = None;
                    self.delete_comment(comment_node_id)
                } else {
                    self.confirm_delete = Some(DeleteTarget::Comment(comment_node_id));
                    Reduction::changed()
                }
            }
            ReviewAction::DeleteDraft(id) => {
                if confirming == Some(DeleteTarget::Draft(id)) {
                    self.delete_draft(id)
                } else {
                    self.confirm_delete = Some(DeleteTarget::Draft(id));
                    Reduction::changed()
                }
            }
            action => self.reduce_other(action, cx),
        };
        reduction.changed |= cleared;
        reduction
    }

    fn reduce_other(&mut self, action: ReviewAction, cx: &SharedContext) -> Reduction {
        match action {
            ReviewAction::OpenAuthModal => {
                self.open_overlay(ReviewOverlay::Auth);
                if self.account.auth == AuthStatus::Done && !self.signed_in() {
                    self.account.auth = AuthStatus::Idle;
                }
                Reduction::changed()
            }
            ReviewAction::CloseOverlay => {
                self.close_overlay();
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
                self.open_overlay(ReviewOverlay::AccountMenu);
                Reduction::changed()
            }
            ReviewAction::SignOut => self.sign_out(cx),
            ReviewAction::LoadPullRequest { url } => self.open_pull_request(url),
            ReviewAction::PreviewPullRequest => self.preview_pull_request(cx),
            ReviewAction::PeekPullRequest { url } => self.peek_pull_request(&url),
            ReviewAction::OpenSubmitMenu => {
                if let Err(reduction) = self.require_token("Sign in to submit a review.") {
                    return reduction;
                }
                self.open_overlay(ReviewOverlay::SubmitReview);
                Reduction::changed()
            }
            ReviewAction::RequestAuthorAvatars(emails) => self.request_author_avatars(&emails),
            ReviewAction::ToggleFileComments(path) => {
                self.file_comments = if self.file_comments.as_ref() == Some(&path) {
                    None
                } else {
                    Some(path)
                };
                Reduction::changed()
            }
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
                self.open_composer(ComposerTarget::General { draft: None }, "")
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
            ReviewAction::SubmitReview { decision } => self.submit_review(decision),
            ReviewAction::DiscardDrafts => self.discard_drafts(),
            ReviewAction::DeleteComment { .. }
            | ReviewAction::DeleteDraft(_)
            | ReviewAction::ToggleCommentMenu { .. } => Reduction::default(),
            ReviewAction::ToggleThread(id) => self.toggle_thread(id),
            ReviewAction::ReplyToFocused => match self.focused_thread() {
                Some(thread) => self.reply_to(thread.id.clone()),
                None => Reduction::default(),
            },
            ReviewAction::ResolveFocused => {
                let Some(thread) = self.focused_thread() else {
                    return Reduction::default();
                };
                let resolved = thread.status.resolution != ReviewResolution::Resolved;
                let id = thread.id.clone();
                self.set_thread_resolved(id, resolved)
            }
            ReviewAction::ToggleFocused => match self.focused_thread() {
                Some(thread) => {
                    let id = thread.id.clone();
                    self.toggle_thread(id)
                }
                None => Reduction::default(),
            },
            ReviewAction::SetThreadResolved { id, resolved } => {
                self.set_thread_resolved(id, resolved)
            }
            ReviewAction::EditDraft(id) => self.edit_draft(id),
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
            ReviewAction::OpenOverview => self.open_overview(),
            ReviewAction::CloseOverview => self.close_overview(),
            ReviewAction::ToggleOverview => {
                if self.overview_open() {
                    self.close_overview()
                } else {
                    self.open_overview()
                }
            }
            ReviewAction::UpdatePullRequest => self.update_pull_request(),
            ReviewAction::RefreshPullRequest => self.refresh_pull_request(),
            // Routed to a reply or the viewport before it gets here.
            ReviewAction::CommentHere => Reduction::default(),
            ReviewAction::WindowFocused(focused) => Reduction::default().effect(
                ReviewEffect::WatchPullRequest(PullWatch::WindowFocused(focused)),
            ),
            ReviewAction::RefreshOverview => {
                if self.overview.loading {
                    Reduction::default()
                } else {
                    self.refresh_overview()
                }
            }
            ReviewAction::ToggleOverviewChecks => {
                self.overview.checks_expanded = !self.overview.checks_expanded;
                Reduction::changed()
            }
            ReviewAction::OpenConversationComposer => self.open_conversation_composer(),
            ReviewAction::OpenLink(url) => {
                if url.starts_with("https://") {
                    Reduction::default().effect(ShellEffect::OpenBrowser { url })
                } else {
                    Reduction::default()
                }
            }
            ReviewAction::Pulls(action) => self.reduce_pulls(action, cx),
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
                    self.close_overlay();
                }
                let mut reduction = Reduction::changed()
                    .effect(ReviewEffect::SaveToken(token))
                    .intent(self.bump_account())
                    .intent(info("GitHub authentication completed."));
                // The bump dropped the watch's scope: watch again, signed in.
                if let Some((pr, info)) = self.pr.as_ref().and_then(|active| {
                    let comparison = active.comparison.as_ref()?;
                    Some((active.pr.clone(), comparison.info.clone()))
                }) {
                    reduction = reduction.effect(self.watch(pr, info));
                }
                reduction.merge(self.fetch_user());
                reduction.merge(self.load_pending());
                if self.pr.as_ref().is_some_and(|pr| pr.comparison.is_some())
                    && let Some(effect) = self.fetch_review()
                {
                    reduction = reduction.effect(effect);
                }
                if self.pr.is_some()
                    && let Some(effect) = self.fetch_overview()
                {
                    reduction = reduction.effect(effect);
                }
                if self.pulls.open {
                    reduction.merge(self.fetch_pulls());
                }
                reduction
            }
            ReviewEvent::TokenLoaded { token } => {
                self.account.token_loaded = true;
                self.account.token = token.filter(|token| !token.is_empty());
                let mut reduction = Reduction::changed();
                reduction.merge(self.fetch_user());
                reduction.merge(self.load_pending());
                if self.pulls.open && self.pulls.data.is_none() && !self.pulls.loading {
                    reduction.merge(self.fetch_pulls());
                }
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
                // Drafts that followed a push were saved with the session,
                // but a save queued while it loaded can land after that.
                let mut reduction = Reduction::changed();
                if self
                    .session()
                    .is_some_and(ReviewSession::has_unposted_drafts)
                    && let Some(effect) = self.save_session()
                {
                    reduction = reduction.effect(effect);
                }
                reduction
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
                let picture = AvatarImage {
                    rgba: Arc::from(image.rgba.as_slice()),
                    width: image.width,
                    height: image.height,
                    cache_key: avatar_cache_key(&url),
                };
                let mut changed = false;
                if self.account.avatar_url.as_deref() == Some(url.as_str()) {
                    self.account.avatar = Some(picture.clone());
                    changed = true;
                }
                if let Some(login) = self.avatars.settle(&url, true) {
                    self.host.set_face(login, picture);
                    changed = true;
                }
                if changed {
                    Reduction::changed()
                } else {
                    Reduction::default()
                }
            }
            ReviewEvent::AvatarFetchFailed { url, message } => {
                tracing::warn!("failed to fetch avatar {url}: {message}");
                self.avatars.settle(&url, false);
                Reduction::default()
            }
            ReviewEvent::ClonesResolved { job, clones } => self.clones_resolved(job, clones, cx),
            ReviewEvent::ReviewCacheReady { repo, result } => self.review_cache_ready(repo, result),
            ReviewEvent::CloneValidated { repo, result } => self.clone_validated(repo, result, cx),
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
                self.pulls.open = false;
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
            Intent::OpenPullRequest { url } => {
                self.pulls.open = false;
                self.open_pull_request(url.clone())
            }
            Intent::FocusReviewCard(card) => {
                if self.focused_card == *card {
                    return Reduction::default();
                }
                self.focused_card = card.clone();
                Reduction::changed()
            }
            Intent::RequestAnnotation(target) => {
                let mut reduction = self.close_overview();
                reduction.merge(self.open_composer_at(target.clone(), cx));
                reduction
            }
            Intent::SelectFile(Some(selected))
                if self.overview.open && selected.origin != SelectionOrigin::Viewport =>
            {
                let reduction = self.close_overview();
                self.focus_request = None;
                reduction
            }
            Intent::GrammarAvailable(_) => {
                self.host.grammars_changed();
                Reduction::changed()
            }
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
            for effect in self.avatar_requests() {
                reduction.effects.push(effect.into());
            }
            self.refresh(cx);
        }
        reduction
    }

    fn avatar_requests(&mut self) -> Vec<ReviewEffect> {
        let comments = self
            .session()
            .into_iter()
            .flat_map(|session| session.threads.iter().flat_map(|thread| &thread.comments))
            .filter_map(|comment| {
                Some((
                    comment.author_login.as_deref()?,
                    comment.author_avatar_url.as_deref()?,
                ))
            });
        let overview = self
            .overview
            .data
            .as_deref()
            .filter(|_| self.overview.open)
            .into_iter()
            .flat_map(|overview| {
                std::iter::once(&overview.author)
                    .chain(overview.reviewers.iter().map(|reviewer| &reviewer.actor))
                    .chain(overview.timeline.iter().map(|item| &item.author))
            })
            .map(|actor| (actor.login.as_str(), actor.avatar_url.as_str()));
        let pulls = self
            .pulls
            .data
            .as_deref()
            .filter(|_| self.pulls.open)
            .into_iter()
            .flatten()
            .flat_map(|pull| pull.reviewers.iter())
            .map(|reviewer| {
                (
                    reviewer.actor.login.as_str(),
                    reviewer.actor.avatar_url.as_str(),
                )
            });
        let mut wanted: Vec<(String, String)> = Vec::new();
        for (login, raw) in comments.chain(overview).chain(pulls) {
            if login.is_empty() {
                continue;
            }
            let Some(url) = avatar_url_sized(raw, REVIEW_AVATAR_PX) else {
                continue;
            };
            if !self.avatars.contains(&url) && wanted.iter().all(|(u, _)| *u != url) {
                wanted.push((url, login.to_owned()));
            }
        }
        wanted
            .into_iter()
            .map(|(url, login)| {
                for evicted in self.avatars.insert(url.clone(), login) {
                    self.host.remove_face(&evicted);
                }
                ReviewEffect::FetchAvatar { url }
            })
            .collect()
    }

    fn open_pull_request(&mut self, url: String) -> Reduction {
        self.pending_url = Some(url);
        let reduction = self.load_pending();
        if self.pending_url.is_some() && self.repo_path.is_none() {
            return reduction.intent(info("Open a repository to load the pull request."));
        }
        reduction
    }

    fn peek_pull_request(&mut self, url: &str) -> Reduction {
        let Some(parsed) = parse_pr_url(url) else {
            return Reduction::default();
        };
        let pr = PullRequestRef {
            owner: parsed.owner,
            repo: parsed.repo,
            number: parsed.number,
        };
        if matches!(
            self.peeks.get(&pr),
            Some(PeekState::Loading | PeekState::Ready(_))
        ) {
            return Reduction::default();
        }
        self.peeks.insert(pr.clone(), PeekState::Loading);
        let scope = self.job(None, Pending::Peek);
        let token = self.token();
        Reduction::changed().effect(ReviewEffect::GitHub {
            scope,
            request: Box::new(GitHubRequest::PeekPullRequest { pr, token }),
        })
    }

    fn preview_pull_request(&mut self, cx: &SharedContext) -> Reduction {
        let Some(repository) = cx.repository.as_ref() else {
            return Reduction::changed()
                .intent(error("Open a repository to preview a pull request."));
        };
        let snapshot = &repository.snapshot;
        if snapshot.location.kind != VcsKind::GIT {
            return Reduction::changed()
                .intent(error("PR preview is only available for Git repositories."));
        }
        let Some(base) = default_pull_request_base(&snapshot.refs) else {
            return Reduction::changed().intent(error("No default branch found for PR preview."));
        };
        Reduction::changed().intent(Intent::RequestCompare(CompareIntent {
            repo_path: snapshot.path.clone(),
            request: VcsCompareRequest {
                spec: VcsCompareSpec::MergeBaseRange {
                    base,
                    head: WORKDIR_REF.to_owned(),
                },
                layout: cx.settings.settings.viewport.layout,
                renderer: saved_renderer(cx),
            },
        }))
    }

    pub fn has_focused_thread(&self) -> bool {
        self.focused_thread().is_some()
    }

    fn focused_thread(&self) -> Option<&ReviewThread> {
        let Some(CardKey::Thread(id)) = &self.focused_card else {
            return None;
        };
        self.session()?
            .threads
            .iter()
            .find(|thread| &thread.id == id)
    }

    pub(super) fn refresh(&mut self, cx: &SharedContext) {
        self.presentation = Rc::new(ReviewPresentation::build(self, cx));
    }

    pub(super) fn bump_presentation(&mut self) {
        self.presentation = Rc::new(self.presentation.bumped());
    }
}

pub(super) fn default_pull_request_base(refs: &[VcsRef]) -> Option<String> {
    let active = refs
        .iter()
        .find(|reference| reference.active && reference.kind == RefKind::Branch)
        .map(|reference| reference.name.as_str());
    let candidate = |name: &str| {
        refs.iter()
            .find(|reference| {
                reference.name == name
                    && active != Some(reference.name.as_str())
                    && matches!(reference.kind, RefKind::Branch | RefKind::RemoteBranch)
            })
            .map(|reference| reference.name.clone())
    };
    const PREFERRED: [&str; 10] = [
        "origin/main",
        "origin/master",
        "upstream/main",
        "upstream/master",
        "origin/develop",
        "origin/development",
        "main",
        "master",
        "develop",
        "development",
    ];
    if let Some(found) = PREFERRED.iter().find_map(|name| candidate(name)) {
        return Some(found);
    }
    ["main", "master", "develop", "development"]
        .iter()
        .find_map(|trunk| {
            let suffix = format!("/{trunk}");
            refs.iter()
                .find(|reference| {
                    reference.name.ends_with(&suffix)
                        && active != Some(reference.name.as_str())
                        && reference.kind == RefKind::RemoteBranch
                })
                .map(|reference| reference.name.clone())
        })
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
        (target == REVIEW_COMPOSER && self.composer.as_ref().is_some_and(|c| !c.preview))
            || (target == REVIEW_PULLS_FILTER && self.pulls.open)
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
        if target == REVIEW_PULLS_FILTER {
            return Some(self.edit_pulls_filter(command, cx.now_ms));
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

    fn after_edit(&mut self, target: FocusId, _cx: &SharedContext) -> Reduction {
        if target != REVIEW_COMPOSER {
            return Reduction::default();
        }
        let Some(active) = self.pr.as_mut().filter(|pr| pr.session_dirty) else {
            return Reduction::default();
        };
        active.session_dirty = false;
        let Some(session) = active.session.clone() else {
            return Reduction::default();
        };
        let scope = ReviewScope {
            account: self.epoch,
            request: JobId(0),
        };
        Reduction::default().effect(ReviewEffect::SaveSessionDraft {
            scope,
            session: Box::new(session),
        })
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
        if target == REVIEW_PULLS_FILTER {
            self.pulls.filter.set_preedit(text, cursor);
            return true;
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
        if target == REVIEW_PULLS_FILTER {
            let text_changed = self.pulls.filter.text() != value;
            self.set_pulls_filter(&value);
            return Some(TextEditOutcome {
                text_changed,
                selection_changed: true,
                ..TextEditOutcome::default()
            });
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

impl CommandSource for ReviewController {
    fn commands(&self, cx: &SharedContext, out: &mut Vec<CommandDescriptor>) {
        let signed_in = self.signed_in();
        let reviewing = self.session().is_some();
        let vcs_git = cx
            .repository
            .as_ref()
            .is_some_and(|repository| repository.snapshot.location.kind == VcsKind::GIT);
        out.push(
            CommandDescriptor::new(
                "review.open_pull_request",
                "Open pull request",
                CommandsAction::OpenPullRequestPicker,
            )
            .detail("Load a GitHub pull request by URL"),
        );
        out.push(
            CommandDescriptor::new(
                "review.refresh_pull_request",
                "Refresh pull request",
                ReviewAction::RefreshPullRequest,
            )
            .detail("Read its commits, comments and checks again")
            .enabled(self.pr.as_ref().is_some_and(|active| !active.loading)),
        );
        out.push(
            CommandDescriptor::new(
                "review.update_pull_request",
                "Update pull request",
                ReviewAction::UpdatePullRequest,
            )
            .detail("Load the latest push to this pull request")
            .enabled(
                self.pr
                    .as_ref()
                    .is_some_and(|active| active.update.is_some() && !active.loading),
            ),
        );
        out.push(
            CommandDescriptor::new(
                "review.your_pull_requests",
                "Your pull requests",
                ReviewAction::Pulls(PullsAction::Open),
            )
            .detail("Open pull requests you authored, across repositories"),
        );
        out.push(
            CommandDescriptor::new(
                "review.preview_pull_request",
                "Preview pull request",
                ReviewAction::PreviewPullRequest,
            )
            .detail("Compare the working tree with the default branch")
            .enabled(vcs_git),
        );
        out.push(
            CommandDescriptor::new(
                "review.submit",
                "Submit review",
                ReviewAction::OpenSubmitMenu,
            )
            .enabled(reviewing && signed_in),
        );
        out.push(
            CommandDescriptor::new(
                "review.approve",
                "Approve pull request",
                ReviewAction::SubmitReview {
                    decision: ReviewDecision::Approve,
                },
            )
            .enabled(reviewing && signed_in),
        );
        out.push(
            CommandDescriptor::new(
                "review.request_changes",
                "Request changes",
                ReviewAction::SubmitReview {
                    decision: ReviewDecision::RequestChanges,
                },
            )
            .enabled(reviewing && signed_in),
        );
        out.push(if self.overview_open() {
            CommandDescriptor::new(
                "review.overview",
                "Show changed files",
                ReviewAction::CloseOverview,
            )
            .detail("Return from the pull request overview to the diff")
        } else {
            CommandDescriptor::new(
                "review.overview",
                "Pull request overview",
                ReviewAction::OpenOverview,
            )
            .detail("Description, checks, reviewers and conversation")
            .enabled(self.pr.is_some())
        });
        out.push(
            CommandDescriptor::new(
                "review.open_in_browser",
                "Open pull request on GitHub",
                ReviewAction::OpenPullRequestInBrowser,
            )
            .enabled(self.pr.is_some()),
        );
        out.push(if signed_in {
            CommandDescriptor::new(
                "review.sign_out",
                "Sign out of GitHub",
                ReviewAction::SignOut,
            )
        } else {
            CommandDescriptor::new(
                "review.sign_in",
                "Sign in to GitHub",
                ReviewAction::OpenAuthModal,
            )
        });
    }
}
