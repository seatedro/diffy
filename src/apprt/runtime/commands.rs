use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::apprt::services::CommandsServices;
use crate::apprt::transport::ScopedEventSender;
use crate::contracts::commands::{CommandsEffect, CommandsEvent};
use crate::platform::persistence::Settings;

pub(crate) const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(250);

pub struct CommandsRuntime {
    save_worker: Sender<SaveRequest>,
}

struct SaveRequest {
    settings: Settings,
    events: ScopedEventSender,
}

impl CommandsRuntime {
    pub fn new(services: CommandsServices) -> Self {
        let (save_worker, receiver) = mpsc::channel();
        thread::spawn(move || save_worker_loop(services, receiver));
        Self { save_worker }
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
            CommandsEffect::LoadFontCatalog => {
                thread::spawn(move || {
                    let (ui, mono) = crate::fonts::scan_font_catalog();
                    events.send(CommandsEvent::FontCatalogReady {
                        ui: ui.into(),
                        mono: mono.into(),
                    });
                });
            }
        }
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
