use crate::actions::SettingsAction;
use crate::effects::Effect;
use crate::events::{AppEvent, SettingsEvent};

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: SettingsAction) -> Vec<Effect> {
    state.apply_settings_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: SettingsEvent) -> Vec<Effect> {
    state.apply_domain_event(AppEvent::from(event))
}
