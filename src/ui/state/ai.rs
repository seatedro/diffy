use crate::actions::AiAction;
use crate::effects::Effect;
use crate::events::{AiEvent, AppEvent};

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: AiAction) -> Vec<Effect> {
    state.apply_ai_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: AiEvent) -> Vec<Effect> {
    state.apply_domain_event(AppEvent::from(event))
}
