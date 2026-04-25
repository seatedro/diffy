use crate::actions::UpdateAction;
use crate::effects::Effect;
use crate::events::{AppEvent, UpdateEvent};

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: UpdateAction) -> Vec<Effect> {
    state.apply_update_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: UpdateEvent) -> Vec<Effect> {
    state.apply_domain_event(AppEvent::from(event))
}
