use crate::actions::WorkspaceAction;
use crate::effects::Effect;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: WorkspaceAction) -> Vec<Effect> {
    state.apply_workspace_action(action)
}

impl AppState {
    pub(super) fn apply_workspace_action(&mut self, action: WorkspaceAction) -> Vec<Effect> {
        match action {
            WorkspaceAction::OpenRepository(path) => self.open_repository(path),
            WorkspaceAction::ShowWorkingTree => self.show_working_tree(),
        }
    }
}
