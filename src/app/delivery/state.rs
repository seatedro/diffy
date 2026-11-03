use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use diffy_syntax::{Catalog, GrammarState, GrammarStatus, LanguageId};
use quark_ui::element::ScrollHandle;

use crate::contracts::commands::{CommandDescriptor, CommandSource, ToastKind, ToastUpdate};
use crate::contracts::delivery::{
    DeliveryAction, DeliveryEffect, DeliveryEvent, GrammarAvailability,
};
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Intent, Reduction};
use crate::contracts::viewport::CompareManifestSnapshot;
use crate::core::keymap::ShortcutCommand;
use crate::core::syntax::grammar;
use crate::core::update::{AvailableUpdate, StagedUpdate};

pub const UPDATE_POLL_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum UpdateState {
    #[default]
    Idle,
    /// A check the user asked for; its result is announced.
    Checking,
    /// A background check; it stays quiet unless the user asks for a check
    /// while it runs.
    CheckingSilently,
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
    catalog: Arc<Catalog>,
    requested: HashSet<String>,
    availability: GrammarAvailability,
    languages_open: bool,
    languages_scroll: ScrollHandle,
    poll_generation: u64,
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
            catalog: Arc::default(),
            requested: HashSet::new(),
            availability: GrammarAvailability::default(),
            languages_open: false,
            languages_scroll: ScrollHandle::new(),
            poll_generation: 0,
        }
    }

    pub fn update(&self) -> &UpdateState {
        &self.update
    }

    pub fn grammars(&self) -> &[GrammarStatus] {
        &self.catalog.statuses
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn languages_open(&self) -> bool {
        self.languages_open
    }

    pub fn languages_scroll(&self) -> &ScrollHandle {
        &self.languages_scroll
    }

    pub fn language_for_path(&self, path: &str) -> Option<&str> {
        let tag = LanguageId::from_path(path)?;
        self.catalog.tags.get(tag.as_str()).map(String::as_str)
    }

    pub fn failed_grammars(&self) -> impl Iterator<Item = &GrammarStatus> {
        self.grammars()
            .iter()
            .filter(|status| matches!(status.state, GrammarState::Failed(_)))
    }

    fn polls_for_updates(&self) -> bool {
        self.updates_supported
            && self.auto_update
            && matches!(self.update, UpdateState::Idle | UpdateState::Failed(_))
    }

    /// `Some(silent)` while a check runs.
    fn running_check(&self) -> Option<bool> {
        match self.update {
            UpdateState::Checking => Some(false),
            UpdateState::CheckingSilently => Some(true),
            _ => None,
        }
    }

    fn check_silently(&mut self) -> Reduction {
        if !self.polls_for_updates() {
            return Reduction::default();
        }
        self.set_update(UpdateState::CheckingSilently)
            .effect(DeliveryEffect::CheckForUpdates { silent: true })
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

    fn staged(&self) -> bool {
        matches!(
            self.update,
            UpdateState::ReadyToRestart(_) | UpdateState::Restarting(_)
        )
    }

    fn schedule_poll(&mut self) -> Reduction {
        self.poll_generation += 1;
        if !self.updates_supported || !self.auto_update || self.staged() {
            return Reduction::default();
        }
        Reduction::default().effect(DeliveryEffect::ScheduleUpdatePoll {
            generation: self.poll_generation,
            after: UPDATE_POLL_INTERVAL,
        })
    }

    fn poll_due(&mut self, generation: u64) -> Reduction {
        if generation != self.poll_generation {
            return Reduction::default();
        }
        let mut reduction = self.check_silently();
        reduction.merge(self.schedule_poll());
        reduction
    }

    fn settings_changed(&mut self, auto_update: bool) -> Reduction {
        if auto_update == self.auto_update {
            return Reduction::default();
        }
        self.auto_update = auto_update;
        let mut reduction = self.check_silently();
        reduction.merge(self.schedule_poll());
        reduction
    }

    fn ensure(&mut self, languages: impl IntoIterator<Item = String>) -> Reduction {
        let missing: Vec<String> = languages
            .into_iter()
            .filter(|language| self.requested.insert(language.clone()))
            .collect();
        if missing.is_empty() {
            return Reduction::default();
        }
        Reduction::default().effect(DeliveryEffect::EnsureGrammars { languages: missing })
    }

    fn startup(&mut self, auto_update: bool) -> Reduction {
        self.auto_update = auto_update;
        let mut reduction = Reduction::default().effect(DeliveryEffect::WatchGrammars);
        reduction.merge(
            self.ensure(
                grammar::COMMON_LANGUAGES
                    .iter()
                    .map(|&language| language.to_owned()),
            ),
        );
        reduction.merge(self.check_silently());
        reduction.merge(self.schedule_poll());
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

    fn grammars_changed(&mut self, catalog: Arc<Catalog>) -> Reduction {
        self.catalog = catalog;
        let installed = |language: &str| {
            self.grammars().iter().any(|status| {
                status.language == language
                    && matches!(status.state, GrammarState::Ready | GrammarState::Installed)
            })
        };
        let mut tags: Vec<String> = self
            .catalog
            .tags
            .iter()
            .filter(|(_, language)| installed(language))
            .map(|(tag, _)| tag.clone())
            .collect();
        tags.sort_unstable();
        let mut reduction = Reduction::changed();
        if *tags != *self.availability.tags {
            self.availability = GrammarAvailability {
                revision: self.availability.revision + 1,
                tags: tags.into(),
            };
            reduction = reduction.intent(Intent::GrammarAvailable(self.availability.clone()));
        }
        reduction
    }

    fn set_update(&mut self, update: UpdateState) -> Reduction {
        self.update = update;
        Reduction::changed()
    }

    fn update_failed(&mut self, message: String, silent: bool) -> Reduction {
        if silent {
            return self.set_update(UpdateState::Idle);
        }
        self.set_update(UpdateState::Failed(message.clone()))
            .intent(Intent::Toast(ToastUpdate::error(message)))
    }
}

pub fn grammar_languages(path: &str) -> impl Iterator<Item = String> {
    LanguageId::from_path(path)
        .map(|language| language.as_str().to_owned())
        .into_iter()
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
                // The background check already running answers the user too.
                if matches!(self.update, UpdateState::CheckingSilently) {
                    return self.set_update(UpdateState::Checking);
                }
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
            DeliveryAction::CloseLanguages => {
                if !self.languages_open {
                    return Reduction::default();
                }
                self.languages_open = false;
                Reduction::changed()
            }
            DeliveryAction::RetryGrammar { language } => {
                let Some(position) = self
                    .grammars()
                    .iter()
                    .position(|status| status.language == language)
                else {
                    return Reduction::default();
                };
                Arc::make_mut(&mut self.catalog).statuses[position].state =
                    GrammarState::Downloading;
                Reduction::changed().effect(DeliveryEffect::RetryGrammar { language })
            }
        }
    }

    fn reduce_event(&mut self, event: Self::Event, _cx: &SharedContext) -> Reduction {
        // Check results go by the running check rather than the flag the
        // check started with: a manual check may have adopted a background
        // one, and a result with no check running changes nothing.
        match event {
            DeliveryEvent::UpdateAvailable { update, .. } => {
                let Some(silent) = self.running_check() else {
                    return Reduction::default();
                };
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
            DeliveryEvent::UpdateNotAvailable { .. } => {
                let Some(silent) = self.running_check() else {
                    return Reduction::default();
                };
                let reduction = self.set_update(UpdateState::Idle);
                if silent {
                    reduction
                } else {
                    reduction.intent(toast(ToastKind::Info, "Diffy is up to date.".to_owned()))
                }
            }
            DeliveryEvent::UpdateCheckFailed { message, .. } => {
                let Some(silent) = self.running_check() else {
                    return Reduction::default();
                };
                self.update_failed(message, silent)
            }
            DeliveryEvent::UpdateInstallFailed { message, silent } => {
                self.update_failed(message, silent)
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
            DeliveryEvent::GrammarsChanged(catalog) => self.grammars_changed(catalog),
            DeliveryEvent::UpdatePollDue { generation } => self.poll_due(generation),
        }
    }

    fn apply_intent(&mut self, intent: &Intent, _cx: &SharedContext) -> Reduction {
        match intent {
            Intent::Startup(startup) => self.startup(startup.settings.auto_update),
            // Update events outlive the workspace, so the poll timer keeps
            // running instead of restarting on every switch.
            Intent::ActivateWorkspace { .. } => {
                Reduction::default().effect(DeliveryEffect::WatchGrammars)
            }
            Intent::PublishManifest(manifest) => self.manifest(manifest),
            Intent::SettingsChanged(snapshot) => {
                self.settings_changed(snapshot.settings.auto_update)
            }
            _ => Reduction::default(),
        }
    }
}

impl CommandSource for DeliveryController {
    fn commands(&self, _cx: &SharedContext, out: &mut Vec<CommandDescriptor>) {
        out.push(
            CommandDescriptor::new(
                "delivery.check_for_updates",
                "Check for updates",
                DeliveryAction::CheckForUpdates,
            )
            .shortcut(ShortcutCommand::CheckUpdates)
            .enabled(!self.busy()),
        );
        if let UpdateState::ReadyToRestart(staged) = &self.update {
            out.push(
                CommandDescriptor::new(
                    "delivery.restart_to_update",
                    "Restart to update",
                    DeliveryAction::RestartToUpdate,
                )
                .detail(format!("Diffy {}", staged.update.version)),
            );
        }
        let mut languages = CommandDescriptor::new(
            "delivery.syntax_languages",
            "Syntax languages",
            DeliveryAction::ToggleLanguages,
        );
        let failed = self.failed_grammars().count();
        if failed > 0 {
            languages = languages.detail(format!("{failed} failed to download or load"));
        }
        out.push(languages);
    }
}

impl crate::app::text_targets::TextHost for DeliveryController {}
