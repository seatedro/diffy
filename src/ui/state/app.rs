use crate::actions::AppAction;
use crate::effects::Effect;
use crate::events::UiEvent;

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: AppAction) -> Vec<Effect> {
    state.apply_app_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: UiEvent) -> Vec<Effect> {
    match event {
        UiEvent::RepositoryDialogClosed { path } => {
            path.map_or_else(Vec::new, |path| state.open_repository(path))
        }
        UiEvent::BrowserOpenFailed { message } => {
            state.push_error(&message);
            Vec::new()
        }
    }
}
