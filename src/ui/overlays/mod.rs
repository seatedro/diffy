pub mod account_menu;
pub mod auth;
pub mod compare_menu;
pub mod picker;
pub mod shortcuts;

pub use account_menu::account_menu;
pub use auth::auth_modal;
pub use compare_menu::compare_menu;
pub use picker::picker;
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
    let top = state
        .overlays
        .stack
        .with(&state.store, |stack| stack.last().cloned())?;
    let picker_query = state
        .overlays
        .picker
        .query
        .with(&state.store, |s| s.clone());
    let picker_entries = state
        .overlays
        .picker
        .entries
        .with(&state.store, |e| e.clone());
    let picker_selected = state.overlays.picker.selected_index.get(&state.store);
    let picker_scroll = state
        .overlays
        .picker
        .list
        .with(&state.store, |l| l.scroll_top_px);
    let palette_query = state
        .overlays
        .command_palette
        .query
        .with(&state.store, |s| s.clone());
    let palette_entries = state
        .overlays
        .command_palette
        .entries
        .with(&state.store, |e| e.clone());
    let palette_selected = state
        .overlays
        .command_palette
        .selected_index
        .get(&state.store);
    let palette_scroll = state
        .overlays
        .command_palette
        .list
        .with(&state.store, |l| l.scroll_top_px);
    Some(match top.surface {
        OverlaySurface::RepoPicker => {
            let placeholder = if cfg!(target_os = "windows") {
                "Search recent or type a path (e.g. C:\\work\\repo)"
            } else {
                "Search recent or type a path (e.g. ~/projects/repo)"
            };
            picker(
                &picker_query,
                placeholder,
                &picker_entries,
                picker_selected,
                picker_scroll as f32,
                Sz::MODAL_XL,
                FocusTarget::PickerInput,
                state,
                theme,
                width,
                height,
            )
        }
        OverlaySurface::RefPicker => {
            let field = state.overlays.ref_picker.active_field.get(&state.store);
            let query = match field {
                CompareField::Left => state.compare.left_ref.get(&state.store),
                CompareField::Right => state.compare.right_ref.get(&state.store),
            };
            let profile = state.repository.location.with(&state.store, |location| {
                crate::ui::vcs::profile(location.as_ref())
            });
            // Panel flush with the title bar: the title-bar segmented control
            // is the header; scrim begins below the title bar so those chips
            // stay interactive.
            let top_offset = theme.metrics.title_bar_height / theme.metrics.ui_scale();
            let layout = picker::PickerLayout {
                top_offset: Some(top_offset),
                // Square corners so the panel looks continuous with the title
                // bar. Halogen has uniform corner rounding only, so bottom
                // corners are square too — the shadow carries the silhouette.
                panel_radius: Some(0.0),
            };
            picker::picker_with_header(
                &query,
                profile.ref_picker_placeholder(),
                &picker_entries,
                picker_selected,
                picker_scroll as f32,
                Sz::MODAL_XL,
                FocusTarget::PickerInput,
                state,
                theme,
                width,
                height,
                None,
                layout,
            )
        }
        OverlaySurface::CommandPalette => picker(
            &palette_query,
            "Type a command, file, repo, or ref",
            &palette_entries,
            palette_selected,
            palette_scroll as f32,
            Sz::MODAL_XL,
            FocusTarget::CommandPaletteInput,
            state,
            theme,
            width,
            height,
        ),
        OverlaySurface::GitHubAuthModal => auth_modal(state, theme, width, height),
        OverlaySurface::KeyboardShortcuts => keyboard_shortcuts(state, theme, width, height),
        OverlaySurface::CompareMenu => compare_menu(state, theme, width, height),
        OverlaySurface::AccountMenu => account_menu(state, theme, width, height),
        OverlaySurface::ThemePicker => picker(
            &picker_query,
            "Search themes\u{2026}",
            &picker_entries,
            picker_selected,
            picker_scroll as f32,
            Sz::MODAL_XL,
            FocusTarget::PickerInput,
            state,
            theme,
            width,
            height,
        ),
    })
}
