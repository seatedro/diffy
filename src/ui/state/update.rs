use crate::actions::UpdateAction;
use crate::effects::{Effect, UpdateEffect};
use crate::events::UpdateEvent;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: UpdateAction) -> Vec<Effect> {
    state.apply_update_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: UpdateEvent) -> Vec<Effect> {
    match event {
        UpdateEvent::UpdateAvailable { update, silent } => {
            state
                .update
                .set(&state.store, UpdateState::Downloading(update.clone()));
            if !silent {
                state.push_info(&format!("Downloading Diffy {}.", update.version));
            }
            vec![UpdateEffect::StageUpdate { update, silent }.into()]
        }
        UpdateEvent::UpdateNotAvailable { silent } => {
            state.update.set(&state.store, UpdateState::Idle);
            if !silent {
                state.push_info("Diffy is up to date.");
            }
            Vec::new()
        }
        UpdateEvent::UpdateCheckFailed { message, silent } => {
            if !silent {
                state
                    .update
                    .set(&state.store, UpdateState::Failed(message.clone()));
                state.push_error(&message);
            }
            Vec::new()
        }
        UpdateEvent::UpdateStaged { staged, silent } => {
            let version = staged.update.version.clone();
            state
                .update
                .set(&state.store, UpdateState::ReadyToRestart(staged));
            if !silent {
                state.push_info(&format!("Diffy {version} is ready. Restart to update."));
            }
            Vec::new()
        }
        UpdateEvent::UpdateInstallFailed { message, silent } => {
            if silent {
                state.update.set(&state.store, UpdateState::Idle);
            } else {
                state
                    .update
                    .set(&state.store, UpdateState::Failed(message.clone()));
                state.push_error(&message);
            }
            Vec::new()
        }
    }
}

impl AppState {
    pub(super) fn apply_update_action(&mut self, action: UpdateAction) -> Vec<Effect> {
        match action {
            UpdateAction::CheckForUpdates => {
                self.update.set(&self.store, UpdateState::Checking);
                vec![UpdateEffect::CheckForUpdates { silent: false }.into()]
            }
            UpdateAction::InstallUpdate => {
                let update = self.update.get(&self.store);
                if let UpdateState::Available(update) = update {
                    self.update
                        .set(&self.store, UpdateState::Downloading(update.clone()));
                    vec![
                        UpdateEffect::StageUpdate {
                            update,
                            silent: false,
                        }
                        .into(),
                    ]
                } else {
                    Vec::new()
                }
            }
            UpdateAction::RestartToUpdate => {
                let update = self.update.get(&self.store);
                if let UpdateState::ReadyToRestart(staged) = update {
                    self.update
                        .set(&self.store, UpdateState::Restarting(staged.clone()));
                    vec![UpdateEffect::ApplyStagedUpdate(staged).into()]
                } else {
                    Vec::new()
                }
            }
        }
    }
}
