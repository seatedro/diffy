use crate::ui::actions::Action;
use crate::ui::components::picker::picker_list;
use crate::ui::design::{Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::state::{AppState, FocusTarget};
use crate::ui::style::Styled;

pub fn repo_picker(state: &AppState, theme: &crate::ui::theme::Theme, width: f32, height: f32) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let panel_width = (Sz::MODAL_XL * scale).min(width - (Sz::MODAL_MARGIN * scale).round());
    let max_list_height = (Sz::REPO_PICKER_HEIGHT * scale).round();

    let placeholder = if cfg!(target_os = "windows") {
        "Search recent or type a path (e.g. C:\\work\\repo)"
    } else {
        "Search recent or type a path (e.g. ~/projects/repo)"
    };

    let panel = div()
        .w(panel_width)
        .flex_col()
        .overflow_hidden()
        .bg(tc.elevated_surface)
        .rounded_lg()
        .border(tc.border)
        .shadow_preset(Shadow::MODAL)
        .on_click(Action::Noop)
        .child(
            div()
                .w_full()
                .px((Sp::MD * scale).round())
                .child(
                    text_input("", &state.overlays.picker.query)
                        .placeholder(placeholder)
                        .focused(state.focus.current == Some(FocusTarget::PickerInput))
                        .on_click(Action::SetFocus(Some(FocusTarget::PickerInput)))
                        .cursor(state.text_edit.cursor)
                        .anchor(state.text_edit.anchor)
                        .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                        .focus_target(FocusTarget::PickerInput)
                        .bare()
                        .w_full()
                        .h((Sz::INPUT * scale).round()),
                ),
        )
        .child(
            div()
                .w_full()
                .h(Sz::SEPARATOR_W)
                .bg(tc.border_variant),
        )
        .child(
            div()
                .p((Sp::XS * scale).round())
                .child(picker_list(
                    &state.overlays.picker.entries,
                    state.overlays.picker.selected_index,
                    state.overlays.picker.list.scroll_top_px as f32,
                    max_list_height,
                    theme,
                )),
        );

    div()
        .absolute()
        .top(0.0)
        .left(0.0)
        .w(width)
        .h(height)
        .z_index(100)
        .flex_col()
        .items_center()
        .bg(tc.overlay_scrim)
        .on_click(Action::CloseOverlay)
        .pt((Sz::MODAL_TOP_OFFSET * scale).round())
        .child(panel)
        .into_any()
}
