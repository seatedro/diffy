use crate::actions::GitHubAction;
use crate::effects::Effect;
use crate::events::{AppEvent, GitHubEvent};

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: GitHubAction) -> Vec<Effect> {
    state.apply_github_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: GitHubEvent) -> Vec<Effect> {
    state.apply_domain_event(AppEvent::from(event))
}
