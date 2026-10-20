use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use diffy_syntax::GrammarStore;

use crate::apprt::services::DeliveryServices;
use crate::apprt::services::delivery::{ensure_grammars, grammar_status, retry_grammar};
use crate::apprt::transport::ScopedEventSender;
use crate::contracts::delivery::{DeliveryEffect, DeliveryEvent};
use crate::core::update::UpdateCheck;

type Watcher = Arc<Mutex<Option<ScopedEventSender>>>;
type Poll = (u64, Duration, ScopedEventSender);

pub struct DeliveryRuntime {
    services: DeliveryServices,
    store: GrammarStore,
    watcher: Watcher,
    subscribed: AtomicBool,
    poll: Mutex<Option<Sender<Poll>>>,
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
            poll: Mutex::default(),
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
                let (store, watcher) = (self.store.clone(), self.watcher.clone());
                thread::spawn(move || {
                    ensure_grammars(&store, &languages);
                    report(&watcher, &events, &store);
                });
            }
            DeliveryEffect::ScheduleUpdatePoll { generation, after } => {
                self.schedule_poll((generation, after, events));
            }
            DeliveryEffect::RetryGrammar { language } => {
                let (store, watcher) = (self.store.clone(), self.watcher.clone());
                thread::spawn(move || {
                    retry_grammar(&store, &language);
                    report(&watcher, &events, &store);
                });
            }
        }
    }

    fn schedule_poll(&self, poll: Poll) {
        let mut sender = self.poll.lock().unwrap_or_else(PoisonError::into_inner);
        let poll = match &*sender {
            Some(existing) => match existing.send(poll) {
                Ok(()) => return,
                Err(failed) => failed.0,
            },
            None => poll,
        };
        let (next, requests) = channel();
        let _ = next.send(poll);
        let spawned = thread::Builder::new()
            .name("diffy-update-poll".to_owned())
            .spawn(move || poll_loop(&requests));
        match spawned {
            Ok(_) => *sender = Some(next),
            Err(error) => tracing::warn!(%error, "update polling unavailable"),
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
        self.store.subscribe_status(move || {
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

fn report(watcher: &Watcher, fallback: &ScopedEventSender, store: &GrammarStore) {
    let current = watcher
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let events = current.as_ref().unwrap_or(fallback);
    events.send(DeliveryEvent::GrammarsChanged(grammar_status(store)));
}

fn poll_loop(requests: &Receiver<Poll>) {
    let mut pending: Option<(u64, Instant, ScopedEventSender)> = None;
    loop {
        let next = match &pending {
            Some((_, at, _)) => requests.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => requests.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match next {
            Ok((generation, after, events)) => {
                pending = Some((generation, Instant::now() + after, events));
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some((generation, _, events)) = pending.take() {
                    events.send(DeliveryEvent::UpdatePollDue { generation });
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
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

        assert_eq!(
            watched,
            vec![GrammarStatus {
                language: "rust",
                state: GrammarState::Installed
            }]
        );
        assert_eq!(
            next(&messages),
            vec![GrammarStatus {
                language: "rust",
                state: GrammarState::Ready
            }]
        );
    }

    #[test]
    fn a_newer_poll_replaces_the_pending_one() {
        let runtime = DeliveryRuntime::with_store(DeliveryServices, GrammarStore::none());
        let (sender, messages) = channel();
        let events = RuntimeEventSender::from_channel(sender).scoped(WorkspaceEpoch::default());
        let schedule = |generation, millis| DeliveryEffect::ScheduleUpdatePoll {
            generation,
            after: Duration::from_millis(millis),
        };

        runtime.dispatch(schedule(1, 200), events.clone());
        runtime.dispatch(schedule(2, 20), events);
        let due = match messages.recv_timeout(Duration::from_secs(10)) {
            Ok(AppMessage::Runtime(envelope)) => match envelope.event {
                AppEvent::Delivery(DeliveryEvent::UpdatePollDue { generation }) => generation,
                other => panic!("unexpected event {other:?}"),
            },
            other => panic!("unexpected message {other:?}"),
        };

        assert_eq!(due, 2);
        assert!(messages.recv_timeout(Duration::from_millis(400)).is_err());
    }
}
