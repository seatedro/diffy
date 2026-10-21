use std::thread;

use crate::apprt::services::viewport::FilesServices;
use crate::apprt::transport::ScopedEventSender;
use crate::contracts::viewport::files::{FilesEffect, FilesEvent};

pub struct FilesRuntime {
    services: FilesServices,
}

impl FilesRuntime {
    pub fn new(services: FilesServices) -> Self {
        Self { services }
    }

    pub fn dispatch(&self, effect: FilesEffect, events: ScopedEventSender) {
        match effect {
            FilesEffect::Load(request) => {
                let services = self.services;
                thread::spawn(move || {
                    let tab = request.tab;
                    let event = match services.load(&request) {
                        Ok(document) => FilesEvent::Loaded { tab, document },
                        Err(message) => FilesEvent::Failed { tab, message },
                    };
                    events.send(event);
                });
            }
        }
    }
}
