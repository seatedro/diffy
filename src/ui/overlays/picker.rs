use halogen::view;

use crate::actions::Action;
use crate::ui::components::picker::picker_list;
use crate::ui::design::{Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::state::{AppState, FocusTarget, PickerItem};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

pub fn picker<T: PickerItem>(
    query: &str,
    placeholder: &str,
    entries: &[T],
    selected_index: usize,
    scroll_top_px: f32,
    panel_width_class: f32,
    focus_target: FocusTarget,
    state: &AppState,
    theme: &Theme,
    width: f32,
    height: f32,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let panel_width = (panel_width_class * scale).min(width - (Sz::MODAL_MARGIN * scale).round());

    view! { scale,
        <div class="absolute flex-col items-center" top={0.0} left={0.0}
             w={width} h={height} z_index={100}
             bg={tc.overlay_scrim} on_click={Action::CloseOverlay}
             pt={Sz::MODAL_TOP_OFFSET}>
            <div class="flex-col overflow-hidden"
                 w={panel_width}
                 bg={tc.elevated_surface}
                 rounded={Rad::XXL}
                 border={tc.border}
                 shadow_preset={Shadow::MODAL}
                 on_click={Action::Noop}>
                <div class="w-full" px={Sp::MD}>
                    {text_input("", query)
                        .placeholder(placeholder)
                        .focused(state.focus.current == Some(focus_target))
                        .on_click(Action::SetFocus(Some(focus_target)))
                        .cursor(state.text_edit.cursor)
                        .anchor(state.text_edit.anchor)
                        .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                        .focus_target(focus_target)
                        .bare()
                        .w_full()
                        .h(theme.metrics.ui_row_height.round())}
                </div>
                <div class="w-full" h={Sz::SEPARATOR_W} bg={tc.border_variant} />
                <div p={Sp::XS}>
                    {picker_list(entries, selected_index, scroll_top_px, Sz::PICKER_MAX_ROWS, theme)}
                </div>
            </div>
        </div>
    }
}
