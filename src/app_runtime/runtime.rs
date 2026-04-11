use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::app_runtime::services::AppServices;
use crate::platform::persistence::Settings;
use crate::effects::Effect;
use crate::events::AppEvent;

const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(250);

pub struct AppRuntime {
    receiver: Receiver<AppEvent>,
    runner: EffectRunner,
}

impl AppRuntime {
    pub fn new(services: AppServices) -> Self {
        let (sender, receiver) = mpsc::channel();
        let save_worker = SaveWorker::new(services.clone(), sender.clone());
        Self {
            receiver,
            runner: EffectRunner {
                services,
                sender,
                save_worker,
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
    sender: Sender<AppEvent>,
    save_worker: SaveWorker,
}

struct SaveWorker {
    sender: Sender<Settings>,
}

impl SaveWorker {
    fn new(services: AppServices, event_sender: Sender<AppEvent>) -> Self {
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
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let _ = sender.send(AppEvent::RepositoryDialogClosed {
                        path: services.open_repository_dialog(),
                    });
                });
            }
            Effect::LoadRepository { path } => {
                let services = self.services.clone();
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let event = match services.load_repository(path.clone()) {
                        Ok(payload) => AppEvent::RepositoryLoaded(payload),
                        Err(error) => AppEvent::RepositoryLoadFailed {
                            path,
                            message: error.to_string(),
                        },
                    };
                    let _ = sender.send(event);
                });
            }
            Effect::RunCompare {
                generation,
                request,
            } => {
                let services = self.services.clone();
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let event = match services.run_compare(generation, request) {
                        Ok(payload) => AppEvent::CompareFinished(payload),
                        Err(error) => AppEvent::CompareFailed {
                            generation,
                            message: error.to_string(),
                        },
                    };
                    let _ = sender.send(event);
                });
            }
            Effect::LoadPullRequest {
                url,
                repo_path,
                github_token,
            } => {
                let services = self.services.clone();
                let sender = self.sender.clone();
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
                    let _ = sender.send(event);
                });
            }
            Effect::StartDeviceFlow { client_id } => {
                let services = self.services.clone();
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let event = match services.start_device_flow(&client_id) {
                        Ok(state) => AppEvent::DeviceFlowStarted(state),
                        Err(error) => AppEvent::DeviceFlowStartFailed {
                            message: error.to_string(),
                        },
                    };
                    let _ = sender.send(event);
                });
            }
            Effect::PollDeviceFlow {
                client_id,
                device_code,
                interval_seconds,
            } => {
                let services = self.services.clone();
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let event =
                        match services.poll_device_flow(&client_id, &device_code, interval_seconds)
                        {
                            Ok(token) => AppEvent::DeviceFlowCompleted { token },
                            Err(error) => AppEvent::DeviceFlowFailed {
                                message: error.to_string(),
                            },
                        };
                    let _ = sender.send(event);
                });
            }
            Effect::SaveSettings(settings) => {
                self.save_worker.dispatch(settings);
            }
            Effect::OpenBrowser { url } => {
                let services = self.services.clone();
                let sender = self.sender.clone();
                thread::spawn(move || {
                    if let Err(error) = services.open_browser(&url) {
                        let _ = sender.send(AppEvent::BrowserOpenFailed {
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
        }
    }
}

fn save_worker_loop(
    services: AppServices,
    event_sender: Sender<AppEvent>,
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
    event_sender: &Sender<AppEvent>,
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
    let _ = event_sender.send(event);
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use tempfile::TempDir;

    use super::{AppRuntime, SETTINGS_SAVE_DEBOUNCE};
    use crate::app_runtime::services::AppServices;
    use crate::platform::persistence::{Settings, SettingsStore};
    use crate::effects::Effect;
    use crate::events::AppEvent;

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
        let runtime = AppRuntime::new(AppServices::new(store.clone()));

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
        let runtime = AppRuntime::new(AppServices::new(store.clone()));

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
