pub mod auth;
pub mod command_palette;
pub mod compare_sheet;
pub mod pull_request;
pub mod ref_picker;
pub mod repo_picker;
pub mod shortcuts;
pub mod theme_picker;

pub use auth::auth_modal;
pub use command_palette::command_palette;
pub use compare_sheet::compare_sheet;
pub use pull_request::pull_request_modal;
pub use ref_picker::ref_picker;
pub use repo_picker::repo_picker;
pub use shortcuts::keyboard_shortcuts;

use crate::ui::element::AnyElement;
use crate::ui::state::{AppState, OverlaySurface};
use crate::ui::theme::Theme;

pub fn render_active_overlay(
    state: &mut AppState,
    theme: &Theme,
    width: f32,
    height: f32,
) -> Option<AnyElement> {
    let top = state.overlays.stack.last().cloned()?;
    Some(match top.surface {
        OverlaySurface::CompareSheet => compare_sheet(state, theme, width, height),
        OverlaySurface::RepoPicker => repo_picker(state, theme, width, height),
        OverlaySurface::RefPicker(field) => ref_picker(state, theme, field, width, height),
        OverlaySurface::CommandPalette => command_palette(state, theme, width, height),
        OverlaySurface::PullRequestModal => pull_request_modal(state, theme, width, height),
        OverlaySurface::GitHubAuthModal => auth_modal(state, theme, width, height),
        OverlaySurface::KeyboardShortcuts => keyboard_shortcuts(state, theme, width, height),
        OverlaySurface::ThemePicker => theme_picker::theme_picker(state, theme, width, height),
    })
}
