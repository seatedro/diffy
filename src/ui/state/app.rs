use crate::actions::AppAction;
use crate::effects::Effect;
use crate::events::UiEvent;

use super::*;

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

impl AppState {
    pub(super) fn apply_app_action(&mut self, action: AppAction) -> Vec<Effect> {
        match action {
            AppAction::Bootstrap => Vec::new(),
            AppAction::OpenRepositoryDialog => vec![UiEffect::OpenRepositoryDialog.into()],
            AppAction::SetFocus(target) => {
                self.set_focus(target);
                Vec::new()
            }
            AppAction::CopyText(text) => {
                self.push_info("Copied to clipboard.");
                vec![UiEffect::SetClipboard(text).into()]
            }
            AppAction::OpenContextMenu { entries, x, y } => {
                self.context_menu.open(entries, x as f32, y as f32);
                Vec::new()
            }
            AppAction::CloseContextMenu => {
                self.context_menu.close();
                Vec::new()
            }
            AppAction::DismissToast(index) => {
                self.toasts.update(&self.store, |toasts| {
                    if index < toasts.len() {
                        toasts.remove(index);
                    }
                });
                Vec::new()
            }
            AppAction::HoverToast(index) => {
                let mut was_any_hovered = false;
                let mut is_any_hovered = false;
                self.toasts.update(&self.store, |toasts| {
                    was_any_hovered = toasts.iter().any(|t| t.hovered);
                    let hovered_id = index.and_then(|i| toasts.get(i)).map(|t| t.id);
                    for toast in toasts.iter_mut() {
                        toast.hovered = Some(toast.id) == hovered_id;
                    }
                    is_any_hovered = toasts.iter().any(|t| t.hovered);
                });
                if was_any_hovered != is_any_hovered {
                    use crate::ui::animation::AnimationKey;
                    let target = if is_any_hovered { 1.0 } else { 0.0 };
                    self.animation.set_target(
                        AnimationKey::ToastStackFan,
                        target,
                        150,
                        self.clock_ms,
                    );
                }
                Vec::new()
            }
            AppAction::ToggleDebugOverlay => {
                let visible = self.debug.overlay_visible.get(&self.store);
                self.store.write(self.debug.overlay_visible, !visible);
                Vec::new()
            }
            AppAction::Noop => Vec::new(),
        }
    }
}
