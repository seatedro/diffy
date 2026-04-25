use crate::actions::CompareAction;
use crate::effects::Effect;
use crate::events::{AppEvent, CompareEvent};

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: CompareAction) -> Vec<Effect> {
    state.apply_compare_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: CompareEvent) -> Vec<Effect> {
    state.apply_domain_event(AppEvent::from(event))
}
