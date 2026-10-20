use std::sync::Arc;
use std::time::Duration;

use diffy_syntax::Catalog;

use crate::core::update::{AvailableUpdate, StagedUpdate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryAction {
    CheckForUpdates,
    RestartToUpdate,
    ToggleLanguages,
    CloseLanguages,
    RetryGrammar { language: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryEffect {
    CheckForUpdates {
        silent: bool,
    },
    StageUpdate {
        update: Box<AvailableUpdate>,
        silent: bool,
    },
    ApplyStagedUpdate(Box<StagedUpdate>),
    WatchGrammars,
    EnsureGrammars {
        languages: Vec<String>,
    },
    RetryGrammar {
        language: String,
    },
    ScheduleUpdatePoll {
        generation: u64,
        after: Duration,
    },
}

#[derive(Debug, Clone)]
pub enum DeliveryEvent {
    UpdateAvailable {
        update: Box<AvailableUpdate>,
        silent: bool,
    },
    UpdateNotAvailable {
        silent: bool,
    },
    UpdateCheckFailed {
        message: String,
        silent: bool,
    },
    UpdateStaged {
        staged: Box<StagedUpdate>,
        silent: bool,
    },
    UpdateInstallFailed {
        message: String,
        silent: bool,
    },
    GrammarsChanged(Arc<Catalog>),
    UpdatePollDue {
        generation: u64,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrammarAvailability {
    pub revision: u64,
    pub installed: Arc<[String]>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::shell::{Action, AppEvent, Effect};

    #[test]
    fn routes_through_root_contracts() {
        assert_eq!(
            Action::from(DeliveryAction::CheckForUpdates),
            Action::Delivery(DeliveryAction::CheckForUpdates)
        );
        assert_eq!(
            Effect::from(DeliveryEffect::WatchGrammars),
            Effect::Delivery(DeliveryEffect::WatchGrammars)
        );
        assert!(matches!(
            AppEvent::from(DeliveryEvent::UpdateNotAvailable { silent: true }),
            AppEvent::Delivery(DeliveryEvent::UpdateNotAvailable { silent: true })
        ));
    }
}
