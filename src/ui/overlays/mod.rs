pub mod auth;
pub mod compare_menu;
pub mod picker;
pub mod pull_request;
pub mod shortcuts;

pub use auth::auth_modal;
pub use compare_menu::compare_menu;
pub use picker::picker;
pub use pull_request::pull_request_modal;
pub use shortcuts::keyboard_shortcuts;

use crate::ui::design::Sz;
use crate::ui::element::AnyElement;
use crate::ui::state::{AppState, CompareField, FocusTarget, OverlaySurface};
use crate::ui::theme::Theme;

pub fn render_active_overlay(
    state: &mut AppState,
    theme: &Theme,
    width: f32,
    height: f32,
) -> Option<AnyElement> {
    let top = state.overlays.stack.last().cloned()?;
    Some(match top.surface {
        OverlaySurface::RepoPicker => {
            let placeholder = if cfg!(target_os = "windows") {
                "Search recent or type a path (e.g. C:\\work\\repo)"
            } else {
                "Search recent or type a path (e.g. ~/projects/repo)"
            };
            picker(
                &state.overlays.picker.query,
                placeholder,
                &state.overlays.picker.entries,
                state.overlays.picker.selected_index,
                state.overlays.picker.list.scroll_top_px as f32,
                Sz::MODAL_XL,
                FocusTarget::PickerInput,
                state,
                theme,
                width,
                height,
            )
        }
        OverlaySurface::RefPicker(field) => {
            let query = match field {
                CompareField::Left => &state.compare.left_ref,
                CompareField::Right => &state.compare.right_ref,
            };
            picker(
                query,
                "Search branches, tags, commits",
                &state.overlays.picker.entries,
                state.overlays.picker.selected_index,
                state.overlays.picker.list.scroll_top_px as f32,
                Sz::MODAL_XL,
                FocusTarget::PickerInput,
                state,
                theme,
                width,
                height,
            )
        }
        OverlaySurface::CommandPalette => picker(
            &state.overlays.command_palette.query,
            "Type a command, file, repo, or ref",
            &state.overlays.command_palette.entries,
            state.overlays.command_palette.selected_index,
            state.overlays.command_palette.list.scroll_top_px as f32,
            Sz::MODAL_XL,
            FocusTarget::CommandPaletteInput,
            state,
            theme,
            width,
            height,
        ),
        OverlaySurface::PullRequestModal => pull_request_modal(state, theme, width, height),
        OverlaySurface::GitHubAuthModal => auth_modal(state, theme, width, height),
        OverlaySurface::KeyboardShortcuts => keyboard_shortcuts(state, theme, width, height),
        OverlaySurface::CompareMenu => compare_menu(state, theme, width, height),
        OverlaySurface::ThemePicker => picker(
            &state.overlays.picker.query,
            "Search themes\u{2026}",
            &state.overlays.picker.entries,
            state.overlays.picker.selected_index,
            state.overlays.picker.list.scroll_top_px as f32,
            Sz::MODAL_XL,
            FocusTarget::PickerInput,
            state,
            theme,
            width,
            height,
        ),
    })
}
