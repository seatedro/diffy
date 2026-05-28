use crate::actions::GitHubAction;
use crate::core::review::{ReviewResolution, ReviewThreadId};
use crate::effects::{Effect, GitHubEffect, UiEffect};
use crate::events::GitHubEvent;
use crate::ui::editor::render_doc::{INVALID_U32, RenderLine};
use crate::ui::editor::state::LineSelection;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: GitHubAction) -> Vec<Effect> {
    state.apply_github_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: GitHubEvent) -> Vec<Effect> {
    match event {
        GitHubEvent::PullRequestLoaded {
            url,
            info,
            left_ref,
            right_ref,
        } => {
            state
                .github
                .pull_request
                .status
                .set(&state.store, AsyncStatus::Ready);

            let key: PrKey = crate::core::forge::github::parse_pr_url(&url)
                .map(|p| (p.owner, p.repo, p.number))
                .unwrap_or_else(|| (String::new(), String::new(), info.number));
            let target = ReviewTarget::github(key.0.clone(), key.1.clone(), key.2);
            state.github.pull_request.cache.update(&state.store, |c| {
                let entry = c.entry(key.clone()).or_insert_with(|| PrCacheEntry {
                    meta: PrPeekMeta::Ready(info.clone()),
                    diff: PrPeekDiff::Idle,
                    last_peek_ms: 0,
                });
                entry.meta = PrPeekMeta::Ready(info.clone());
                entry.diff = PrPeekDiff::Ready {
                    url: url.clone(),
                    left_ref: left_ref.clone(),
                    right_ref: right_ref.clone(),
                    info: info.clone(),
                };
            });
            state
                .github
                .pull_request
                .review_comments
                .update(&state.store, |comments| {
                    let entry = comments.entry(key.clone()).or_default();
                    entry.status = AsyncStatus::Loading;
                    entry.message = None;
                });
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    let session = sessions.entry(key.clone()).or_insert_with(|| {
                        ReviewSession::new(
                            ReviewTarget::github(key.0.clone(), key.1.clone(), key.2),
                            info.clone(),
                        )
                    });
                    session.pull_request = info.clone();
                    session.status = crate::core::review::ReviewSessionStatus::Loading;
                    session.status_message = None;
                });
            let mut effects: Vec<Effect> = vec![
                GitHubEffect::LoadReviewSession {
                    target: target.clone(),
                    pull_request: info.clone(),
                }
                .into(),
                GitHubEffect::FetchPullRequestReviewComments {
                    owner: key.0.clone(),
                    repo: key.1.clone(),
                    number: key.2,
                    github_token: state.github_access_token.clone(),
                }
                .into(),
                GitHubEffect::FetchPullRequestReviewData {
                    owner: key.0.clone(),
                    repo: key.1.clone(),
                    number: key.2,
                    github_token: state.github_access_token.clone(),
                }
                .into(),
            ];

            let pending_match = state
                .github
                .pull_request
                .pending_confirm
                .with(&state.store, |p| p.as_ref() == Some(&key));
            if pending_match {
                state
                    .github
                    .pull_request
                    .pending_confirm
                    .set(&state.store, None);
                state
                    .github
                    .pull_request
                    .active
                    .set(&state.store, Some(key));
                effects.extend(state.apply_pr_compare(left_ref, right_ref));
                return effects;
            }
            effects.extend(state.rebuild_command_palette_if_open());
            effects
        }
        GitHubEvent::PullRequestLoadFailed { url, message } => {
            state
                .github
                .pull_request
                .status
                .set(&state.store, AsyncStatus::Failed);
            if let Some(parsed) = crate::core::forge::github::parse_pr_url(&url) {
                let key: PrKey = (parsed.owner, parsed.repo, parsed.number);
                state.github.pull_request.cache.update(&state.store, |c| {
                    if let Some(entry) = c.get_mut(&key) {
                        entry.diff = PrPeekDiff::Failed(message.clone());
                    }
                });
                let pending_match = state
                    .github
                    .pull_request
                    .pending_confirm
                    .with(&state.store, |p| p.as_ref() == Some(&key));
                if pending_match {
                    state
                        .github
                        .pull_request
                        .pending_confirm
                        .set(&state.store, None);
                }
            }
            state.push_error(&message);
            state.rebuild_command_palette_if_open()
        }
        GitHubEvent::PullRequestPeeked {
            owner,
            repo,
            number,
            info,
        } => {
            let key: PrKey = (owner, repo, number);
            state.github.pull_request.cache.update(&state.store, |c| {
                if let Some(entry) = c.get_mut(&key) {
                    entry.meta = PrPeekMeta::Ready(info);
                }
            });
            state.rebuild_command_palette_if_open()
        }
        GitHubEvent::PullRequestPeekFailed {
            owner,
            repo,
            number,
            message,
        } => {
            let key: PrKey = (owner, repo, number);
            state.github.pull_request.cache.update(&state.store, |c| {
                if let Some(entry) = c.get_mut(&key) {
                    entry.meta = PrPeekMeta::Failed(message);
                }
            });
            state.rebuild_command_palette_if_open()
        }
        GitHubEvent::PullRequestReviewCommentsLoaded {
            owner,
            repo,
            number,
            comments,
        } => {
            let key: PrKey = (owner, repo, number);
            let info = state.github.pull_request.cache.with(&state.store, |cache| {
                match cache.get(&key).map(|entry| &entry.meta) {
                    Some(PrPeekMeta::Ready(info)) => Some(info.clone()),
                    _ => None,
                }
            });
            let session_comments = comments.clone();
            state
                .github
                .pull_request
                .review_comments
                .update(&state.store, |map| {
                    map.insert(
                        key.clone(),
                        PrReviewCommentsEntry {
                            status: AsyncStatus::Ready,
                            comments,
                            message: None,
                        },
                    );
                });
            if let Some(info) = info {
                state
                    .github
                    .pull_request
                    .review_sessions
                    .update(&state.store, |sessions| {
                        let session = sessions.entry(key.clone()).or_insert_with(|| {
                            ReviewSession::new(
                                ReviewTarget::github(key.0.clone(), key.1.clone(), key.2),
                                info,
                            )
                        });
                        session.replace_github_comments(session_comments);
                    });
            }
            save_review_session_effect(state, &key)
        }
        GitHubEvent::PullRequestReviewDataLoaded {
            owner,
            repo,
            number,
            data,
        } => {
            let key: PrKey = (owner, repo, number);
            let info = state.github.pull_request.cache.with(&state.store, |cache| {
                match cache.get(&key).map(|entry| &entry.meta) {
                    Some(PrPeekMeta::Ready(info)) => Some(info.clone()),
                    _ => None,
                }
            });
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.apply_github_review_data(data);
                    } else if let Some(info) = info {
                        let mut session = ReviewSession::new(
                            ReviewTarget::github(key.0.clone(), key.1.clone(), key.2),
                            info,
                        );
                        session.apply_github_review_data(data);
                        sessions.insert(key.clone(), session);
                    }
                });
            save_review_session_effect(state, &key)
        }
        GitHubEvent::PullRequestReviewDataLoadFailed {
            owner,
            repo,
            number,
            message,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.status_message = Some(message.clone());
                    }
                });
            tracing::warn!("failed to fetch pull request review data: {message}");
            Vec::new()
        }
        GitHubEvent::PullRequestReviewCommentsLoadFailed {
            owner,
            repo,
            number,
            message,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_comments
                .update(&state.store, |map| {
                    let entry = map.entry(key.clone()).or_default();
                    entry.status = AsyncStatus::Failed;
                    entry.message = Some(message.clone());
                });
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.status = crate::core::review::ReviewSessionStatus::Failed;
                        session.status_message = Some(message.clone());
                    }
                });
            tracing::warn!("failed to fetch pull request review comments: {message}");
            Vec::new()
        }
        GitHubEvent::PullRequestReviewCommentCreated {
            owner,
            repo,
            number,
            comment,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_comments
                .update(&state.store, |map| {
                    let entry = map.entry(key.clone()).or_default();
                    entry.status = AsyncStatus::Ready;
                    entry.message = None;
                    entry.comments.push(comment.clone());
                });
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.apply_github_comment(comment.clone());
                    }
                });
            state
                .github
                .pull_request
                .review_composer
                .set(&state.store, ReviewCommentComposerState::default());
            state.review_comment_editor.request_clear();
            state
                .editor
                .line_selection
                .update(&state.store, |ls| ls.clear());
            state.set_focus(Some(FocusTarget::Editor));
            state.push_info("Review comment posted.");
            save_review_session_effect(state, &key)
        }
        GitHubEvent::PullRequestReviewCommentCreateFailed {
            owner: _,
            repo: _,
            number: _,
            message,
        } => {
            state
                .github
                .pull_request
                .review_composer
                .update(&state.store, |composer| {
                    composer.status = AsyncStatus::Failed;
                    composer.message = Some(message.clone());
                });
            state.push_error(&message);
            Vec::new()
        }
        GitHubEvent::PullRequestReviewCommentReplied {
            owner,
            repo,
            number,
            comment,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_comments
                .update(&state.store, |map| {
                    let entry = map.entry(key.clone()).or_default();
                    entry.status = AsyncStatus::Ready;
                    entry.message = None;
                    entry.comments.push(comment.clone());
                });
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.apply_github_comment(comment.clone());
                    }
                });
            save_review_session_effect(state, &key)
        }
        GitHubEvent::PullRequestReviewCommentReplyFailed {
            owner: _,
            repo: _,
            number: _,
            message,
        } => {
            state.push_error(&message);
            Vec::new()
        }
        GitHubEvent::PullRequestReviewCommentUpdated {
            owner,
            repo,
            number,
            comment,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_comments
                .update(&state.store, |map| {
                    if let Some(entry) = map.get_mut(&key)
                        && let Some(existing) = entry
                            .comments
                            .iter_mut()
                            .find(|existing| existing.id == comment.id)
                    {
                        *existing = comment;
                    }
                });
            vec![
                GitHubEffect::FetchPullRequestReviewData {
                    owner: key.0.clone(),
                    repo: key.1.clone(),
                    number: key.2,
                    github_token: state.github_access_token.clone(),
                }
                .into(),
            ]
        }
        GitHubEvent::PullRequestReviewCommentUpdateFailed {
            owner: _,
            repo: _,
            number: _,
            comment_id: _,
            message,
        } => {
            state.push_error(&message);
            Vec::new()
        }
        GitHubEvent::PullRequestReviewCommentDeleted {
            owner,
            repo,
            number,
            comment_id,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_comments
                .update(&state.store, |map| {
                    if let Some(entry) = map.get_mut(&key) {
                        entry.comments.retain(|comment| comment.id != comment_id);
                    }
                });
            vec![
                GitHubEffect::FetchPullRequestReviewData {
                    owner: key.0.clone(),
                    repo: key.1.clone(),
                    number: key.2,
                    github_token: state.github_access_token.clone(),
                }
                .into(),
            ]
        }
        GitHubEvent::PullRequestReviewCommentDeleteFailed {
            owner: _,
            repo: _,
            number: _,
            comment_id: _,
            message,
        } => {
            state.push_error(&message);
            Vec::new()
        }
        GitHubEvent::PullRequestReviewCreated {
            owner,
            repo,
            number,
            review: _,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.status = crate::core::review::ReviewSessionStatus::Ready;
                        session.status_message = None;
                    }
                });
            vec![
                GitHubEffect::FetchPullRequestReviewData {
                    owner: key.0.clone(),
                    repo: key.1.clone(),
                    number: key.2,
                    github_token: state.github_access_token.clone(),
                }
                .into(),
            ]
        }
        GitHubEvent::PullRequestReviewCreateFailed {
            owner,
            repo,
            number,
            message,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.status = crate::core::review::ReviewSessionStatus::Failed;
                        session.status_message = Some(message.clone());
                    }
                });
            state.push_error(&message);
            Vec::new()
        }
        GitHubEvent::PullRequestReviewDraftsSubmitted {
            owner,
            repo,
            number,
            review: _,
            draft_ids,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.status = crate::core::review::ReviewSessionStatus::Ready;
                        session.status_message = None;
                        for draft_id in &draft_ids {
                            session.mark_draft_submitted(*draft_id, None);
                        }
                    }
                });
            let mut effects = save_review_session_effect(state, &key);
            effects.push(
                GitHubEffect::FetchPullRequestReviewData {
                    owner: key.0.clone(),
                    repo: key.1.clone(),
                    number: key.2,
                    github_token: state.github_access_token.clone(),
                }
                .into(),
            );
            effects
        }
        GitHubEvent::PullRequestReviewDraftsSubmitFailed {
            owner,
            repo,
            number,
            draft_ids,
            message,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.status = crate::core::review::ReviewSessionStatus::Failed;
                        session.status_message = Some(message.clone());
                        for draft_id in &draft_ids {
                            session.mark_draft_failed(*draft_id, message.clone());
                        }
                    }
                });
            state.push_error(&message);
            save_review_session_effect(state, &key)
        }
        GitHubEvent::PullRequestReviewSubmitted {
            owner,
            repo,
            number,
            review: _,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.status = crate::core::review::ReviewSessionStatus::Ready;
                        session.status_message = None;
                    }
                });
            vec![
                GitHubEffect::FetchPullRequestReviewData {
                    owner: key.0.clone(),
                    repo: key.1.clone(),
                    number: key.2,
                    github_token: state.github_access_token.clone(),
                }
                .into(),
            ]
        }
        GitHubEvent::PullRequestReviewSubmitFailed {
            owner,
            repo,
            number,
            review_id: _,
            message,
        } => {
            let key: PrKey = (owner, repo, number);
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.status = crate::core::review::ReviewSessionStatus::Failed;
                        session.status_message = Some(message.clone());
                    }
                });
            state.push_error(&message);
            Vec::new()
        }
        GitHubEvent::PullRequestReviewThreadReplyAdded {
            owner,
            repo,
            number,
            thread_node_id: _,
            comment: _,
        }
        | GitHubEvent::PullRequestReviewCommentGraphqlUpdated {
            owner,
            repo,
            number,
            comment: _,
        } => vec![
            GitHubEffect::FetchPullRequestReviewData {
                owner,
                repo,
                number,
                github_token: state.github_access_token.clone(),
            }
            .into(),
        ],
        GitHubEvent::PullRequestReviewCommentGraphqlDeleted {
            owner,
            repo,
            number,
            comment_node_id: _,
            comment: _,
        } => vec![
            GitHubEffect::FetchPullRequestReviewData {
                owner,
                repo,
                number,
                github_token: state.github_access_token.clone(),
            }
            .into(),
        ],
        GitHubEvent::PullRequestReviewThreadReplyAddFailed {
            owner: _,
            repo: _,
            number: _,
            thread_node_id: _,
            message,
        }
        | GitHubEvent::PullRequestReviewCommentGraphqlUpdateFailed {
            owner: _,
            repo: _,
            number: _,
            comment_node_id: _,
            message,
        }
        | GitHubEvent::PullRequestReviewCommentGraphqlDeleteFailed {
            owner: _,
            repo: _,
            number: _,
            comment_node_id: _,
            message,
        }
        | GitHubEvent::PullRequestReviewThreadResolutionChangeFailed {
            owner: _,
            repo: _,
            number: _,
            thread_node_id: _,
            message,
        } => {
            state.push_error(&message);
            Vec::new()
        }
        GitHubEvent::PullRequestReviewThreadResolutionChanged {
            owner,
            repo,
            number,
            resolution,
        } => {
            let key: PrKey = (owner, repo, number);
            let thread_id = ReviewThreadId::github_node(resolution.thread_node_id);
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(session) = sessions.get_mut(&key) {
                        session.mark_thread_resolution(
                            &thread_id,
                            if resolution.is_resolved {
                                ReviewResolution::Resolved
                            } else {
                                ReviewResolution::Unresolved
                            },
                        );
                    }
                });
            save_review_session_effect(state, &key)
        }
        GitHubEvent::ReviewSessionLoaded { target, session } => {
            let key: PrKey = (target.owner, target.repo, target.number);
            state
                .github
                .pull_request
                .review_sessions
                .update(&state.store, |sessions| {
                    if let Some(current) = sessions.get_mut(&key) {
                        current.merge_persisted_state(session);
                    } else {
                        sessions.insert(key, session);
                    }
                });
            Vec::new()
        }
        GitHubEvent::ReviewSessionLoadFailed { target: _, message } => {
            tracing::warn!("failed to load review session: {message}");
            Vec::new()
        }
        GitHubEvent::ReviewSessionSaved { key: _ } => Vec::new(),
        GitHubEvent::ReviewSessionSaveFailed { key: _, message } => {
            tracing::warn!("failed to save review session: {message}");
            Vec::new()
        }
        GitHubEvent::DeviceFlowStarted(device_flow) => {
            state
                .github
                .auth
                .status
                .set(&state.store, AsyncStatus::Loading);
            state
                .github
                .auth
                .device_flow
                .set(&state.store, Some(device_flow.clone()));
            vec![
                UiEffect::OpenBrowser {
                    url: device_flow.verification_uri.clone(),
                }
                .into(),
                GitHubEffect::PollDeviceFlow {
                    client_id: state.github.client_id.get(&state.store),
                    device_code: device_flow.device_code,
                    interval_seconds: device_flow.interval,
                }
                .into(),
            ]
        }
        GitHubEvent::DeviceFlowStartFailed { message } => {
            state
                .github
                .auth
                .status
                .set(&state.store, AsyncStatus::Failed);
            state.push_error(&message);
            Vec::new()
        }
        GitHubEvent::DeviceFlowCompleted { token } => {
            state
                .github
                .auth
                .status
                .set(&state.store, AsyncStatus::Ready);
            state.github.auth.device_flow.set(&state.store, None);
            state.github.auth.token_present.set(&state.store, true);
            state.github_access_token = Some(token.clone());
            state.push_info("GitHub authentication completed.");
            if state.overlays_top() == Some(OverlaySurface::GitHubAuthModal) {
                state.pop_overlay();
            }
            let mut effects = state.persist_settings_effect();
            if state.startup.keyring_enabled {
                effects.push(GitHubEffect::SaveGitHubToken(token.clone()).into());
            }
            effects.push(GitHubEffect::FetchGitHubUser { token }.into());
            effects
        }
        GitHubEvent::DeviceFlowFailed { message } => {
            state
                .github
                .auth
                .status
                .set(&state.store, AsyncStatus::Failed);
            state.push_error(&message);
            Vec::new()
        }
        GitHubEvent::GitHubTokenLoaded { token } => {
            state.github_access_token = token.clone();
            let has_token = token.is_some();
            state.github.auth.token_present.set(&state.store, has_token);
            let mut effects = Vec::new();
            if let Some(token) = token
                && state.github.auth.user.with(&state.store, |u| u.is_none())
            {
                effects.push(GitHubEffect::FetchGitHubUser { token }.into());
            }
            effects
        }
        GitHubEvent::GitHubTokenLoadFailed { message } => {
            state.github_access_token = None;
            state.github.auth.token_present.set(&state.store, false);
            state.push_error(&format!("Keyring unavailable: {message}"));
            Vec::new()
        }
        GitHubEvent::GitHubTokenSaveFailed { message } => {
            state.github_access_token = None;
            state.github.auth.token_present.set(&state.store, false);
            state.github.auth.user.set(&state.store, None);
            state.settings.github_user = None;
            state.push_error(&format!("Couldn't save GitHub token to keyring: {message}"));
            state.persist_settings_effect()
        }
        GitHubEvent::GitHubUserFetched { user } => {
            let avatar_src = avatar_url_sized(&user.avatar_url, 128);
            let previous_login = state
                .github
                .auth
                .user
                .with(&state.store, |u| u.as_ref().map(|u| u.login.clone()));
            state.github.auth.user.set(&state.store, Some(user.clone()));
            state.settings.github_user = Some(user.clone());
            if previous_login
                .as_deref()
                .is_some_and(|login| login != user.login.as_str())
            {
                state
                    .github
                    .pull_request
                    .cache
                    .update(&state.store, |c| c.clear());
                state.github.pull_request.active.set(&state.store, None);
                state
                    .github
                    .pull_request
                    .review_comments
                    .update(&state.store, |c| c.clear());
                state
                    .github
                    .pull_request
                    .review_sessions
                    .update(&state.store, |c| c.clear());
                state
                    .github
                    .pull_request
                    .review_composer
                    .set(&state.store, ReviewCommentComposerState::default());
                state.review_comment_editor.request_clear();
            }
            let mut effects = state.persist_settings_effect();
            if let Some(url) = avatar_src {
                let already_have = state
                    .github
                    .auth
                    .avatar
                    .with(&state.store, |a| a.as_ref().is_some_and(|b| b.url == url));
                if !already_have && !state.github.auth.avatar_fetching.get(&state.store) {
                    state.github.auth.avatar_fetching.set(&state.store, true);
                    effects.push(GitHubEffect::FetchAvatar { url }.into());
                }
            }
            effects
        }
        GitHubEvent::GitHubUserFetchFailed { message } => {
            tracing::warn!("failed to fetch GitHub user: {message}");
            Vec::new()
        }
        GitHubEvent::AvatarFetched {
            url,
            rgba,
            width,
            height,
        } => {
            state.github.auth.avatar_fetching.set(&state.store, false);
            let cache_key = avatar_cache_key(&url);
            state.github.auth.avatar.set(
                &state.store,
                Some(AvatarBitmap {
                    url,
                    rgba,
                    width,
                    height,
                    cache_key,
                }),
            );
            Vec::new()
        }
        GitHubEvent::AvatarFetchFailed { url, message } => {
            state.github.auth.avatar_fetching.set(&state.store, false);
            tracing::warn!("failed to fetch avatar {url}: {message}");
            Vec::new()
        }
    }
}

impl AppState {
    pub(super) fn apply_github_action(&mut self, action: GitHubAction) -> Vec<Effect> {
        match action {
            GitHubAction::StartGitHubDeviceFlow => {
                self.github
                    .auth
                    .status
                    .set(&self.store, AsyncStatus::Loading);
                // Surface the auth modal so the user sees the device code once the
                // HTTP call returns. Without this the browser opens but the user has
                // no way to see the code they need to type.
                if self.overlays_top() != Some(OverlaySurface::GitHubAuthModal) {
                    self.push_overlay(
                        OverlaySurface::GitHubAuthModal,
                        Some(FocusTarget::AuthPrimaryAction),
                    );
                }
                vec![
                    GitHubEffect::StartDeviceFlow {
                        client_id: self.github.client_id.get(&self.store),
                    }
                    .into(),
                ]
            }
            GitHubAction::OpenDeviceFlowBrowser => {
                let verification_uri = self.github.auth.device_flow.with(&self.store, |opt| {
                    opt.as_ref().map(|df| df.verification_uri.clone())
                });
                if let Some(url) = verification_uri {
                    vec![UiEffect::OpenBrowser { url }.into()]
                } else {
                    Vec::new()
                }
            }
            GitHubAction::OpenAccountMenu => {
                self.push_overlay(OverlaySurface::AccountMenu, None);
                Vec::new()
            }
            GitHubAction::SignOutGitHub => {
                self.github.auth.token_present.set(&self.store, false);
                self.github.auth.user.set(&self.store, None);
                self.github.auth.avatar.set(&self.store, None);
                self.github.auth.avatar_fetching.set(&self.store, false);
                self.github.auth.device_flow.set(&self.store, None);
                self.github.auth.status.set(&self.store, AsyncStatus::Idle);
                // Stale peek/load errors from an unauthenticated session shouldn't
                // linger across sign-in transitions — drop the cache so the user
                // re-runs the flow with the new credentials.
                self.github
                    .pull_request
                    .cache
                    .update(&self.store, |c| c.clear());
                self.github
                    .pull_request
                    .pending_confirm
                    .set(&self.store, None);
                self.github.pull_request.active.set(&self.store, None);
                self.github
                    .pull_request
                    .review_comments
                    .update(&self.store, |c| c.clear());
                self.github
                    .pull_request
                    .review_sessions
                    .update(&self.store, |c| c.clear());
                self.github
                    .pull_request
                    .review_composer
                    .set(&self.store, ReviewCommentComposerState::default());
                self.review_comment_editor.request_clear();
                self.github_access_token = None;
                self.settings.github_user = None;
                self.push_info("Signed out of GitHub.");
                let mut effects = self.persist_settings_effect();
                effects.push(GitHubEffect::ClearGitHubToken.into());
                effects
            }
            GitHubAction::OpenReviewCommentComposer => self.open_review_comment_composer(),
            GitHubAction::SubmitReviewComment => self.submit_review_comment(),
            GitHubAction::CancelReviewComment => {
                self.github
                    .pull_request
                    .review_composer
                    .set(&self.store, ReviewCommentComposerState::default());
                self.review_comment_editor.request_clear();
                self.set_focus(Some(FocusTarget::Editor));
                Vec::new()
            }
        }
    }
}

impl AppState {
    pub(crate) fn active_pull_request_key(&self) -> Option<PrKey> {
        self.github.pull_request.active.get(&self.store)
    }

    pub(crate) fn pull_request_review_enabled(&self) -> bool {
        self.workspace.source.get(&self.store) == WorkspaceSource::Compare
            && self.active_pull_request_key().is_some()
    }

    pub(crate) fn active_pr_review_comments_for_file(
        &self,
        file: &carbon::FileDiff,
    ) -> Vec<PullRequestReviewComment> {
        let Some(key) = self.active_pull_request_key() else {
            return Vec::new();
        };
        let old_path = file.old_path.as_deref();
        let new_path = file.new_path.as_deref();
        self.github
            .pull_request
            .review_comments
            .with(&self.store, |map| {
                map.get(&key)
                    .map(|entry| {
                        entry
                            .comments
                            .iter()
                            .filter(|comment| {
                                Some(comment.path.as_str()) == old_path
                                    || Some(comment.path.as_str()) == new_path
                            })
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default()
            })
    }

    fn open_review_comment_composer(&mut self) -> Vec<Effect> {
        if self
            .github_access_token
            .as_deref()
            .unwrap_or_default()
            .is_empty()
        {
            self.push_overlay(
                OverlaySurface::GitHubAuthModal,
                Some(FocusTarget::AuthPrimaryAction),
            );
            self.push_info("Sign in to add review comments.");
            return Vec::new();
        }

        let Some(draft) = self.build_review_comment_draft(String::new()) else {
            return Vec::new();
        };
        self.github.pull_request.review_composer.set(
            &self.store,
            ReviewCommentComposerState {
                draft: Some(draft),
                status: AsyncStatus::Ready,
                message: None,
            },
        );
        self.review_comment_editor.request_clear();
        self.set_focus(Some(FocusTarget::ReviewCommentEditor));
        Vec::new()
    }

    fn submit_review_comment(&mut self) -> Vec<Effect> {
        let body = self.review_comment_editor.text().trim().to_owned();
        if body.is_empty() {
            self.push_error("Write a comment before submitting.");
            return Vec::new();
        }
        let token = match self.github_access_token.clone() {
            Some(token) if !token.is_empty() => token,
            _ => {
                self.push_overlay(
                    OverlaySurface::GitHubAuthModal,
                    Some(FocusTarget::AuthPrimaryAction),
                );
                self.push_info("Sign in to add review comments.");
                return Vec::new();
            }
        };

        let Some(mut draft) = self
            .github
            .pull_request
            .review_composer
            .with(&self.store, |composer| composer.draft.clone())
            .or_else(|| self.build_review_comment_draft(String::new()))
        else {
            return Vec::new();
        };
        draft.request.body = body;
        let (owner, repo, number) = draft.key.clone();
        self.github
            .pull_request
            .review_composer
            .update(&self.store, |composer| {
                composer.draft = Some(draft.clone());
                composer.status = AsyncStatus::Loading;
                composer.message = None;
            });
        vec![
            GitHubEffect::CreatePullRequestReviewComment {
                owner,
                repo,
                number,
                github_token: Some(token),
                comment: draft.request,
            }
            .into(),
        ]
    }

    fn build_review_comment_draft(&mut self, body: String) -> Option<ReviewCommentDraft> {
        let key = self.active_pull_request_key()?;
        let info = self.github.pull_request.cache.with(&self.store, |cache| {
            match cache.get(&key).map(|entry| &entry.meta) {
                Some(PrPeekMeta::Ready(info)) => Some(info.clone()),
                _ => None,
            }
        })?;
        let line_selection = self.editor.line_selection.get(&self.store);
        let Some((path, side, line, start_line)) =
            self.workspace.active_file.with(&self.store, |af| {
                let active = af.as_ref()?;
                selected_review_range(
                    &active.carbon_file,
                    &active.render_doc.lines,
                    &line_selection,
                )
            })
        else {
            self.push_error("Select one or more changed lines on one side of the diff.");
            return None;
        };

        Some(ReviewCommentDraft {
            key,
            request: CreatePullRequestReviewComment {
                body,
                commit_id: info.head_sha,
                path,
                line,
                side,
                start_line,
                start_side: start_line.map(|_| side),
            },
        })
    }
}

fn selected_review_range(
    file: &carbon::FileDiff,
    lines: &[RenderLine],
    selection: &LineSelection,
) -> Option<(String, GitHubReviewSide, u32, Option<u32>)> {
    let mut selected = Vec::new();
    for line in lines {
        if line.hunk_index < 0 {
            continue;
        }
        let hunk_id = line.hunk_index as u32;
        let new_selected = line.new_line_index >= 0
            && selection.contains(hunk_id, carbon::DiffSide::New, line.new_line_index as u32)
            && line.new_line_no != INVALID_U32;
        let old_selected = line.old_line_index >= 0
            && selection.contains(hunk_id, carbon::DiffSide::Old, line.old_line_index as u32)
            && line.old_line_no != INVALID_U32;

        if new_selected {
            selected.push((hunk_id, GitHubReviewSide::Right, line.new_line_no));
        } else if old_selected {
            selected.push((hunk_id, GitHubReviewSide::Left, line.old_line_no));
        }
    }

    selected.sort_unstable();
    selected.dedup();
    let (hunk_id, side, first_line) = selected.first().copied()?;
    if selected.iter().any(|(candidate_hunk, candidate_side, _)| {
        *candidate_hunk != hunk_id || *candidate_side != side
    }) {
        return None;
    }
    let line = selected
        .last()
        .map(|(_, _, line)| *line)
        .unwrap_or(first_line);
    let start_line = (first_line != line).then_some(first_line);
    let path = match side {
        GitHubReviewSide::Right => file.new_path.as_ref().or(file.old_path.as_ref()),
        GitHubReviewSide::Left => file.old_path.as_ref().or(file.new_path.as_ref()),
    }?
    .clone();
    Some((path, side, line, start_line))
}

fn save_review_session_effect(state: &AppState, key: &PrKey) -> Vec<Effect> {
    state
        .github
        .pull_request
        .review_sessions
        .with(&state.store, |sessions| {
            sessions
                .get(key)
                .cloned()
                .map(|session| GitHubEffect::SaveReviewSession { session }.into())
                .into_iter()
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::editor::render_doc::RenderRowKind;
    use crate::ui::editor::state::LineSelectionKey;

    fn file() -> carbon::FileDiff {
        carbon::FileDiff {
            old_path: Some("old.rs".to_owned()),
            new_path: Some("new.rs".to_owned()),
            ..carbon::FileDiff::default()
        }
    }

    #[test]
    fn review_comment_range_prefers_right_side_for_modified_rows() {
        let lines = vec![RenderLine {
            kind: RenderRowKind::Modified as u8,
            hunk_index: 0,
            old_line_no: 10,
            new_line_no: 12,
            old_line_index: 9,
            new_line_index: 11,
            ..RenderLine::default()
        }];
        let mut selection = LineSelection::default();
        selection.entries.insert(LineSelectionKey {
            hunk_id: 0,
            side: carbon::DiffSide::Old,
            source_index: 9,
        });
        selection.entries.insert(LineSelectionKey {
            hunk_id: 0,
            side: carbon::DiffSide::New,
            source_index: 11,
        });

        let range = selected_review_range(&file(), &lines, &selection).unwrap();

        assert_eq!(range.0, "new.rs");
        assert_eq!(range.1, GitHubReviewSide::Right);
        assert_eq!(range.2, 12);
        assert_eq!(range.3, None);
    }

    #[test]
    fn review_comment_range_uses_left_side_for_removed_rows() {
        let lines = vec![
            RenderLine {
                kind: RenderRowKind::Removed as u8,
                hunk_index: 0,
                old_line_no: 5,
                new_line_no: INVALID_U32,
                old_line_index: 4,
                new_line_index: -1,
                ..RenderLine::default()
            },
            RenderLine {
                kind: RenderRowKind::Removed as u8,
                hunk_index: 0,
                old_line_no: 6,
                new_line_no: INVALID_U32,
                old_line_index: 5,
                new_line_index: -1,
                ..RenderLine::default()
            },
        ];
        let mut selection = LineSelection::default();
        selection.entries.insert(LineSelectionKey {
            hunk_id: 0,
            side: carbon::DiffSide::Old,
            source_index: 4,
        });
        selection.entries.insert(LineSelectionKey {
            hunk_id: 0,
            side: carbon::DiffSide::Old,
            source_index: 5,
        });

        let range = selected_review_range(&file(), &lines, &selection).unwrap();

        assert_eq!(range.0, "old.rs");
        assert_eq!(range.1, GitHubReviewSide::Left);
        assert_eq!(range.2, 6);
        assert_eq!(range.3, Some(5));
    }
}
