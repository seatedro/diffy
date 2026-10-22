use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use crate::apprt::transport::ScopedEventSender;
use crate::contracts::viewport::symbols::ResolveRequest;
use crate::contracts::viewport::symbols::{IndexRequest, SymbolsEffect, SymbolsEvent};
use crate::core::symbols::{Indexer, describe, open_cache, resolve};
use crate::core::syntax::Grammars;

type Job = (Box<IndexRequest>, ScopedEventSender);
type Description = (Box<ResolveRequest>, ScopedEventSender);

pub struct SymbolsRuntime {
    jobs: Sender<Job>,
    latest: Arc<AtomicU64>,
    descriptions: Sender<Description>,
}

impl Default for SymbolsRuntime {
    fn default() -> Self {
        let (jobs, queue) = mpsc::channel();
        let latest = Arc::new(AtomicU64::new(0));
        let worker_latest = latest.clone();
        let spawned = thread::Builder::new()
            .name("diffy-symbols".to_owned())
            .spawn(move || {
                crate::platform::energy::background_thread();
                run(&queue, &worker_latest)
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "symbol indexer unavailable");
        }
        let (descriptions, requests) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("diffy-describe".to_owned())
            .spawn(move || describe_loop(&requests));
        if let Err(error) = spawned {
            tracing::warn!(%error, "symbol descriptions unavailable");
        }
        Self {
            jobs,
            latest,
            descriptions,
        }
    }
}

impl SymbolsRuntime {
    pub fn dispatch(&self, effect: SymbolsEffect, events: ScopedEventSender) {
        match effect {
            SymbolsEffect::Index(request) => {
                self.latest.store(request.generation, Ordering::Release);
                let _ = self.jobs.send((request, events));
            }
            SymbolsEffect::Describe(request) => {
                let _ = self.descriptions.send((request, events));
            }
            SymbolsEffect::Resolve(request) => {
                thread::spawn(move || {
                    let _work =
                        crate::platform::energy::work(crate::platform::energy::Kind::Resolve);
                    let result = resolve(
                        &Grammars::shared().store(),
                        &request.repo_path,
                        &request.position,
                        &request.head,
                        request.index.as_deref(),
                    )
                    .map_err(|error| error.user_message());
                    events.send(SymbolsEvent::Resolved {
                        generation: request.generation,
                        result,
                    });
                });
            }
        }
    }
}

fn describe_loop(requests: &Receiver<Description>) {
    while let Ok(mut request) = requests.recv() {
        while let Ok(newer) = requests.try_recv() {
            request = newer;
        }
        let (request, events) = request;
        let _work = crate::platform::energy::work(crate::platform::energy::Kind::Resolve);
        let description = describe(
            &Grammars::shared().store(),
            &request.repo_path,
            &request.position,
            &request.head,
            request.index.as_deref(),
        )
        .unwrap_or_else(|error| {
            tracing::debug!(error = %error.user_message(), "describe failed");
            None
        });
        events.send(SymbolsEvent::Described {
            generation: request.generation,
            description,
        });
    }
}

fn run(queue: &Receiver<Job>, latest: &AtomicU64) {
    let mut indexer = None;
    while let Ok(mut job) = queue.recv() {
        while let Ok(newer) = queue.try_recv() {
            job = newer;
        }
        let (request, events) = job;
        let generation = request.generation;
        let cancelled = || latest.load(Ordering::Acquire) != generation;
        if cancelled() {
            continue;
        }
        let mut progress = |done, total| {
            events.send(SymbolsEvent::Progress {
                generation,
                done,
                total,
            });
        };
        let started = Instant::now();
        let _work = crate::platform::energy::work(crate::platform::energy::Kind::Index)
            .detail(request.revision.id.clone());
        let indexer =
            indexer.get_or_insert_with(|| Indexer::new(Grammars::shared().store(), open_cache()));
        let event = match indexer.run(
            &request.repo_path,
            &request.revision.id,
            &cancelled,
            &mut progress,
        ) {
            Ok(Some(index)) => {
                tracing::info!(
                    files = index.files().len(),
                    symbols = index.symbol_count(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "symbols indexed"
                );
                SymbolsEvent::Indexed {
                    generation,
                    index: Arc::new(index),
                }
            }
            Ok(None) => continue,
            Err(error) => SymbolsEvent::Failed {
                generation,
                message: error.user_message(),
            },
        };
        events.send(event);
    }
}
