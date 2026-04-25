use crate::actions::GitHubAction;
use crate::effects::{Effect, GitHubEffect, UiEffect};
use crate::events::GitHubEvent;

use super::{
    AppState, AsyncStatus, AvatarBitmap, OverlaySurface, PrCacheEntry, PrKey, PrPeekDiff,
    PrPeekMeta, avatar_cache_key, avatar_url_sized,
};

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

            let key: PrKey = crate::core::vcs::github::parse_pr_url(&url)
                .map(|p| (p.owner, p.repo, p.number))
                .unwrap_or_else(|| (String::new(), String::new(), info.number));
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
                return state.apply_pr_compare(left_ref, right_ref);
            }
            state.rebuild_command_palette_if_open()
        }
        GitHubEvent::PullRequestLoadFailed { url, message } => {
            state
                .github
                .pull_request
                .status
                .set(&state.store, AsyncStatus::Failed);
            if let Some(parsed) = crate::core::vcs::github::parse_pr_url(&url) {
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
            effects.push(GitHubEffect::SaveGitHubToken(token.clone()).into());
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
            if previous_login.as_deref() != Some(user.login.as_str()) {
                state
                    .github
                    .pull_request
                    .cache
                    .update(&state.store, |c| c.clear());
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
