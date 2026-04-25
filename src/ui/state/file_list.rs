use crate::actions::FileListAction;
use crate::effects::Effect;

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: FileListAction) -> Vec<Effect> {
    state.apply_file_list_action(action)
}
