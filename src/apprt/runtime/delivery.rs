use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use diffy_syntax::GrammarStore;

use crate::apprt::services::DeliveryServices;
use crate::apprt::services::delivery::{ensure_grammars, grammar_status, retry_grammar};
use crate::apprt::transport::ScopedEventSender;
use crate::contracts::delivery::{DeliveryEffect, DeliveryEvent};
use crate::core::update::UpdateCheck;

type Watcher = Arc<Mutex<Option<ScopedEventSender>>>;

pub struct DeliveryRuntime {
    services: DeliveryServices,
    store: GrammarStore,
    watcher: Watcher,
    subscribed: AtomicBool,
}

impl DeliveryRuntime {
    pub fn new(services: DeliveryServices) -> Self {
        Self::with_store(services, services.grammar_store())
    }

    pub fn with_store(services: DeliveryServices, store: GrammarStore) -> Self {
        Self {
            services,
            store,
            watcher: Arc::default(),
            subscribed: AtomicBool::new(false),
        }
    }

    pub fn dispatch(&self, effect: DeliveryEffect, events: ScopedEventSender) {
        let services = self.services;
        match effect {
            DeliveryEffect::CheckForUpdates { silent } => {
                thread::spawn(move || {
                    let event = match services.check_for_updates(crate::APP_VERSION) {
                        Ok(UpdateCheck::Available(update)) => DeliveryEvent::UpdateAvailable {
                            update: Box::new(update),
                            silent,
                        },
                        Ok(UpdateCheck::NotAvailable) => {
                            DeliveryEvent::UpdateNotAvailable { silent }
                        }
                        Err(error) => DeliveryEvent::UpdateCheckFailed {
                            message: error.user_message(),
                            silent,
                        },
                    };
                    events.send(event);
                });
            }
            DeliveryEffect::StageUpdate { update, silent } => {
                thread::spawn(move || {
                    let event = match services.stage_update(&update) {
                        Ok(staged) => DeliveryEvent::UpdateStaged {
                            staged: Box::new(staged),
                            silent,
                        },
                        Err(error) => DeliveryEvent::UpdateInstallFailed {
                            message: error.user_message(),
                            silent,
                        },
                    };
                    events.send(event);
                });
            }
            DeliveryEffect::ApplyStagedUpdate(staged) => {
                thread::spawn(move || {
                    if let Err(error) = services.apply_staged_update(&staged) {
                        events.send(DeliveryEvent::UpdateInstallFailed {
                            message: error.user_message(),
                            silent: false,
                        });
                    }
                });
            }
            DeliveryEffect::WatchGrammars => self.watch(events),
            DeliveryEffect::EnsureGrammars { languages } => {
                let store = self.store.clone();
                thread::spawn(move || {
                    ensure_grammars(&store, &languages);
                    events.send(DeliveryEvent::GrammarsChanged(grammar_status(&store)));
                });
            }
            DeliveryEffect::RetryGrammar { language } => {
                let store = self.store.clone();
                thread::spawn(move || {
                    retry_grammar(&store, &language);
                    events.send(DeliveryEvent::GrammarsChanged(grammar_status(&store)));
                });
            }
        }
    }

    fn watch(&self, events: ScopedEventSender) {
        events.send(DeliveryEvent::GrammarsChanged(grammar_status(&self.store)));
        *self.watcher.lock().unwrap_or_else(PoisonError::into_inner) = Some(events);
        if self.subscribed.swap(true, Ordering::AcqRel) {
            return;
        }
        let watcher = Arc::downgrade(&self.watcher);
        let store = self.store.clone();
        self.store.subscribe(move || {
            let Some(watcher) = watcher.upgrade() else {
                return false;
            };
            let events = watcher
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(events) = events {
                events.send(DeliveryEvent::GrammarsChanged(grammar_status(&store)));
            }
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    use diffy_syntax::testing::legacy as fixture;
    use diffy_syntax::{GrammarState, GrammarStatus};

    use super::*;
    use crate::apprt::transport::RuntimeEventSender;
    use crate::contracts::shared::WorkspaceEpoch;
    use crate::contracts::shell::{AppEvent, AppMessage};

    fn next(messages: &Receiver<AppMessage>) -> Vec<GrammarStatus> {
        match messages.recv_timeout(Duration::from_secs(30)) {
            Ok(AppMessage::Runtime(envelope)) => match envelope.event {
                AppEvent::Delivery(DeliveryEvent::GrammarsChanged(grammars)) => grammars.to_vec(),
                other => panic!("unexpected event {other:?}"),
            },
            other => panic!("unexpected message {other:?}"),
        }
    }

    #[test]
    fn grammar_effects_report_store_status() {
        let Some(packs) = fixture::native_packs(&[("rust", "rust")]) else {
            return;
        };
        let storage = tempfile::tempdir().unwrap();
        fixture::install_signed(storage.path(), &packs).unwrap();
        let store = fixture::cached_store(storage.path()).unwrap();
        let runtime = DeliveryRuntime::with_store(DeliveryServices, store);
        let (sender, messages) = channel();
        let events = RuntimeEventSender::from_channel(sender).scoped(WorkspaceEpoch::default());

        runtime.dispatch(DeliveryEffect::WatchGrammars, events.clone());
        let watched = next(&messages);
        runtime.dispatch(
            DeliveryEffect::EnsureGrammars {
                languages: vec!["rust"],
            },
            events,
        );

        assert!(watched.is_empty());
        assert_eq!(
            next(&messages),
            vec![GrammarStatus {
                language: "rust",
                state: GrammarState::Ready
            }]
        );
    }
}
