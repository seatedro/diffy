use std::collections::HashSet;
use std::sync::Arc;

use diffy_syntax::{GrammarState, GrammarStatus};

use crate::contracts::commands::{ToastKind, ToastUpdate};
use crate::contracts::delivery::{
    DeliveryAction, DeliveryEffect, DeliveryEvent, GrammarAvailability,
};
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Intent, Reduction};
use crate::contracts::viewport::CompareManifestSnapshot;
use crate::core::syntax::grammar;
use crate::core::update::{AvailableUpdate, StagedUpdate};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    Downloading(AvailableUpdate),
    ReadyToRestart(StagedUpdate),
    Restarting(StagedUpdate),
    Failed(String),
}

#[derive(Debug)]
pub struct DeliveryController {
    updates_supported: bool,
    auto_update: bool,
    update: UpdateState,
    grammars: Arc<[GrammarStatus]>,
    requested: HashSet<&'static str>,
    availability: GrammarAvailability,
    languages_open: bool,
}

impl Default for DeliveryController {
    fn default() -> Self {
        Self::new(crate::core::update::updates_configured() && !cfg!(debug_assertions))
    }
}

impl DeliveryController {
    pub fn new(updates_supported: bool) -> Self {
        Self {
            updates_supported,
            auto_update: false,
            update: UpdateState::Idle,
            grammars: Arc::from([]),
            requested: HashSet::new(),
            availability: GrammarAvailability::default(),
            languages_open: false,
        }
    }

    pub fn update(&self) -> &UpdateState {
        &self.update
    }

    pub fn grammars(&self) -> &[GrammarStatus] {
        &self.grammars
    }

    pub fn languages_open(&self) -> bool {
        self.languages_open
    }

    pub fn failed_grammars(&self) -> impl Iterator<Item = &GrammarStatus> {
        self.grammars
            .iter()
            .filter(|status| matches!(status.state, GrammarState::Failed(_)))
    }

    fn polls_for_updates(&self) -> bool {
        self.updates_supported && self.auto_update && !self.busy()
    }

    fn busy(&self) -> bool {
        matches!(
            self.update,
            UpdateState::Checking
                | UpdateState::Downloading(_)
                | UpdateState::ReadyToRestart(_)
                | UpdateState::Restarting(_)
        )
    }

    fn ensure(&mut self, languages: impl IntoIterator<Item = &'static str>) -> Reduction {
        let missing: Vec<&'static str> = languages
            .into_iter()
            .filter(|language| self.requested.insert(language))
            .collect();
        if missing.is_empty() {
            return Reduction::default();
        }
        Reduction::default().effect(DeliveryEffect::EnsureGrammars { languages: missing })
    }

    fn startup(&mut self, auto_update: bool) -> Reduction {
        self.auto_update = auto_update;
        let mut reduction = Reduction::default().effect(DeliveryEffect::WatchGrammars);
        reduction.merge(self.ensure(grammar::common_languages().map(|language| language.name)));
        if self.polls_for_updates() {
            reduction = reduction.effect(DeliveryEffect::CheckForUpdates { silent: true });
        }
        reduction
    }

    fn manifest(&mut self, manifest: &CompareManifestSnapshot) -> Reduction {
        let mut languages = Vec::new();
        for file in manifest.files.iter() {
            for path in [file.paths.old_path(), file.paths.new_path()]
                .into_iter()
                .flatten()
            {
                languages.extend(grammar_languages(&path));
            }
        }
        self.ensure(languages)
    }

    fn grammars_changed(&mut self, grammars: Arc<[GrammarStatus]>) -> Reduction {
        self.grammars = grammars;
        let installed: Vec<String> = self
            .grammars
            .iter()
            .filter(|status| matches!(status.state, GrammarState::Ready | GrammarState::Installed))
            .map(|status| status.language.to_owned())
            .collect();
        let mut reduction = Reduction::changed();
        if *installed != *self.availability.installed {
            self.availability = GrammarAvailability {
                revision: self.availability.revision + 1,
                installed: installed.into(),
            };
            reduction = reduction.intent(Intent::GrammarAvailable(self.availability.clone()));
        }
        reduction
    }

    fn set_update(&mut self, update: UpdateState) -> Reduction {
        self.update = update;
        Reduction::changed()
    }
}

pub fn grammar_languages(path: &str) -> impl Iterator<Item = &'static str> {
    let highlight = grammar::language_for_path(path).map(|language| language.name);
    let semantic = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .filter(|extension| matches!(extension.as_str(), "js" | "mjs" | "jsx"))
        .map(|_| "javascript");
    highlight.into_iter().chain(semantic)
}

fn toast(kind: ToastKind, message: String) -> Intent {
    Intent::Toast(ToastUpdate::Show {
        id: None,
        kind,
        message,
        description: None,
    })
}

impl Controller for DeliveryController {
    type Action = DeliveryAction;
    type Event = DeliveryEvent;

    fn reduce_action(&mut self, action: Self::Action, _cx: &SharedContext) -> Reduction {
        match action {
            DeliveryAction::CheckForUpdates => {
                if self.busy() {
                    return Reduction::default();
                }
                self.set_update(UpdateState::Checking)
                    .effect(DeliveryEffect::CheckForUpdates { silent: false })
            }
            DeliveryAction::RestartToUpdate => match &self.update {
                UpdateState::ReadyToRestart(staged) => {
                    let staged = staged.clone();
                    self.set_update(UpdateState::Restarting(staged.clone()))
                        .effect(DeliveryEffect::ApplyStagedUpdate(Box::new(staged)))
                }
                _ => Reduction::default(),
            },
            DeliveryAction::ToggleLanguages => {
                self.languages_open = !self.languages_open;
                Reduction::changed()
            }
            DeliveryAction::RetryGrammar { language } => {
                let Some(position) = self
                    .grammars
                    .iter()
                    .position(|status| status.language == language)
                else {
                    return Reduction::default();
                };
                let mut grammars = self.grammars.to_vec();
                grammars[position].state = GrammarState::Downloading;
                self.grammars = grammars.into();
                Reduction::changed().effect(DeliveryEffect::RetryGrammar { language })
            }
        }
    }

    fn reduce_event(&mut self, event: Self::Event, _cx: &SharedContext) -> Reduction {
        match event {
            DeliveryEvent::UpdateAvailable { update, silent } => {
                let message = format!("Downloading Diffy {}.", update.version);
                let reduction = self
                    .set_update(UpdateState::Downloading((*update).clone()))
                    .effect(DeliveryEffect::StageUpdate { update, silent });
                if silent {
                    reduction
                } else {
                    reduction.intent(toast(ToastKind::Info, message))
                }
            }
            DeliveryEvent::UpdateNotAvailable { silent } => {
                let reduction = self.set_update(UpdateState::Idle);
                if silent {
                    reduction
                } else {
                    reduction.intent(toast(ToastKind::Info, "Diffy is up to date.".to_owned()))
                }
            }
            DeliveryEvent::UpdateCheckFailed { message, silent }
            | DeliveryEvent::UpdateInstallFailed { message, silent } => {
                if silent {
                    return self.set_update(UpdateState::Idle);
                }
                self.set_update(UpdateState::Failed(message.clone()))
                    .intent(Intent::Toast(ToastUpdate::error(message)))
            }
            DeliveryEvent::UpdateStaged { staged, silent } => {
                let message = format!(
                    "Diffy {} is ready. Restart to update.",
                    staged.update.version
                );
                let reduction = self.set_update(UpdateState::ReadyToRestart(*staged));
                if silent {
                    reduction
                } else {
                    reduction.intent(toast(ToastKind::Success, message))
                }
            }
            DeliveryEvent::GrammarsChanged(grammars) => self.grammars_changed(grammars),
        }
    }

    fn apply_intent(&mut self, intent: &Intent, _cx: &SharedContext) -> Reduction {
        match intent {
            Intent::Startup(startup) => self.startup(startup.settings.auto_update),
            Intent::ActivateWorkspace { .. } => {
                Reduction::default().effect(DeliveryEffect::WatchGrammars)
            }
            Intent::PublishManifest(manifest) => self.manifest(manifest),
            Intent::SettingsChanged(snapshot) => {
                let enabled = snapshot.settings.auto_update && !self.auto_update;
                self.auto_update = snapshot.settings.auto_update;
                if enabled && self.polls_for_updates() {
                    return Reduction::default()
                        .effect(DeliveryEffect::CheckForUpdates { silent: true });
                }
                Reduction::default()
            }
            _ => Reduction::default(),
        }
    }
}

impl crate::app::text_targets::TextHost for DeliveryController {}
