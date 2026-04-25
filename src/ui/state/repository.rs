use crate::actions::RepositoryAction;
use crate::effects::Effect;
use crate::events::{AppEvent, RepositoryEvent};

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: RepositoryAction) -> Vec<Effect> {
    state.apply_repository_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: RepositoryEvent) -> Vec<Effect> {
    state.apply_domain_event(AppEvent::from(event))
}
