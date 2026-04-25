use crate::actions::SyntaxAction;
use crate::effects::Effect;
use crate::events::{AppEvent, SyntaxEvent};

use super::AppState;

pub(super) fn reduce_action(_state: &mut AppState, action: SyntaxAction) -> Vec<Effect> {
    match action {}
}

pub(super) fn reduce_event(state: &mut AppState, event: SyntaxEvent) -> Vec<Effect> {
    state.apply_domain_event(AppEvent::from(event))
}
