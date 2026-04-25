use crate::actions::AppAction;
use crate::effects::Effect;
use crate::events::{AppEvent, UiEvent};

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: AppAction) -> Vec<Effect> {
    state.apply_app_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: UiEvent) -> Vec<Effect> {
    state.apply_domain_event(AppEvent::from(event))
}
