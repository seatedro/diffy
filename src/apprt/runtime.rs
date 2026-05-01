use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::apprt::compare::CompareScheduler;
use crate::apprt::git_worker::GitWorker;
use crate::apprt::services::AppServices;
use crate::apprt::watcher::RepoWatchWorker;
use crate::effects::{
    AiEffect, CompareEffect, Effect, GitHubEffect, RepositoryEffect, SettingsEffect, SyntaxEffect,
    UiEffect, UpdateEffect,
};
use crate::events::{
    AiEvent, AppEvent, CompareEvent, GitHubEvent, RepositoryEvent, SettingsEvent, SyntaxEvent,
    UiEvent, UpdateEvent,
};
use crate::platform::persistence::Settings;
use winit::event_loop::EventLoopProxy;

const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(250);

pub struct AppRuntime {
    receiver: Receiver<AppEvent>,
    runner: EffectRunner,
}

impl AppRuntime {
    pub fn new(services: AppServices, wake_proxy: Option<EventLoopProxy<()>>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let event_sender = RuntimeEventSender::new(sender, wake_proxy);
        let save_worker = SaveWorker::new(services.clone(), event_sender.clone());
        let git_worker = GitWorker::new(event_sender.clone());
        let compare_scheduler = CompareScheduler::new(services.clone(), event_sender.clone());
        let repo_watch_worker = RepoWatchWorker::new(git_worker.sender());
        Self {
            receiver,
            runner: EffectRunner {
                services,
                event_sender,
                save_worker,
                git_worker,
                compare_scheduler,
                repo_watch_worker,
            },
        }
    }

    pub fn dispatch_all(&self, effects: Vec<Effect>) {
        for effect in effects {
            self.runner.dispatch(effect);
        }
    }

    pub fn drain_events(&self) -> Vec<AppEvent> {
        self.receiver.try_iter().collect()
    }
}

struct EffectRunner {
    services: AppServices,
    event_sender: RuntimeEventSender,
    save_worker: SaveWorker,
    git_worker: GitWorker,
    compare_scheduler: CompareScheduler,
    repo_watch_worker: RepoWatchWorker,
}

struct SaveWorker {
    sender: Sender<Settings>,
}

#[derive(Clone)]
pub(crate) struct RuntimeEventSender {
    sender: Sender<AppEvent>,
    wake_proxy: Option<EventLoopProxy<()>>,
}

impl RuntimeEventSender {
    fn new(sender: Sender<AppEvent>, wake_proxy: Option<EventLoopProxy<()>>) -> Self {
        Self { sender, wake_proxy }
    }

    pub(crate) fn send<E: Into<AppEvent>>(&self, event: E) {
        let event = event.into();
        if self.sender.send(event).is_ok() {
            if let Some(wake_proxy) = &self.wake_proxy {
                let _ = wake_proxy.send_event(());
            }
        }
    }
}

impl SaveWorker {
    fn new(services: AppServices, event_sender: RuntimeEventSender) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || save_worker_loop(services, event_sender, receiver));
        Self { sender }
    }

    fn dispatch(&self, settings: Settings) {
        let _ = self.sender.send(settings);
    }
}

impl EffectRunner {
    fn dispatch(&self, effect: Effect) {
        match effect {
            Effect::Ui(UiEffect::OpenRepositoryDialog) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    event_sender.send(UiEvent::RepositoryDialogClosed {
                        path: services.open_repository_dialog(),
                    });
                });
            }
            Effect::Repository(RepositoryEffect::WatchRepository { path }) => {
                self.repo_watch_worker.dispatch(path);
            }
            Effect::Repository(RepositoryEffect::SyncRepository {
                path,
                reason,
                reporter_generation,
            }) => {
                self.git_worker
                    .dispatch_sync(path, reason, reporter_generation);
            }
            Effect::Repository(RepositoryEffect::ApplyStatusOperation(request)) => {
                self.git_worker.dispatch_operation(
                    request.repo_path,
                    request.item,
                    request.operation,
                );
            }
            Effect::Repository(RepositoryEffect::ApplyBatchStatusOperation(request)) => {
                self.git_worker.dispatch_batch_operation(
                    request.repo_path,
                    request.items,
                    request.operation,
                );
            }
            Effect::Repository(RepositoryEffect::ApplyPatchOperation(request)) => {
                self.git_worker.dispatch_patch_operation(
                    request.repo_path,
                    request.patch,
                    request.scope,
                    request.operation,
                );
            }
            Effect::Repository(RepositoryEffect::CreateCommit(request)) => {
                self.git_worker
                    .dispatch_commit(request.repo_path, request.message);
            }
            Effect::Repository(RepositoryEffect::FetchRemote(request)) => {
                self.git_worker
                    .dispatch_fetch(request.repo_path, request.remote, request.toast_id);
            }
            Effect::Repository(RepositoryEffect::Push(request)) => {
                self.git_worker.dispatch_push(
                    request.repo_path,
                    request.remote,
                    request.refspec,
                    request.force_with_lease,
                    request.toast_id,
                );
            }
            Effect::Repository(RepositoryEffect::PullFf(request)) => {
                self.git_worker.dispatch_pull_ff(
                    request.repo_path,
                    request.remote,
                    request.branch,
                    request.toast_id,
                );
            }
            Effect::Compare(CompareEffect::Run(task)) => {
                let generation = task.generation;
                let request = task.request;
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let reporter =
                        crate::apprt::ProgressReporter::new(generation, event_sender.clone());
                    let event = match services.run_compare(generation, request, Some(&reporter)) {
                        Ok(payload) => CompareEvent::CompareFinished(payload),
                        Err(error) => CompareEvent::CompareFailed {
                            generation,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::Compare(CompareEffect::LoadStats(task)) => {
                self.compare_scheduler.dispatch_load_stats(task);
            }
            Effect::Compare(CompareEffect::LoadHistory(task)) => {
                let generation = task.generation;
                let request = task.request;
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.load_compare_history(generation, request) {
                        Ok(payload) => CompareEvent::CompareHistoryReady(payload),
                        Err(error) => CompareEvent::CompareHistoryFailed {
                            generation,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::Compare(CompareEffect::LoadFile(task)) => {
                self.compare_scheduler.dispatch_load_file(task);
            }
            Effect::Compare(CompareEffect::LoadFileStats(task)) => {
                self.compare_scheduler.dispatch_load_file_stats(task);
            }
            Effect::Repository(RepositoryEffect::LoadStatusDiff { task, index }) => {
                let generation = task.generation;
                let request = task.request;
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.load_status_diff(generation, index, request) {
                        Ok(payload) => CompareEvent::StatusDiffFinished(payload),
                        Err(error) => CompareEvent::StatusDiffFailed {
                            generation,
                            index,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::Syntax(SyntaxEffect::LoadFileSyntax(task)) => {
                let generation = task.generation;
                let request = task.request;
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let tokens = services.load_file_syntax(&request);
                    event_sender.send(SyntaxEvent::FileSyntaxReady(
                        crate::events::FileSyntaxReady {
                            generation,
                            request_id: request.request_id,
                            file_index: request.file_index,
                            path: request.path,
                            window: request.window,
                            tokens,
                        },
                    ));
                });
            }
            Effect::GitHub(GitHubEffect::LoadPullRequest {
                url,
                repo_path,
                github_token,
            }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.load_pull_request(&url, &repo_path, github_token) {
                        Ok((info, left_ref, right_ref)) => GitHubEvent::PullRequestLoaded {
                            url,
                            info,
                            left_ref,
                            right_ref,
                        },
                        Err(error) => GitHubEvent::PullRequestLoadFailed {
                            url,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::GitHub(GitHubEffect::StartDeviceFlow { client_id }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.start_device_flow(&client_id) {
                        Ok(state) => GitHubEvent::DeviceFlowStarted(state),
                        Err(error) => GitHubEvent::DeviceFlowStartFailed {
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::GitHub(GitHubEffect::PollDeviceFlow {
                client_id,
                device_code,
                interval_seconds,
            }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event =
                        match services.poll_device_flow(&client_id, &device_code, interval_seconds)
                        {
                            Ok(token) => GitHubEvent::DeviceFlowCompleted { token },
                            Err(error) => GitHubEvent::DeviceFlowFailed {
                                message: error.to_string(),
                            },
                        };
                    event_sender.send(event);
                });
            }
            Effect::GitHub(GitHubEffect::LoadGitHubToken) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.load_github_token() {
                        Ok(token) => GitHubEvent::GitHubTokenLoaded { token },
                        Err(error) => GitHubEvent::GitHubTokenLoadFailed {
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::GitHub(GitHubEffect::SaveGitHubToken(token)) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    if let Err(error) = services.save_github_token(&token) {
                        event_sender.send(GitHubEvent::GitHubTokenSaveFailed {
                            message: error.to_string(),
                        });
                    }
                });
            }
            Effect::GitHub(GitHubEffect::ClearGitHubToken) => {
                let services = self.services.clone();
                thread::spawn(move || {
                    if let Err(error) = services.clear_github_token() {
                        tracing::warn!("failed to clear GitHub token: {error}");
                    }
                });
            }
            Effect::GitHub(GitHubEffect::FetchGitHubUser { token }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.fetch_github_user(&token) {
                        Ok(user) => GitHubEvent::GitHubUserFetched { user },
                        Err(error) => GitHubEvent::GitHubUserFetchFailed {
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::GitHub(GitHubEffect::PeekPullRequest {
                owner,
                repo,
                number,
                github_token,
            }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event =
                        match services.peek_pull_request(&owner, &repo, number, github_token) {
                            Ok(info) => GitHubEvent::PullRequestPeeked {
                                owner,
                                repo,
                                number,
                                info,
                            },
                            Err(error) => GitHubEvent::PullRequestPeekFailed {
                                owner,
                                repo,
                                number,
                                message: error.to_string(),
                            },
                        };
                    event_sender.send(event);
                });
            }
            Effect::GitHub(GitHubEffect::FetchPullRequestReviewComments {
                owner,
                repo,
                number,
                github_token,
            }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.fetch_pull_request_review_comments(
                        &owner,
                        &repo,
                        number,
                        github_token,
                    ) {
                        Ok(comments) => GitHubEvent::PullRequestReviewCommentsLoaded {
                            owner,
                            repo,
                            number,
                            comments,
                        },
                        Err(error) => GitHubEvent::PullRequestReviewCommentsLoadFailed {
                            owner,
                            repo,
                            number,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::GitHub(GitHubEffect::CreatePullRequestReviewComment {
                owner,
                repo,
                number,
                github_token,
                comment,
            }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.create_pull_request_review_comment(
                        &owner,
                        &repo,
                        number,
                        github_token,
                        &comment,
                    ) {
                        Ok(comment) => GitHubEvent::PullRequestReviewCommentCreated {
                            owner,
                            repo,
                            number,
                            comment,
                        },
                        Err(error) => GitHubEvent::PullRequestReviewCommentCreateFailed {
                            owner,
                            repo,
                            number,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::GitHub(GitHubEffect::FetchAvatar { url }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.fetch_avatar(&url) {
                        Ok((rgba, width, height)) => GitHubEvent::AvatarFetched {
                            url,
                            rgba: std::sync::Arc::new(rgba),
                            width,
                            height,
                        },
                        Err(error) => GitHubEvent::AvatarFetchFailed {
                            url,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::Compare(CompareEffect::ResolveRef {
                repo_path,
                query,
                generation,
            }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.resolve_ref(&repo_path, &query) {
                        Ok((short_oid, summary)) => CompareEvent::RefResolved {
                            query,
                            generation,
                            short_oid,
                            summary,
                        },
                        Err(_) => CompareEvent::RefResolveFailed { generation },
                    };
                    event_sender.send(event);
                });
            }
            Effect::Settings(SettingsEffect::SaveSettings(settings)) => {
                self.save_worker.dispatch(settings);
            }
            Effect::Update(UpdateEffect::CheckForUpdates { silent }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.check_for_updates(crate::APP_VERSION) {
                        Ok(crate::core::update::UpdateCheck::Available(update)) => {
                            UpdateEvent::UpdateAvailable { update, silent }
                        }
                        Ok(crate::core::update::UpdateCheck::NotAvailable) => {
                            UpdateEvent::UpdateNotAvailable { silent }
                        }
                        Err(error) => UpdateEvent::UpdateCheckFailed {
                            message: error.to_string(),
                            silent,
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::Update(UpdateEffect::StageUpdate { update, silent }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || match services.stage_update(&update) {
                    Ok(staged) => event_sender.send(UpdateEvent::UpdateStaged { staged, silent }),
                    Err(error) => event_sender.send(UpdateEvent::UpdateInstallFailed {
                        message: error.to_string(),
                        silent,
                    }),
                });
            }
            Effect::Update(UpdateEffect::ApplyStagedUpdate(staged)) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    if let Err(error) = services.apply_staged_update(&staged) {
                        event_sender.send(UpdateEvent::UpdateInstallFailed {
                            message: error.to_string(),
                            silent: false,
                        });
                    }
                });
            }
            Effect::Ui(UiEffect::OpenBrowser { url }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    if let Err(error) = services.open_browser(&url) {
                        event_sender.send(UiEvent::BrowserOpenFailed {
                            message: error.to_string(),
                        });
                    }
                });
            }
            Effect::Ui(UiEffect::SetClipboard(text)) => {
                thread::spawn(move || {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                });
            }
            Effect::Repository(RepositoryEffect::FetchContextLines(request)) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.fetch_context_lines(&request) {
                        Ok((old_lines, new_lines)) => {
                            RepositoryEvent::ContextLinesReady(crate::events::ContextLinesReady {
                                generation: request.generation,
                                file_index: request.file_index,
                                path: request.path,
                                hunk_index: request.hunk_index,
                                direction: request.direction,
                                amount: request.amount,
                                old_lines,
                                new_lines,
                            })
                        }
                        Err(error) => RepositoryEvent::ContextLinesFailed {
                            generation: request.generation,
                            file_index: request.file_index,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::Syntax(SyntaxEffect::InstallCommonSyntaxPacks) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let language = "common syntax".to_owned();
                    tracing::info!("syntax pack install started");
                    event_sender.send(SyntaxEvent::SyntaxPackInstallStarted {
                        language: language.clone(),
                    });
                    match services.install_common_syntax_packs() {
                        Ok(languages) => {
                            let installed_count = languages.len();
                            for language in languages {
                                event_sender.send(SyntaxEvent::SyntaxPackInstalled { language });
                            }
                            tracing::info!(installed_count, "syntax pack install finished");
                            event_sender.send(SyntaxEvent::SyntaxPackInstallFinished { language });
                        }
                        Err(error) => {
                            tracing::warn!(%error, "syntax pack install failed");
                            event_sender.send(SyntaxEvent::SyntaxPackInstallFailed { language });
                        }
                    }
                });
            }
            Effect::Syntax(SyntaxEffect::EnsureSyntaxPackForPath { path }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let highlighter = phosphor::Highlighter::new();
                    let language = highlighter
                        .guess_language(std::path::Path::new(&path))
                        .filter(|language| !highlighter.is_parser_available(*language))
                        .map(|language| language.name().to_owned());
                    if let Some(language) = &language {
                        tracing::info!(language, path = %path, "syntax pack install started");
                        event_sender.send(SyntaxEvent::SyntaxPackInstallStarted {
                            language: language.clone(),
                        });
                    }
                    match services.ensure_syntax_pack_for_path(&path) {
                        Ok(Some(installed)) => {
                            tracing::info!(language = %installed, path = %path, "syntax pack installed");
                            event_sender.send(SyntaxEvent::SyntaxPackInstalled {
                                language: installed,
                            });
                            if let Some(language) = language {
                                event_sender
                                    .send(SyntaxEvent::SyntaxPackInstallFinished { language });
                            }
                        }
                        Ok(None) => {
                            if let Some(language) = language {
                                event_sender
                                    .send(SyntaxEvent::SyntaxPackInstallFinished { language });
                            }
                        }
                        Err(error) => {
                            tracing::warn!(path = %path, %error, "syntax pack install failed");
                            if let Some(language) = language {
                                event_sender
                                    .send(SyntaxEvent::SyntaxPackInstallFailed { language });
                            }
                        }
                    }
                });
            }
            Effect::Syntax(SyntaxEffect::EnsureSyntaxPacksForPaths { paths }) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let highlighter = phosphor::Highlighter::new();
                    let mut seen = HashSet::new();
                    let mut languages = Vec::new();
                    for path in &paths {
                        let Some(language) = highlighter
                            .guess_language(std::path::Path::new(path))
                            .filter(|language| !highlighter.is_parser_available(*language))
                        else {
                            continue;
                        };
                        if seen.insert(language) {
                            languages.push(language.name().to_owned());
                        }
                    }
                    if languages.is_empty() {
                        return;
                    }

                    tracing::info!(
                        languages = ?languages,
                        path_count = paths.len(),
                        "syntax pack batch install started"
                    );
                    for language in &languages {
                        event_sender.send(SyntaxEvent::SyntaxPackInstallStarted {
                            language: language.clone(),
                        });
                    }
                    match services.ensure_syntax_packs_for_paths(&paths) {
                        Ok(installed) => {
                            let installed_count = installed.len();
                            for language in installed {
                                event_sender.send(SyntaxEvent::SyntaxPackInstalled { language });
                            }
                            tracing::info!(installed_count, "syntax pack batch install finished");
                            for language in languages {
                                event_sender
                                    .send(SyntaxEvent::SyntaxPackInstallFinished { language });
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "syntax pack batch install failed");
                            for language in languages {
                                event_sender
                                    .send(SyntaxEvent::SyntaxPackInstallFailed { language });
                            }
                        }
                    }
                });
            }
            Effect::Ai(AiEffect::LoadAiKeys) => {
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match crate::apprt::services::load_ai_keys() {
                        Ok((openai, anthropic)) => AiEvent::AiKeysLoaded { openai, anthropic },
                        Err(error) => AiEvent::AiKeysLoadFailed {
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::Ai(AiEffect::SaveAiKey { kind, value }) => {
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    if let Err(error) = crate::platform::secrets::save_ai_key(kind, &value) {
                        event_sender.send(AiEvent::AiKeySaveFailed {
                            message: error.to_string(),
                        });
                    }
                });
            }
            Effect::Ai(AiEffect::ClearAiKey { kind }) => {
                thread::spawn(move || {
                    if let Err(error) = crate::platform::secrets::clear_ai_key(kind) {
                        tracing::warn!("failed to clear AI key from keyring: {error}");
                    }
                });
            }
            Effect::Ai(AiEffect::GenerateCommitMessage(request)) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    services.run_commit_message_generation(request, event_sender);
                });
            }
        }
    }
}

fn save_worker_loop(
    services: AppServices,
    event_sender: RuntimeEventSender,
    receiver: Receiver<Settings>,
) {
    let mut last_saved: Option<Settings> = None;

    loop {
        let mut pending = match receiver.recv() {
            Ok(settings) => settings,
            Err(_) => break,
        };

        loop {
            match receiver.recv_timeout(SETTINGS_SAVE_DEBOUNCE) {
                Ok(next) => pending = next,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    persist_settings(&services, &event_sender, &mut last_saved, pending);
                    return;
                }
            }
        }

        persist_settings(&services, &event_sender, &mut last_saved, pending);
    }
}

fn persist_settings(
    services: &AppServices,
    event_sender: &RuntimeEventSender,
    last_saved: &mut Option<Settings>,
    settings: Settings,
) {
    if last_saved.as_ref() == Some(&settings) {
        return;
    }

    tracing::debug!(
        theme = %settings.theme_name,
        mode = ?settings.theme_mode,
        "persisting settings"
    );

    let event = match services.save_settings(&settings) {
        Ok(()) => {
            *last_saved = Some(settings);
            SettingsEvent::SettingsSaved
        }
        Err(error) => SettingsEvent::SettingsSaveFailed {
            message: error.to_string(),
        },
    };
    event_sender.send(event);
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use tempfile::TempDir;

    use super::{AppRuntime, SETTINGS_SAVE_DEBOUNCE};
    use crate::apprt::services::AppServices;
    use crate::effects::SettingsEffect;
    use crate::events::{AppEvent, SettingsEvent};
    use crate::platform::persistence::{Settings, SettingsStore};

    fn wait_for<P>(mut predicate: P)
    where
        P: FnMut() -> bool,
    {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if predicate() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for condition");
    }

    #[test]
    fn save_settings_coalesces_rapid_updates_and_keeps_latest() {
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new_in(dir.path());
        let runtime = AppRuntime::new(AppServices::new(store.clone()), None);

        let mut settings = Settings::default();
        settings.theme_name = "One".to_owned();
        runtime.dispatch_all(vec![SettingsEffect::SaveSettings(settings.clone()).into()]);

        settings.theme_name = "Two".to_owned();
        runtime.dispatch_all(vec![SettingsEffect::SaveSettings(settings.clone()).into()]);

        settings.theme_name = "Three".to_owned();
        runtime.dispatch_all(vec![SettingsEffect::SaveSettings(settings.clone()).into()]);

        wait_for(|| {
            store
                .load()
                .map(|saved| saved.theme_name == "Three")
                .unwrap_or(false)
        });

        let saved_events = runtime
            .drain_events()
            .into_iter()
            .filter(|event| matches!(event, AppEvent::Settings(SettingsEvent::SettingsSaved)))
            .count();
        assert_eq!(saved_events, 1);
    }

    #[test]
    fn save_settings_skips_duplicate_snapshots() {
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new_in(dir.path());
        let runtime = AppRuntime::new(AppServices::new(store.clone()), None);

        let mut settings = Settings::default();
        settings.theme_name = "Gruvbox Hard".to_owned();

        runtime.dispatch_all(vec![SettingsEffect::SaveSettings(settings.clone()).into()]);
        wait_for(|| {
            store
                .load()
                .map(|saved| saved.theme_name == settings.theme_name)
                .unwrap_or(false)
        });
        let _ = runtime.drain_events();

        runtime.dispatch_all(vec![SettingsEffect::SaveSettings(settings.clone()).into()]);
        thread::sleep(SETTINGS_SAVE_DEBOUNCE + Duration::from_millis(100));

        let saved_events = runtime
            .drain_events()
            .into_iter()
            .filter(|event| matches!(event, AppEvent::Settings(SettingsEvent::SettingsSaved)))
            .count();
        assert_eq!(saved_events, 0);
    }
}
