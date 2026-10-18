use std::sync::Arc;

use crate::core::update::{AvailableUpdate, StagedUpdate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryAction {
    CheckForUpdates,
    InstallUpdate,
    RestartToUpdate,
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
    InstallCommonGrammars,
    EnsureGrammarForPath {
        path: String,
    },
    EnsureGrammarsForPaths {
        paths: Vec<String>,
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
    GrammarInstallStarted {
        language: String,
    },
    GrammarsInstalled {
        languages: Arc<[String]>,
    },
    GrammarInstallFinished {
        language: String,
    },
    GrammarInstallFailed {
        language: String,
    },
}

/// Highlighters drop cached results whose availability revision is older than this one.
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
            Effect::from(DeliveryEffect::InstallCommonGrammars),
            Effect::Delivery(DeliveryEffect::InstallCommonGrammars)
        );
        assert!(matches!(
            AppEvent::from(DeliveryEvent::UpdateNotAvailable { silent: true }),
            AppEvent::Delivery(DeliveryEvent::UpdateNotAvailable { silent: true })
        ));
    }
}
