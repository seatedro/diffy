use crate::actions::SettingsAction;
use crate::effects::Effect;
use crate::events::SettingsEvent;

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: SettingsAction) -> Vec<Effect> {
    state.apply_settings_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: SettingsEvent) -> Vec<Effect> {
    match event {
        SettingsEvent::SettingsSaved => Vec::new(),
        SettingsEvent::SettingsSaveFailed { message } => {
            state.push_error(&message);
            Vec::new()
        }
    }
}
