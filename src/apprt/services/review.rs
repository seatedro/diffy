use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::contracts::review::{
    AvatarImage, DraftCarry, GitHubRequest, GitHubResponse, HeadChange, PullRequestComparison,
    PullRequestRef, Secret,
};
use crate::core::compare::{LayoutMode, RendererKind};
use crate::core::error::{DiffyError, Result};
use crate::core::forge::github::{
    DeviceFlowState, GitHubApi, PullRequestFingerprint, PullRequestInfo, parse_pr_url,
    poll_for_token, start_device_flow,
};
use crate::core::frecency::{FrecencyStore, recent_repo_paths};
use crate::core::http;
use crate::core::repo_discovery;
use crate::core::review::{
    ReviewDraft, ReviewDraftKind, ReviewSession, ReviewSessionKey, ReviewSide, ReviewTarget,
};
use crate::core::vcs::discovery;
use crate::core::vcs::git::{
    LineHunk, github_remotes, init_pull_request_cache, is_pull_request_cache, line_hunks,
    merge_base_oid, pr_ref_paths, ref_oid,
};
use crate::core::vcs::model::{VcsCompareRequest, VcsKind};
use crate::platform::persistence::SettingsStore;
use crate::platform::review_store::ReviewStore;
use crate::platform::secrets;
use crate::platform::startup::{is_review_cache, review_cache_dir};

const DEV_GITHUB_TOKEN_FILE_NAME: &str = "github-token.dev";
const CLONE_CANDIDATES: usize = 400;

pub trait GitHubBackend: Send + Sync + 'static {
    fn execute(&self, request: GitHubRequest, emit: &mut dyn FnMut(GitHubResponse));
}

#[derive(Clone)]
pub struct ReviewServices {
    settings_store: SettingsStore,
    review_store: ReviewStore,
    github: Arc<dyn GitHubBackend>,
}

impl ReviewServices {
    pub fn new(
        settings_store: SettingsStore,
        review_store: ReviewStore,
        github: Arc<dyn GitHubBackend>,
    ) -> Self {
        Self {
            settings_store,
            review_store,
            github,
        }
    }

    pub fn github(&self) -> &Arc<dyn GitHubBackend> {
        &self.github
    }

    pub fn start_device_flow(&self, client_id: &str) -> Result<DeviceFlowState> {
        if client_id.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub client id is empty. Set DIFFY_GITHUB_CLIENT_ID.".to_owned(),
            ));
        }
        start_device_flow(client_id)
    }

    pub fn poll_device_flow(
        &self,
        client_id: &str,
        device_code: &str,
        interval_seconds: u32,
    ) -> Result<Secret> {
        loop {
            match poll_for_token(client_id, device_code)? {
                Some(token) => return Ok(Secret::new(token)),
                None => thread::sleep(Duration::from_secs(u64::from(interval_seconds.max(5)))),
            }
        }
    }

    pub fn load_keyring_token(&self) -> Result<Option<Secret>> {
        Ok(secrets::load_github_token()?.map(Secret::new))
    }

    pub fn save_keyring_token(&self, token: &Secret) -> Result<()> {
        secrets::save_github_token(token.expose())
    }

    pub fn clear_keyring_token(&self) -> Result<()> {
        secrets::clear_github_token()
    }

    pub fn load_dev_token(&self) -> Result<Option<Secret>> {
        Ok(secrets::load_github_token_file(&self.dev_token_path()?)?.map(Secret::new))
    }

    pub fn save_dev_token(&self, token: &Secret) -> Result<()> {
        secrets::save_github_token_file(&self.dev_token_path()?, token.expose())
    }

    pub fn clear_dev_token(&self) -> Result<()> {
        secrets::clear_github_token_file(&self.dev_token_path()?)
    }

    fn dev_token_path(&self) -> Result<PathBuf> {
        Ok(self.settings_store.dir().join(DEV_GITHUB_TOKEN_FILE_NAME))
    }

    /// The session for `pull_request`'s head. Drafts not yet posted on an
    /// earlier head, `carry`'s or ones left in the store, follow it there:
    /// moved to where their lines went, or outdated when those lines
    /// changed.
    pub fn load_session(
        &self,
        target: ReviewTarget,
        pull_request: PullRequestInfo,
        repo_path: Option<&Path>,
        merge_base: Option<&str>,
        carry: Option<DraftCarry>,
    ) -> Result<ReviewSession> {
        let head = pull_request.head_sha.clone();
        let mut session = self
            .review_store
            .load_session(&target, &head)?
            .unwrap_or_else(|| ReviewSession::new(target.clone(), pull_request.clone()));
        let mut earlier: Vec<ReviewSession> = self
            .review_store
            .load_sessions_for_target(&target)?
            .into_iter()
            .filter(|old| old.pull_request.head_sha != head && old.has_unposted_drafts())
            .collect();
        let carry_head = carry.as_ref().map(|carry| carry.from.head_sha.clone());
        let mut carried = Vec::new();
        for old in &mut earlier {
            let drafts = old.take_unposted_drafts();
            // In memory, carry's drafts are newer than the ones saved for
            // its head: those are only cleared.
            if carry_head.as_deref() != Some(old.pull_request.head_sha.as_str()) {
                carried.push((old.pull_request.clone(), None, drafts));
            }
        }
        if let Some(carry) = carry {
            carried.push((carry.from, carry.from_merge_base, carry.drafts));
        }
        if carried.is_empty() {
            return Ok(session);
        }
        let to = Commits {
            info: &pull_request,
            merge_base,
        };
        for (from, from_merge_base, drafts) in carried {
            let from = Commits {
                info: &from,
                merge_base: from_merge_base.as_deref(),
            };
            session.adopt_drafts(follow_drafts(repo_path, &from, &to, drafts));
        }
        // The new head's session first: drafts are cleared from the old
        // ones only once they are safe there.
        self.review_store.save_session(&session)?;
        for old in &earlier {
            self.review_store.save_session(old)?;
        }
        Ok(session)
    }

    pub fn save_session(&self, session: &ReviewSession) -> Result<ReviewSessionKey> {
        let key = session.key();
        self.review_store.save_session(session)?;
        Ok(key)
    }

    pub fn resolve_clones(&self, current: Option<PathBuf>) -> Vec<(String, PathBuf)> {
        let dir = self.settings_store.dir();
        let recent = FrecencyStore::open(dir)
            .map(|store| recent_repo_paths(Some(&store), CLONE_CANDIDATES))
            .unwrap_or_default();
        let discovered = repo_discovery::load_cache(dir)
            .map(|scan| scan.known.into_iter().chain(scan.repos))
            .into_iter()
            .flatten()
            .map(|repo| repo.path);
        let mut seen = HashSet::new();
        let mut clones: Vec<(String, PathBuf)> = Vec::new();
        let candidates = current.into_iter().chain(recent).chain(discovered);
        // A review cache stands in only for a repository with no clone.
        for path in candidates
            .filter(|path| seen.insert(path.clone()))
            .take(CLONE_CANDIDATES)
            .filter(|path| !is_review_cache(path))
        {
            let Ok((root, keys)) = github_remotes(&path) else {
                continue;
            };
            for key in keys {
                if clones.iter().all(|(known, _)| *known != key) {
                    clones.push((key, root.clone()));
                }
            }
        }
        clones
    }

    /// The repository kept for reviewing `repo`'s pull requests without a
    /// clone, made on first use.
    pub fn prepare_review_cache(&self, repo: &str) -> Result<PathBuf> {
        let valid = |part: &str| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        let Some((owner, name)) = repo.split_once('/').filter(|(o, n)| valid(o) && valid(n)) else {
            return Err(DiffyError::Parse(format!(
                "{repo} is not a GitHub repository"
            )));
        };
        let dir = review_cache_dir()
            .ok_or_else(|| DiffyError::General("no cache directory to keep it in".to_owned()))?
            .join(owner.to_ascii_lowercase())
            .join(name.to_ascii_lowercase());
        init_pull_request_cache(&dir, owner, name)?;
        Ok(dir)
    }

    pub fn validate_clone(&self, repo: &str, path: &Path) -> Result<PathBuf> {
        let (root, keys) = github_remotes(path).map_err(|_| {
            DiffyError::General(format!("{} is not a Git repository.", path.display()))
        })?;
        if keys.iter().any(|key| key.eq_ignore_ascii_case(repo)) {
            Ok(root)
        } else {
            Err(DiffyError::General(format!(
                "No remote in {} points at {repo}.",
                root.display()
            )))
        }
    }

    pub fn fetch_avatar(&self, url: &str) -> Result<AvatarImage> {
        let bytes = http::block_on(async {
            let response = crate::core::http::client()
                .get(url)
                .header("User-Agent", "diffy/0.1")
                .send()
                .await
                .map_err(|error| DiffyError::network(format!("avatar fetch failed: {error}")))?;
            http::response_bytes(response, "avatar fetch").await
        })?;
        let img = image::load_from_memory(&bytes)
            .map_err(|error| DiffyError::Parse(format!("avatar decode failed: {error}")))?
            .to_rgba8();
        let (width, height) = img.dimensions();
        let mut rgba = img.into_raw();
        apply_circular_mask(&mut rgba, width, height);
        Ok(AvatarImage {
            rgba,
            width,
            height,
        })
    }
}

#[derive(Clone)]
pub struct LiveGitHub {
    review_store: ReviewStore,
}

impl LiveGitHub {
    pub fn new(review_store: ReviewStore) -> Self {
        Self { review_store }
    }

    fn load_pull_request(
        &self,
        url: &str,
        repo_path: &Path,
        token: Option<Secret>,
    ) -> Result<PullRequestComparison> {
        let total_started = Instant::now();
        let parsed = parse_pr_url(url)
            .ok_or_else(|| DiffyError::Parse("not a valid GitHub pull request URL".to_owned()))?;
        let token = token.map(Secret::into_inner).unwrap_or_default();

        let stage_started = Instant::now();
        let mut repo = discovery::open_repository(repo_path)?;
        tracing::info!(
            owner = %parsed.owner,
            repo = %parsed.repo,
            number = parsed.number,
            elapsed_ms = stage_started.elapsed().as_millis(),
            "github-pr: opened repository"
        );
        if !repo.capabilities().github_pull_requests {
            return Err(DiffyError::General(
                "this repository backend does not support GitHub pull request comparisons"
                    .to_owned(),
            ));
        }
        if repo.location().kind != VcsKind::GIT {
            repo = discovery::open_git_repository(repo_path)?;
        }
        let stage_started = Instant::now();
        let refs = repo.resolve_pull_request_comparison(url, &token)?;
        let info = refs.info;
        tracing::info!(
            owner = %parsed.owner,
            repo = %parsed.repo,
            number = parsed.number,
            changed_files = info.changed_files,
            elapsed_ms = stage_started.elapsed().as_millis(),
            total_elapsed_ms = total_started.elapsed().as_millis(),
            "github-pr: resolved comparison refs"
        );
        Ok(PullRequestComparison {
            info,
            left_ref: refs.base,
            right_ref: refs.head,
            merge_base: refs.merge_base,
        })
    }

    fn cached_pull_request_comparison(
        &self,
        url: &str,
        repo_path: &Path,
    ) -> Result<Option<PullRequestComparison>> {
        let parsed = parse_pr_url(url)
            .ok_or_else(|| DiffyError::Parse("not a valid GitHub pull request URL".to_owned()))?;
        let target = ReviewTarget::github(parsed.owner, parsed.repo, parsed.number);
        let sessions = self.review_store.load_sessions_for_target(&target)?;
        if sessions.is_empty() {
            return Ok(None);
        }

        let mut repo = discovery::open_repository(repo_path)?;
        if !repo.capabilities().github_pull_requests {
            return Ok(None);
        }
        if repo.location().kind != VcsKind::GIT {
            let Ok(git_repo) = discovery::open_git_repository(repo_path) else {
                return Ok(None);
            };
            repo = git_repo;
        }

        let shallow = is_pull_request_cache(repo_path);
        for session in sessions.into_iter().rev() {
            let info = session.pull_request;
            if info.base_branch.is_empty() || info.head_branch.is_empty() {
                continue;
            }
            let (left_ref, right_ref) =
                pr_ref_paths(info.number, &info.base_branch, &info.head_branch);
            let merge_base = if shallow {
                match ref_oid(repo_path, &left_ref) {
                    Ok(oid) => Some(oid),
                    Err(_) => continue,
                }
            } else {
                None
            };
            let comparison = PullRequestComparison {
                left_ref,
                right_ref,
                info,
                merge_base,
            };
            let request = VcsCompareRequest {
                spec: comparison.spec(),
                layout: LayoutMode::default(),
                renderer: RendererKind::default(),
            };
            if repo.resolve_compare_request(&request).is_ok() {
                tracing::info!(
                    owner = %target.owner,
                    repo = %target.repo,
                    number = target.number,
                    "github-pr: using cached comparison refs"
                );
                return Ok(Some(comparison));
            }
        }

        Ok(None)
    }
}

/// Moves `drafts` written on `from`'s head to `to`'s: new-side lines
/// through the diff between the heads, old-side lines through the diff
/// between the merge bases. Lines a change touched, or that can't be
/// followed, make the draft outdated rather than land somewhere wrong.
/// A head and, when it was fetched without history, its merge base.
struct Commits<'a> {
    info: &'a PullRequestInfo,
    merge_base: Option<&'a str>,
}

impl Commits<'_> {
    fn merge_base(&self, repo: &Path) -> Option<String> {
        match self.merge_base {
            Some(oid) => Some(oid.to_owned()),
            None => merge_base_oid(repo, &self.info.base_sha, &self.info.head_sha).ok(),
        }
    }
}

fn follow_drafts(
    repo_path: Option<&Path>,
    from: &Commits,
    to: &Commits,
    mut drafts: Vec<ReviewDraft>,
) -> Vec<ReviewDraft> {
    let mut bases: Option<Option<(String, String)>> = None;
    let mut hunks: HashMap<(ReviewSide, String), Option<Vec<LineHunk>>> = HashMap::new();
    for draft in &mut drafts {
        if !matches!(draft.kind, ReviewDraftKind::InlineComment) {
            continue;
        }
        let Some(anchor) = draft.anchor.as_mut() else {
            continue;
        };
        let (Some(side), Some(lines)) = (anchor.side, anchor.line_range) else {
            continue;
        };
        let commits = match side {
            ReviewSide::New => Some((from.info.head_sha.clone(), to.info.head_sha.clone())),
            ReviewSide::Old => bases
                .get_or_insert_with(|| {
                    let repo = repo_path?;
                    Some((from.merge_base(repo)?, to.merge_base(repo)?))
                })
                .clone(),
        };
        let followed = commits.and_then(|(old, new)| {
            if old == new {
                return Some(lines);
            }
            let hunks = hunks.entry((side, anchor.path.clone())).or_insert_with(|| {
                line_hunks(repo_path?, &old, &new, &anchor.path)
                    .inspect_err(|error| tracing::warn!("could not follow a draft: {error}"))
                    .ok()
            });
            lines.follow(hunks.as_deref()?)
        });
        match followed {
            Some(lines) => anchor.line_range = Some(lines),
            None => anchor.mark_outdated(),
        }
    }
    drafts
}

fn api(token: Option<Secret>) -> GitHubApi {
    GitHubApi::with_token(token.map(Secret::into_inner).unwrap_or_default())
}

fn public_api(token: Option<Secret>) -> GitHubApi {
    match token {
        Some(token) if !token.is_empty() => GitHubApi::with_token(token.into_inner()),
        _ => GitHubApi::new(),
    }
}

impl GitHubBackend for LiveGitHub {
    fn execute(&self, request: GitHubRequest, emit: &mut dyn FnMut(GitHubResponse)) {
        let response = match request {
            GitHubRequest::LoadPullRequest {
                url,
                repo_path,
                token,
            } => {
                if let Ok(Some(comparison)) = self.cached_pull_request_comparison(&url, &repo_path)
                {
                    emit(GitHubResponse::PullRequestCached {
                        url: url.clone(),
                        comparison,
                    });
                }
                match self.load_pull_request(&url, &repo_path, token) {
                    Ok(comparison) => GitHubResponse::PullRequestLoaded { url, comparison },
                    Err(error) => GitHubResponse::PullRequestLoadFailed {
                        url,
                        message: error.user_message(),
                    },
                }
            }
            GitHubRequest::FetchUser { token } => {
                if token.is_empty() {
                    GitHubResponse::UserFetchFailed {
                        message: "cannot fetch GitHub user without a token".to_owned(),
                    }
                } else {
                    match GitHubApi::with_token(token.into_inner()).fetch_current_user() {
                        Ok(user) => GitHubResponse::UserFetched(user),
                        Err(error) => GitHubResponse::UserFetchFailed {
                            message: error.user_message(),
                        },
                    }
                }
            }
            GitHubRequest::PeekPullRequest { pr, token } => {
                match public_api(token).fetch_pull_request(&pr.owner, &pr.repo, pr.number) {
                    Ok(info) => GitHubResponse::PullRequestPeeked { pr, info },
                    Err(error) => GitHubResponse::PullRequestPeekFailed {
                        pr,
                        message: error.user_message(),
                    },
                }
            }
            GitHubRequest::FetchReviewComments { pr, token } => match public_api(token)
                .fetch_pull_request_review_comments(&pr.owner, &pr.repo, pr.number)
            {
                Ok(comments) => GitHubResponse::ReviewCommentsLoaded { pr, comments },
                Err(error) => GitHubResponse::ReviewCommentsLoadFailed {
                    pr,
                    message: error.user_message(),
                },
            },
            GitHubRequest::FetchReviewData { pr, token } => {
                match api(token).fetch_pull_request_review_data(&pr.owner, &pr.repo, pr.number) {
                    Ok(data) => GitHubResponse::ReviewDataLoaded {
                        pr,
                        data: Box::new(data),
                    },
                    Err(error) => GitHubResponse::ReviewDataLoadFailed {
                        pr,
                        message: error.user_message(),
                    },
                }
            }
            GitHubRequest::CreateReviewComment { pr, token, comment } => match api(token)
                .create_pull_request_review_comment(&pr.owner, &pr.repo, pr.number, &comment)
            {
                Ok(comment) => GitHubResponse::ReviewCommentCreated { pr, comment },
                Err(error) => GitHubResponse::ReviewCommentCreateFailed {
                    pr,
                    message: error.user_message(),
                },
            },
            GitHubRequest::CreateReviewReply {
                pr,
                comment_id,
                token,
                reply,
            } => match api(token).create_pull_request_review_reply(
                &pr.owner, &pr.repo, pr.number, comment_id, &reply,
            ) {
                Ok(comment) => GitHubResponse::ReviewCommentReplied { pr, comment },
                Err(error) => GitHubResponse::ReviewCommentReplyFailed {
                    pr,
                    message: error.user_message(),
                },
            },
            GitHubRequest::UpdateReviewComment {
                pr,
                comment_id,
                token,
                update,
            } => match api(token)
                .update_pull_request_review_comment(&pr.owner, &pr.repo, comment_id, &update)
            {
                Ok(comment) => GitHubResponse::ReviewCommentUpdated { pr, comment },
                Err(error) => GitHubResponse::ReviewCommentUpdateFailed {
                    pr,
                    comment_id,
                    message: error.user_message(),
                },
            },
            GitHubRequest::DeleteReviewComment {
                pr,
                comment_id,
                token,
            } => match api(token)
                .delete_pull_request_review_comment(&pr.owner, &pr.repo, comment_id)
            {
                Ok(()) => GitHubResponse::ReviewCommentDeleted { pr, comment_id },
                Err(error) => GitHubResponse::ReviewCommentDeleteFailed {
                    pr,
                    comment_id,
                    message: error.user_message(),
                },
            },
            GitHubRequest::CreateReview { pr, token, review } => match api(token)
                .create_pull_request_review(&pr.owner, &pr.repo, pr.number, &review)
            {
                Ok(review) => GitHubResponse::ReviewCreated { pr, review },
                Err(error) => GitHubResponse::ReviewCreateFailed {
                    pr,
                    message: error.user_message(),
                },
            },
            GitHubRequest::SubmitSessionDrafts {
                session,
                decision,
                body,
                token,
            } => {
                let pr = PullRequestRef {
                    owner: session.target.owner.clone(),
                    repo: session.target.repo.clone(),
                    number: session.target.number,
                };
                let draft_ids = session
                    .pending_drafts()
                    .map(|draft| draft.id)
                    .collect::<Vec<_>>();
                let result = session
                    .build_github_review_request(decision, body)
                    .and_then(|review| {
                        api(token)
                            .create_pull_request_review(&pr.owner, &pr.repo, pr.number, &review)
                    });
                match result {
                    Ok(review) => GitHubResponse::DraftsSubmitted {
                        pr,
                        review,
                        draft_ids,
                    },
                    Err(error) => GitHubResponse::DraftsSubmitFailed {
                        pr,
                        draft_ids,
                        message: error.user_message(),
                    },
                }
            }
            GitHubRequest::AddThreadReply {
                pr,
                thread_node_id,
                review_node_id,
                token,
                body,
            } => match api(token).add_pull_request_review_thread_reply(
                &thread_node_id,
                review_node_id.as_deref(),
                &body,
            ) {
                Ok(comment) => GitHubResponse::ThreadReplyAdded {
                    pr,
                    thread_node_id,
                    comment,
                },
                Err(error) => GitHubResponse::ThreadReplyAddFailed {
                    pr,
                    thread_node_id,
                    message: error.user_message(),
                },
            },
            GitHubRequest::UpdateCommentGraphql {
                pr,
                comment_node_id,
                token,
                body,
            } => match api(token)
                .update_pull_request_review_comment_graphql(&comment_node_id, &body)
            {
                Ok(comment) => GitHubResponse::CommentGraphqlUpdated { pr, comment },
                Err(error) => GitHubResponse::CommentGraphqlUpdateFailed {
                    pr,
                    comment_node_id,
                    message: error.user_message(),
                },
            },
            GitHubRequest::DeleteCommentGraphql {
                pr,
                comment_node_id,
                token,
            } => match api(token).delete_pull_request_review_comment_graphql(&comment_node_id) {
                Ok(comment) => GitHubResponse::CommentGraphqlDeleted {
                    pr,
                    comment_node_id,
                    comment,
                },
                Err(error) => GitHubResponse::CommentGraphqlDeleteFailed {
                    pr,
                    comment_node_id,
                    message: error.user_message(),
                },
            },
            GitHubRequest::SetThreadResolution {
                pr,
                thread_node_id,
                token,
                resolved,
            } => match api(token)
                .set_pull_request_review_thread_resolution(&thread_node_id, resolved)
            {
                Ok(resolution) => GitHubResponse::ThreadResolutionChanged { pr, resolution },
                Err(error) => GitHubResponse::ThreadResolutionChangeFailed {
                    pr,
                    thread_node_id,
                    message: error.user_message(),
                },
            },
            GitHubRequest::SubmitReview {
                pr,
                review_id,
                token,
                submit,
            } => match api(token)
                .submit_pull_request_review(&pr.owner, &pr.repo, pr.number, review_id, &submit)
            {
                Ok(review) => GitHubResponse::ReviewSubmitted { pr, review },
                Err(error) => GitHubResponse::ReviewSubmitFailed {
                    pr,
                    review_id,
                    message: error.user_message(),
                },
            },
            GitHubRequest::FetchPullRequestOverview { pr, token } => match public_api(token)
                .fetch_pull_request_overview(&pr.owner, &pr.repo, pr.number)
            {
                Ok(overview) => GitHubResponse::PullRequestOverviewLoaded {
                    pr,
                    overview: Box::new(overview),
                },
                Err(error) => GitHubResponse::PullRequestOverviewLoadFailed {
                    pr,
                    message: error.user_message(),
                },
            },
            GitHubRequest::CheckPullRequest {
                pr,
                token,
                head_sha,
            } => {
                let api = public_api(token);
                // Signed in, one GraphQL read sees comments and checks too;
                // signed out, GraphQL isn't open, so REST sees the rest.
                let now = if api.has_token() {
                    api.fetch_pull_request_fingerprint(&pr.owner, &pr.repo, pr.number)
                } else {
                    api.fetch_pull_request(&pr.owner, &pr.repo, pr.number)
                        .map(|info| PullRequestFingerprint {
                            title: info.title,
                            state: info.state,
                            merged: info.merged,
                            draft: info.draft,
                            head_sha: info.head_sha,
                            base_branch: info.base_branch,
                            ..PullRequestFingerprint::default()
                        })
                };
                match now {
                    Ok(now) => {
                        let change = if now.head_sha == head_sha {
                            HeadChange::Same
                        } else {
                            match api.compare_commits(&pr.owner, &pr.repo, &head_sha, &now.head_sha)
                            {
                                Ok(compared) if compared.status == "identical" => HeadChange::Same,
                                Ok(compared) if compared.status == "ahead" => {
                                    HeadChange::Ahead(compared.ahead_by)
                                }
                                Ok(_) => HeadChange::Rewritten,
                                Err(error) => {
                                    tracing::warn!("could not compare pull request heads: {error}");
                                    HeadChange::Moved
                                }
                            }
                        };
                        GitHubResponse::PullRequestChecked {
                            pr,
                            now: Box::new(now),
                            change,
                            remarks_changed: false,
                            checks_changed: false,
                        }
                    }
                    Err(error) => GitHubResponse::PullRequestCheckFailed {
                        pr,
                        message: error.user_message(),
                    },
                }
            }
            GitHubRequest::FetchMyPullRequests { token } => {
                match GitHubApi::with_token(token.into_inner()).fetch_my_pull_requests() {
                    Ok(pulls) => GitHubResponse::MyPullRequestsLoaded(pulls),
                    Err(error) => GitHubResponse::MyPullRequestsLoadFailed {
                        message: error.user_message(),
                    },
                }
            }
            GitHubRequest::AddPullRequestComment { pr, token, body } => {
                match api(token).add_pull_request_comment(&pr.owner, &pr.repo, pr.number, &body) {
                    Ok(()) => GitHubResponse::PullRequestCommentAdded { pr },
                    Err(error) => GitHubResponse::PullRequestCommentAddFailed {
                        pr,
                        body,
                        message: error.user_message(),
                    },
                }
            }
        };
        emit(response);
    }
}

/// Apply an anti-aliased circular alpha mask to a square-ish RGBA buffer in-place.
/// Pixels outside the inscribed circle are transparent; a 1-pixel band at the edge
/// is feathered by coverage so the circle renders smoothly.
fn apply_circular_mask(rgba: &mut [u8], width: u32, height: u32) {
    let cx = (width as f32 - 1.0) / 2.0;
    let cy = (height as f32 - 1.0) / 2.0;
    let radius = width.min(height) as f32 / 2.0 - 0.5;
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let coverage = (radius - dist + 0.5).clamp(0.0, 1.0);
            if coverage < 1.0 {
                let idx = ((y * width + x) * 4 + 3) as usize;
                if let Some(alpha) = rgba.get_mut(idx) {
                    *alpha = (f32::from(*alpha) * coverage).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}
