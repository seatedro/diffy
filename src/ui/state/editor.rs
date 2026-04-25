use crate::actions::EditorAction;
use crate::effects::Effect;

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: EditorAction) -> Vec<Effect> {
    state.apply_editor_action(action)
}
