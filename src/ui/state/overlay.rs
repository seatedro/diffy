use crate::actions::OverlayAction;
use crate::effects::Effect;

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: OverlayAction) -> Vec<Effect> {
    state.apply_overlay_action(action)
}
