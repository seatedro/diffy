#[cfg(test)]
pub mod fixtures;
#[cfg(test)]
pub mod harness;
pub mod keymap;
mod launcher;
mod launcher_view;
mod list;
mod palette;
pub mod picker;
pub mod platform;
mod ref_picker;
mod ref_picker_view;
pub mod repo_picker;
mod settings_view;
mod state;
#[cfg(test)]
mod tests;
mod toasts;
mod view;

pub use launcher_view::launcher;
pub use platform::{
    PlatformRequest, apply as apply_platform, key_action, menu_action, take_requests,
};
pub use settings_view::settings_slot;
pub use state::{CommandsController, LayerKind, builtin_commands, normalize as normalize_settings};
pub use toasts::{ToastHost, ToastItem};
pub use view::{leader_menu, overlays, status_hints, toasts};
