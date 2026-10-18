use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::contracts::review::{
    AvatarImage, GitHubRequest, GitHubResponse, PullRequestComparison, PullRequestRef, Secret,
};
use crate::core::compare::{LayoutMode, RendererKind};
use crate::core::error::{DiffyError, Result};
use crate::core::forge::github::{
    DeviceFlowState, GitHubApi, PullRequestInfo, parse_pr_url, poll_for_token, start_device_flow,
};
use crate::core::http;
use crate::core::review::{ReviewSession, ReviewSessionKey, ReviewTarget};
use crate::core::vcs::discovery;
use crate::core::vcs::git::pr_ref_path;
use crate::core::vcs::model::{VcsCompareRequest, VcsCompareSpec, VcsKind};
use crate::platform::persistence::SettingsStore;
use crate::platform::review_store::ReviewStore;
use crate::platform::secrets;

const DEV_GITHUB_TOKEN_FILE_NAME: &str = "github-token.dev";

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
        let parent = self.settings_store.path().parent().ok_or_else(|| {
            DiffyError::General(format!(
                "settings path has no parent directory: {}",
                self.settings_store.path().display()
            ))
        })?;
        Ok(parent.join(DEV_GITHUB_TOKEN_FILE_NAME))
    }

    pub fn load_session(
        &self,
        target: ReviewTarget,
        pull_request: PullRequestInfo,
    ) -> Result<ReviewSession> {
        Ok(self
            .review_store
            .load_session(&target, &pull_request.head_sha)?
            .unwrap_or_else(|| ReviewSession::new(target, pull_request)))
    }

    pub fn save_session(&self, session: &ReviewSession) -> Result<ReviewSessionKey> {
        let key = session.key();
        self.review_store.save_session(session)?;
        Ok(key)
    }

    pub fn fetch_avatar(&self, url: &str) -> Result<AvatarImage> {
        let bytes = http::block_on(async {
            let response = reqwest::Client::new()
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
        let (info, left_ref, right_ref) = repo.resolve_pull_request_comparison(url, &token)?;
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
            left_ref,
            right_ref,
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

        for session in sessions.into_iter().rev() {
            let info = session.pull_request;
            if info.base_branch.is_empty() || info.head_branch.is_empty() {
                continue;
            }
            let left_ref = pr_ref_path(info.number, &info.base_branch);
            let right_ref = pr_ref_path(info.number, &info.head_branch);
            let request = VcsCompareRequest {
                spec: VcsCompareSpec::MergeBaseRange {
                    base: left_ref.clone(),
                    head: right_ref.clone(),
                },
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
                return Ok(Some(PullRequestComparison {
                    info,
                    left_ref,
                    right_ref,
                }));
            }
        }

        Ok(None)
    }
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
