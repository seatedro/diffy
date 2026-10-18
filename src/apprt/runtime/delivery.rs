use std::thread;

use crate::apprt::services::DeliveryServices;
use crate::apprt::transport::ScopedEventSender;
use crate::contracts::delivery::{DeliveryEffect, DeliveryEvent};
use crate::core::update::UpdateCheck;

const COMMON_GRAMMARS_LABEL: &str = "common syntax";

pub struct DeliveryRuntime {
    services: DeliveryServices,
}

impl DeliveryRuntime {
    pub fn new(services: DeliveryServices) -> Self {
        Self { services }
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
            DeliveryEffect::InstallCommonGrammars => {
                thread::spawn(move || install_common_grammars(services, events));
            }
            DeliveryEffect::EnsureGrammarForPath { path } => {
                thread::spawn(move || ensure_grammar_for_path(services, events, path));
            }
            DeliveryEffect::EnsureGrammarsForPaths { paths } => {
                thread::spawn(move || ensure_grammars_for_paths(services, events, paths));
            }
        }
    }
}

fn install_common_grammars(services: DeliveryServices, events: ScopedEventSender) {
    let language = COMMON_GRAMMARS_LABEL.to_owned();
    tracing::info!("syntax pack install started");
    events.send(DeliveryEvent::GrammarInstallStarted {
        language: language.clone(),
    });
    match services.install_common_grammars() {
        Ok(languages) => {
            let installed_count = languages.len();
            if installed_count > 0 {
                events.send(DeliveryEvent::GrammarsInstalled {
                    languages: languages.into(),
                });
            }
            tracing::info!(installed_count, "syntax pack install finished");
            events.send(DeliveryEvent::GrammarInstallFinished { language });
        }
        Err(error) => {
            tracing::warn!(%error, "syntax pack install failed");
            events.send(DeliveryEvent::GrammarInstallFailed { language });
        }
    }
}

fn ensure_grammar_for_path(services: DeliveryServices, events: ScopedEventSender, path: String) {
    let language = services.missing_grammar_for_path(&path);
    if let Some(language) = &language {
        tracing::info!(language, path = %path, "syntax pack install started");
        events.send(DeliveryEvent::GrammarInstallStarted {
            language: language.clone(),
        });
    }
    match services.ensure_grammar_for_path(&path) {
        Ok(installed) => {
            if let Some(installed) = installed {
                tracing::info!(language = %installed, path = %path, "syntax pack installed");
                events.send(DeliveryEvent::GrammarsInstalled {
                    languages: vec![installed].into(),
                });
            }
            if let Some(language) = language {
                events.send(DeliveryEvent::GrammarInstallFinished { language });
            }
        }
        Err(error) => {
            tracing::warn!(path = %path, %error, "syntax pack install failed");
            if let Some(language) = language {
                events.send(DeliveryEvent::GrammarInstallFailed { language });
            }
        }
    }
}

fn ensure_grammars_for_paths(
    services: DeliveryServices,
    events: ScopedEventSender,
    paths: Vec<String>,
) {
    let languages = services.missing_grammars_for_paths(&paths);
    if languages.is_empty() {
        return;
    }
    tracing::info!(
        languages = ?languages,
        path_count = paths.len(),
        "syntax pack batch install started"
    );
    for language in &languages {
        events.send(DeliveryEvent::GrammarInstallStarted {
            language: language.clone(),
        });
    }
    match services.ensure_grammars_for_paths(&paths) {
        Ok(installed) => {
            let installed_count = installed.len();
            if installed_count > 0 {
                events.send(DeliveryEvent::GrammarsInstalled {
                    languages: installed.into(),
                });
            }
            tracing::info!(installed_count, "syntax pack batch install finished");
            for language in languages {
                events.send(DeliveryEvent::GrammarInstallFinished { language });
            }
        }
        Err(error) => {
            tracing::warn!(%error, "syntax pack batch install failed");
            for language in languages {
                events.send(DeliveryEvent::GrammarInstallFailed { language });
            }
        }
    }
}
