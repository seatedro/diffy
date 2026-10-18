use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::apprt::services::CommandsServices;
use crate::apprt::transport::ScopedEventSender;
use std::sync::Arc;

use crate::apprt::services::viewport::pipeline::PipelineServices;
use crate::contracts::commands::{CommandsEffect, CommandsEvent, RefCatalogRequest};
use crate::contracts::viewport::pipeline::DiffSizeRequest;
use crate::core::ref_catalog::{RefCatalog, build_catalog, filter};
use crate::core::vcs::git::ref_metadata;
use crate::fonts::{FontRole, font_family_entries};
use crate::platform::persistence::Settings;

pub(crate) const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(250);
const DIFF_SIZE_SETTLE: Duration = Duration::from_millis(120);

pub struct CommandsRuntime {
    save_worker: Sender<SaveRequest>,
    ref_worker: Sender<(RefJob, ScopedEventSender)>,
    size_worker: Sender<(DiffSizeRequest, ScopedEventSender)>,
}

enum RefJob {
    Catalog(Box<RefCatalogRequest>),
    Filter {
        generation: u64,
        catalog: Arc<RefCatalog>,
        query: String,
    },
}

struct SaveRequest {
    settings: Settings,
    events: ScopedEventSender,
}

impl CommandsRuntime {
    pub fn new(services: CommandsServices) -> Self {
        let (save_worker, receiver) = mpsc::channel();
        thread::spawn(move || save_worker_loop(services, receiver));
        let (ref_worker, refs) = mpsc::channel();
        thread::spawn(move || ref_worker_loop(refs));
        let (size_worker, sizes) = mpsc::channel();
        thread::spawn(move || size_worker_loop(sizes));
        Self {
            save_worker,
            ref_worker,
            size_worker,
        }
    }

    pub fn dispatch(&self, effect: CommandsEffect, events: ScopedEventSender) {
        match effect {
            CommandsEffect::SaveSettings(settings) => {
                let request = SaveRequest {
                    settings: *settings,
                    events,
                };
                if let Err(mpsc::SendError(request)) = self.save_worker.send(request) {
                    request.events.send(CommandsEvent::SettingsSaveFailed {
                        message: "settings writer stopped".to_owned(),
                    });
                }
            }
            CommandsEffect::LoadRefCatalog(request) => {
                let _ = self.ref_worker.send((RefJob::Catalog(request), events));
            }
            CommandsEffect::FilterRefs {
                generation,
                catalog,
                query,
            } => {
                let job = RefJob::Filter {
                    generation,
                    catalog,
                    query,
                };
                let _ = self.ref_worker.send((job, events));
            }
            CommandsEffect::LoadDiffSize(request) => {
                let _ = self.size_worker.send((*request, events));
            }
            CommandsEffect::LoadFontCatalog => {
                thread::spawn(move || {
                    events.send(CommandsEvent::FontCatalogReady {
                        ui: font_family_entries(FontRole::Ui),
                        mono: font_family_entries(FontRole::Mono),
                    });
                });
            }
        }
    }
}

fn ref_worker_loop(receiver: Receiver<(RefJob, ScopedEventSender)>) {
    while let Ok(first) = receiver.recv() {
        let mut jobs = vec![first];
        jobs.extend(receiver.try_iter());
        let last_filter = jobs
            .iter()
            .rposition(|(job, _)| matches!(job, RefJob::Filter { .. }));
        for (index, (job, events)) in jobs.into_iter().enumerate() {
            match job {
                RefJob::Catalog(request) => {
                    let metadata = if request.git {
                        ref_metadata(&request.repo_path).unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    let snapshot = &request.snapshot;
                    let catalog = build_catalog(
                        &snapshot.refs,
                        &snapshot.changes,
                        &metadata,
                        &request.recent,
                        true,
                    );
                    events.send(CommandsEvent::RefCatalogReady {
                        generation: request.generation,
                        catalog: Arc::new(catalog),
                    });
                }
                RefJob::Filter { .. } if Some(index) != last_filter => {}
                RefJob::Filter {
                    generation,
                    catalog,
                    query,
                } => {
                    let results = filter(&catalog, &query);
                    events.send(CommandsEvent::RefsFiltered {
                        generation,
                        results: Arc::new(results),
                    });
                }
            }
        }
    }
}

fn size_worker_loop(receiver: Receiver<(DiffSizeRequest, ScopedEventSender)>) {
    while let Ok(mut latest) = receiver.recv() {
        thread::sleep(DIFF_SIZE_SETTLE);
        if let Some(next) = receiver.try_iter().last() {
            latest = next;
        }
        let (request, events) = latest;
        let size = PipelineServices.diff_size(&request).ok();
        events.send(CommandsEvent::DiffSizeReady {
            token: request.token,
            size,
        });
    }
}

fn save_worker_loop(services: CommandsServices, receiver: Receiver<SaveRequest>) {
    let mut last_saved: Option<Settings> = None;

    while let Ok(mut pending) = receiver.recv() {
        loop {
            match receiver.recv_timeout(SETTINGS_SAVE_DEBOUNCE) {
                Ok(next) => pending = next,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    persist_settings(&services, &mut last_saved, pending);
                    return;
                }
            }
        }
        persist_settings(&services, &mut last_saved, pending);
    }
}

fn persist_settings(
    services: &CommandsServices,
    last_saved: &mut Option<Settings>,
    request: SaveRequest,
) {
    let SaveRequest { settings, events } = request;
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
            CommandsEvent::SettingsSaved
        }
        Err(error) => CommandsEvent::SettingsSaveFailed {
            message: error.user_message(),
        },
    };
    events.send(event);
}
