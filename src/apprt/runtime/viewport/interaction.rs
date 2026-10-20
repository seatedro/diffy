use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use crate::apprt::services::viewport::InteractionServices;
use crate::apprt::transport::ScopedEventSender;
use crate::contracts::viewport::interaction::{InteractionEffect, InteractionEvent};

pub struct InteractionRuntime {
    services: InteractionServices,
    current_search: Arc<AtomicU64>,
}

impl InteractionRuntime {
    pub fn new(services: InteractionServices) -> Self {
        Self {
            services,
            current_search: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn dispatch(&self, effect: InteractionEffect, events: ScopedEventSender) {
        match effect {
            InteractionEffect::Search(request) => {
                let job = request.job;
                if self.current_search.fetch_max(job.0, Ordering::AcqRel) > job.0 {
                    return;
                }
                let services = self.services;
                let current = Arc::clone(&self.current_search);
                thread::spawn(move || {
                    let result = services.search(&request);
                    if current.load(Ordering::Acquire) > job.0 {
                        return;
                    }
                    let event = match result {
                        Ok(matches) => InteractionEvent::SearchFinished { job, matches },
                        Err(failure) => InteractionEvent::SearchFailed { job, failure },
                    };
                    events.send(event);
                });
            }
            InteractionEffect::CancelSearch(job) => {
                self.current_search
                    .fetch_max(job.0.saturating_add(1), Ordering::AcqRel);
            }
        }
    }
}
