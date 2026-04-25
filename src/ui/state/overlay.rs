use crate::actions::OverlayAction;
use crate::effects::Effect;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: OverlayAction) -> Vec<Effect> {
    state.apply_overlay_action(action)
}

impl AppState {
    pub(super) fn apply_overlay_action(&mut self, action: OverlayAction) -> Vec<Effect> {
        use OverlayAction::*;
        match action {
            OpenRepoPicker => {
                self.open_repo_picker();
                Vec::new()
            }
            OpenRefPicker(field) => self.open_ref_picker(field),
            OpenCommandPalette => self.open_command_palette(),
            OpenGitHubAuthModal => {
                self.push_overlay(
                    OverlaySurface::GitHubAuthModal,
                    Some(FocusTarget::AuthPrimaryAction),
                );
                Vec::new()
            }
            CloseOverlay => {
                if self.overlays_top() == Some(OverlaySurface::RefPicker) {
                    return self.cancel_ref_picker();
                }
                self.pop_overlay();
                Vec::new()
            }
            MoveOverlaySelection(delta) => {
                self.move_overlay_selection(delta);
                Vec::new()
            }
            ConfirmOverlaySelection => self.confirm_overlay_selection(),
            TabCompletePickerDir => {
                self.tab_complete_picker_dir();
                Vec::new()
            }
            SelectOverlayEntry(index) => {
                self.select_overlay_entry(index);
                self.confirm_overlay_selection()
            }
            HoverOverlayEntry(Some(index)) => {
                self.overlays
                    .picker
                    .hovered_index
                    .set(&self.store, Some(index));
                self.select_overlay_entry(index);
                Vec::new()
            }
            HoverOverlayEntry(None) => {
                self.overlays.picker.hovered_index.set(&self.store, None);
                Vec::new()
            }
            ScrollActiveOverlayListPx(delta_px) => {
                self.scroll_active_overlay_list_px(delta_px);
                Vec::new()
            }
            ShowKeyboardShortcuts => {
                if self.overlays_top() == Some(OverlaySurface::KeyboardShortcuts) {
                    self.pop_overlay();
                } else {
                    self.push_overlay(OverlaySurface::KeyboardShortcuts, None);
                }
                Vec::new()
            }
        }
    }
}
