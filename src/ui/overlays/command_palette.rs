use halogen::view;

use crate::ui::actions::Action;
use crate::ui::components::picker::picker_list;
use crate::ui::design::{Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::state::{AppState, FocusTarget};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

pub fn command_palette(state: &AppState, theme: &Theme, width: f32, height: f32) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let panel_width = (Sz::MODAL_MD * scale).min(width - (Sz::MODAL_MARGIN * scale).round());

    let panel = div()
        .w(panel_width)
        .flex_col()
        .overflow_hidden()
        .bg(tc.elevated_surface)
        .rounded_lg()
        .border(tc.border)
        .shadow_preset(Shadow::MODAL)
        .on_click(Action::Noop)
        .child(view! { scale,
            <div class="w-full" px={Sp::MD}>
                {text_input("", &state.overlays.command_palette.query)
                    .placeholder("Type a command, file, repo, or ref")
                    .focused(state.focus.current == Some(FocusTarget::CommandPaletteInput))
                    .on_click(Action::SetFocus(Some(FocusTarget::CommandPaletteInput)))
                    .cursor(state.text_edit.cursor)
                    .anchor(state.text_edit.anchor)
                    .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                    .focus_target(FocusTarget::CommandPaletteInput)
                    .bare()
                    .w_full()
                    .h((Sz::INPUT * scale).round())}
            </div>
        })
        .child(view! {
            <div class="w-full" h={Sz::SEPARATOR_W} bg={tc.border_variant} />
        })
        .child(view! { scale,
            <div p={Sp::XS}>
                {picker_list(
                    &state.overlays.command_palette.entries,
                    state.overlays.command_palette.selected_index,
                    state.overlays.command_palette.list.scroll_top_px as f32,
                    state.overlays.command_palette.list.viewport_height_px as f32,
                    theme,
                )}
            </div>
        });

    view! { scale,
        <div class="absolute flex-col items-center" top={0.0} left={0.0}
             w={width} h={height} z_index={100}
             bg={tc.overlay_scrim} on_click={Action::CloseOverlay}
             pt={Sz::MODAL_TOP_OFFSET}>
            {panel}
        </div>
    }
}
