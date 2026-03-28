use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::app_runtime::services::AppServices;
use crate::ui::effects::Effect;
use crate::ui::events::AppEvent;

pub struct AppRuntime {
    receiver: Receiver<AppEvent>,
    runner: EffectRunner,
}

impl AppRuntime {
    pub fn new(services: AppServices) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            receiver,
            runner: EffectRunner { services, sender },
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
                let services = self.services.clone();
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let event = match services.save_settings(&settings) {
                        Ok(()) => AppEvent::SettingsSaved,
                        Err(error) => AppEvent::SettingsSaveFailed {
                            message: error.to_string(),
                        },
                    };
                    let _ = sender.send(event);
                });
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
        }
    }
}
