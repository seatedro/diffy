use crate::actions::WorkspaceAction;
use crate::effects::Effect;

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: WorkspaceAction) -> Vec<Effect> {
    state.apply_workspace_action(action)
}
