use std::sync::Arc;
use std::thread;

use crate::apprt::services::ReviewServices;
use crate::apprt::transport::ScopedEventSender;
use crate::contracts::review::{ReviewEffect, ReviewEvent};
use crate::platform::startup::GitHubTokenStore;

pub struct ReviewRuntime {
    services: ReviewServices,
    token_store: GitHubTokenStore,
}

impl ReviewRuntime {
    pub fn new(services: ReviewServices, token_store: GitHubTokenStore) -> Self {
        Self {
            services,
            token_store,
        }
    }

    pub fn dispatch(&self, effect: ReviewEffect, events: ScopedEventSender) {
        match effect {
            ReviewEffect::GitHub { scope, request } => {
                let github = Arc::clone(self.services.github());
                thread::spawn(move || {
                    github.execute(*request, &mut |response| {
                        events.send(ReviewEvent::GitHub {
                            scope,
                            response: Box::new(response),
                        });
                    });
                });
            }
            ReviewEffect::StartDeviceFlow { flow, client_id } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let event = match services.start_device_flow(&client_id) {
                        Ok(state) => ReviewEvent::DeviceFlowStarted { flow, state },
                        Err(error) => ReviewEvent::DeviceFlowStartFailed {
                            flow,
                            message: error.user_message(),
                        },
                    };
                    events.send(event);
                });
            }
            ReviewEffect::PollDeviceFlow {
                flow,
                client_id,
                device_code,
                interval_seconds,
            } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let event =
                        match services.poll_device_flow(&client_id, &device_code, interval_seconds)
                        {
                            Ok(token) => ReviewEvent::DeviceFlowCompleted { flow, token },
                            Err(error) => ReviewEvent::DeviceFlowFailed {
                                flow,
                                message: error.user_message(),
                            },
                        };
                    events.send(event);
                });
            }
            ReviewEffect::LoadToken => {
                if self.token_store == GitHubTokenStore::Disabled {
                    events.send(ReviewEvent::TokenLoaded { token: None });
                    return;
                }
                let services = self.services.clone();
                let store = self.token_store;
                thread::spawn(move || {
                    let result = match store {
                        GitHubTokenStore::Keyring => services.load_keyring_token(),
                        GitHubTokenStore::DevFile => services.load_dev_token(),
                        GitHubTokenStore::Disabled => Ok(None),
                    };
                    let event = match result {
                        Ok(token) => ReviewEvent::TokenLoaded { token },
                        Err(error) => ReviewEvent::TokenLoadFailed {
                            message: error.user_message(),
                        },
                    };
                    events.send(event);
                });
            }
            ReviewEffect::SaveToken(token) => {
                if self.token_store == GitHubTokenStore::Disabled {
                    return;
                }
                let services = self.services.clone();
                let store = self.token_store;
                thread::spawn(move || {
                    let result = match store {
                        GitHubTokenStore::Keyring => services.save_keyring_token(&token),
                        GitHubTokenStore::DevFile => services.save_dev_token(&token),
                        GitHubTokenStore::Disabled => Ok(()),
                    };
                    if let Err(error) = result {
                        events.send(ReviewEvent::TokenSaveFailed {
                            message: error.user_message(),
                        });
                    }
                });
            }
            ReviewEffect::ClearToken => {
                if self.token_store == GitHubTokenStore::Disabled {
                    return;
                }
                let services = self.services.clone();
                let store = self.token_store;
                thread::spawn(move || {
                    let result = match store {
                        GitHubTokenStore::Keyring => services.clear_keyring_token(),
                        GitHubTokenStore::DevFile => services.clear_dev_token(),
                        GitHubTokenStore::Disabled => Ok(()),
                    };
                    if let Err(error) = result {
                        tracing::warn!("failed to clear GitHub token: {error}");
                    }
                });
            }
            ReviewEffect::LoadSession {
                scope,
                target,
                pull_request,
            } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let event = match services.load_session(target.clone(), *pull_request) {
                        Ok(session) => ReviewEvent::SessionLoaded {
                            scope,
                            target,
                            session: Box::new(session),
                        },
                        Err(error) => ReviewEvent::SessionLoadFailed {
                            scope,
                            target,
                            message: error.user_message(),
                        },
                    };
                    events.send(event);
                });
            }
            ReviewEffect::SaveSession { scope, session } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let event = match services.save_session(&session) {
                        Ok(key) => ReviewEvent::SessionSaved { scope, key },
                        Err(error) => ReviewEvent::SessionSaveFailed {
                            scope,
                            key: session.key(),
                            message: error.user_message(),
                        },
                    };
                    events.send(event);
                });
            }
            ReviewEffect::FetchAvatar { url } => {
                let services = self.services.clone();
                thread::spawn(move || {
                    let event = match services.fetch_avatar(&url) {
                        Ok(image) => ReviewEvent::AvatarFetched {
                            url,
                            image: Arc::new(image),
                        },
                        Err(error) => ReviewEvent::AvatarFetchFailed {
                            url,
                            message: error.user_message(),
                        },
                    };
                    events.send(event);
                });
            }
        }
    }
}
