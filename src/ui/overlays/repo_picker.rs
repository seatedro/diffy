use crate::ui::actions::Action;
use crate::ui::components::button::{Button, ButtonStyle};
use crate::ui::components::modal::Modal;
use crate::ui::components::picker::picker_list;
use crate::ui::design::Sz;
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::{AppState, FocusTarget};
use crate::ui::style::Styled;

pub fn repo_picker(state: &AppState, theme: &crate::ui::theme::Theme, width: f32, height: f32) -> AnyElement {
    let scale = (theme.metrics.ui_font_size / 16.0).max(0.7);

    Modal::new(
        "Repository Picker",
        "Search or type a path to a git repository.",
        lucide::FOLDER_OPEN,
        Sz::MODAL_XL * scale,
        width,
        height,
    )
    .height(420.0)
    .body_child(
        text_input("Search or type a path", &state.overlays.picker.query)
            .placeholder("C:\\work\\repo")
            .focused(state.focus.current == Some(FocusTarget::PickerInput))
            .on_click(Action::SetFocus(Some(FocusTarget::PickerInput)))
            .cursor(state.text_edit.cursor)
            .anchor(state.text_edit.anchor)
            .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
            .focus_target(FocusTarget::PickerInput)
            .w_full()
            .h(Sz::INPUT * scale),
    )
    .body_child(picker_list(
        &state.overlays.picker.entries,
        state.overlays.picker.selected_index,
        state.overlays.picker.list.scroll_top_px,
        theme,
    ))
    .body_child(
        Button::new(Action::OpenRepositoryDialog)
            .icon(lucide::FOLDER_OPEN)
            .label("Browse Folders")
            .style(ButtonStyle::Subtle),
    )
    .into_any()
}
