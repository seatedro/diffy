use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::apprt::git_worker::GitWorker;
use crate::apprt::services::AppServices;
use crate::apprt::watcher::RepoWatchWorker;
use crate::effects::Effect;
use crate::events::AppEvent;
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
        let repo_watch_worker = RepoWatchWorker::new(git_worker.sender());
        Self {
            receiver,
            runner: EffectRunner {
                services,
                event_sender,
                save_worker,
                git_worker,
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

    pub(crate) fn send(&self, event: AppEvent) {
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
            Effect::OpenRepositoryDialog => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    event_sender.send(AppEvent::RepositoryDialogClosed {
                        path: services.open_repository_dialog(),
                    });
                });
            }
            Effect::WatchRepository { path } => {
                self.repo_watch_worker.dispatch(path);
            }
            Effect::SyncRepository { path, reason } => {
                self.git_worker.dispatch_sync(path, reason);
            }
            Effect::ApplyStatusOperation(request) => {
                self.git_worker.dispatch_operation(
                    request.repo_path,
                    request.item,
                    request.operation,
                );
            }
            Effect::ApplyBatchStatusOperation(request) => {
                self.git_worker.dispatch_batch_operation(
                    request.repo_path,
                    request.items,
                    request.operation,
                );
            }
            Effect::ApplyPatchOperation(request) => {
                self.git_worker.dispatch_patch_operation(
                    request.repo_path,
                    request.patch,
                    request.scope,
                    request.operation,
                );
            }
            Effect::CreateCommit(request) => {
                self.git_worker
                    .dispatch_commit(request.repo_path, request.message);
            }
            Effect::FetchRemote(request) => {
                self.git_worker
                    .dispatch_fetch(request.repo_path, request.remote, request.toast_id);
            }
            Effect::Push(request) => {
                self.git_worker.dispatch_push(
                    request.repo_path,
                    request.remote,
                    request.refspec,
                    request.force_with_lease,
                    request.toast_id,
                );
            }
            Effect::PullFf(request) => {
                self.git_worker.dispatch_pull_ff(
                    request.repo_path,
                    request.remote,
                    request.branch,
                    request.toast_id,
                );
            }
            Effect::RunCompare {
                generation,
                request,
            } => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.run_compare(generation, request) {
                        Ok(payload) => AppEvent::CompareFinished(payload),
                        Err(error) => AppEvent::CompareFailed {
                            generation,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::LoadStatusDiff {
                generation,
                index,
                request,
            } => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.load_status_diff(generation, index, request) {
                        Ok(payload) => AppEvent::StatusDiffFinished(payload),
                        Err(error) => AppEvent::StatusDiffFailed {
                            generation,
                            index,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::LoadPullRequest {
                url,
                repo_path,
                github_token,
            } => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.load_pull_request(&url, &repo_path, github_token) {
                        Ok((info, left_ref, right_ref)) => AppEvent::PullRequestLoaded {
                            url,
                            info,
                            left_ref,
                            right_ref,
                        },
                        Err(error) => AppEvent::PullRequestLoadFailed {
                            url,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::StartDeviceFlow { client_id } => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.start_device_flow(&client_id) {
                        Ok(state) => AppEvent::DeviceFlowStarted(state),
                        Err(error) => AppEvent::DeviceFlowStartFailed {
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::PollDeviceFlow {
                client_id,
                device_code,
                interval_seconds,
            } => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event =
                        match services.poll_device_flow(&client_id, &device_code, interval_seconds)
                        {
                            Ok(token) => AppEvent::DeviceFlowCompleted { token },
                            Err(error) => AppEvent::DeviceFlowFailed {
                                message: error.to_string(),
                            },
                        };
                    event_sender.send(event);
                });
            }
            Effect::LoadGitHubToken => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.load_github_token() {
                        Ok(token) => AppEvent::GitHubTokenLoaded { token },
                        Err(error) => AppEvent::GitHubTokenLoadFailed {
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::SaveGitHubToken(token) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    if let Err(error) = services.save_github_token(&token) {
                        event_sender.send(AppEvent::GitHubTokenSaveFailed {
                            message: error.to_string(),
                        });
                    }
                });
            }
            Effect::ClearGitHubToken => {
                let services = self.services.clone();
                thread::spawn(move || {
                    if let Err(error) = services.clear_github_token() {
                        tracing::warn!("failed to clear GitHub token: {error}");
                    }
                });
            }
            Effect::FetchGitHubUser { token } => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.fetch_github_user(&token) {
                        Ok(user) => AppEvent::GitHubUserFetched { user },
                        Err(error) => AppEvent::GitHubUserFetchFailed {
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::PeekPullRequest {
                owner,
                repo,
                number,
                github_token,
            } => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event =
                        match services.peek_pull_request(&owner, &repo, number, github_token) {
                            Ok(info) => AppEvent::PullRequestPeeked {
                                owner,
                                repo,
                                number,
                                info,
                            },
                            Err(error) => AppEvent::PullRequestPeekFailed {
                                owner,
                                repo,
                                number,
                                message: error.to_string(),
                            },
                        };
                    event_sender.send(event);
                });
            }
            Effect::FetchAvatar { url } => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.fetch_avatar(&url) {
                        Ok((rgba, width, height)) => AppEvent::AvatarFetched {
                            url,
                            rgba: std::sync::Arc::new(rgba),
                            width,
                            height,
                        },
                        Err(error) => AppEvent::AvatarFetchFailed {
                            url,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
                });
            }
            Effect::ResolveRef {
                repo_path,
                query,
                generation,
            } => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.resolve_ref(&repo_path, &query) {
                        Ok((short_oid, summary)) => AppEvent::RefResolved {
                            query,
                            generation,
                            short_oid,
                            summary,
                        },
                        Err(_) => AppEvent::RefResolveFailed { generation },
                    };
                    event_sender.send(event);
                });
            }
            Effect::SaveSettings(settings) => {
                self.save_worker.dispatch(settings);
            }
            Effect::OpenBrowser { url } => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    if let Err(error) = services.open_browser(&url) {
                        event_sender.send(AppEvent::BrowserOpenFailed {
                            message: error.to_string(),
                        });
                    }
                });
            }
            Effect::SetClipboard(text) => {
                thread::spawn(move || {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                });
            }
            Effect::FetchContextLines(request) => {
                let services = self.services.clone();
                let event_sender = self.event_sender.clone();
                thread::spawn(move || {
                    let event = match services.fetch_context_lines(&request) {
                        Ok(lines) => {
                            AppEvent::ContextLinesReady(crate::events::ContextLinesReady {
                                generation: request.generation,
                                file_index: request.file_index,
                                path: request.path,
                                hunk_index: request.hunk_index,
                                direction: request.direction,
                                amount: request.amount,
                                lines,
                            })
                        }
                        Err(error) => AppEvent::ContextLinesFailed {
                            generation: request.generation,
                            file_index: request.file_index,
                            message: error.to_string(),
                        },
                    };
                    event_sender.send(event);
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
            AppEvent::SettingsSaved
        }
        Err(error) => AppEvent::SettingsSaveFailed {
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
    use crate::effects::Effect;
    use crate::events::AppEvent;
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
        runtime.dispatch_all(vec![Effect::SaveSettings(settings.clone())]);

        settings.theme_name = "Two".to_owned();
        runtime.dispatch_all(vec![Effect::SaveSettings(settings.clone())]);

        settings.theme_name = "Three".to_owned();
        runtime.dispatch_all(vec![Effect::SaveSettings(settings.clone())]);

        wait_for(|| {
            store
                .load()
                .map(|saved| saved.theme_name == "Three")
                .unwrap_or(false)
        });

        let saved_events = runtime
            .drain_events()
            .into_iter()
            .filter(|event| matches!(event, AppEvent::SettingsSaved))
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

        runtime.dispatch_all(vec![Effect::SaveSettings(settings.clone())]);
        wait_for(|| {
            store
                .load()
                .map(|saved| saved.theme_name == settings.theme_name)
                .unwrap_or(false)
        });
        let _ = runtime.drain_events();

        runtime.dispatch_all(vec![Effect::SaveSettings(settings.clone())]);
        thread::sleep(SETTINGS_SAVE_DEBOUNCE + Duration::from_millis(100));

        let saved_events = runtime
            .drain_events()
            .into_iter()
            .filter(|event| matches!(event, AppEvent::SettingsSaved))
            .count();
        assert_eq!(saved_events, 0);
    }
}
