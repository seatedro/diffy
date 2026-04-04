use crate::ui::actions::Action;
use crate::ui::components::button::{Button, ButtonStyle};
use crate::ui::components::modal::Modal;
use crate::ui::components::picker::picker_list_no_scrollbar;
use crate::ui::design::Sz;
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::{AppState, FocusTarget};
use crate::ui::style::Styled;

pub fn repo_picker(state: &AppState, theme: &crate::ui::theme::Theme, width: f32, height: f32) -> AnyElement {
    let scale = theme.metrics.ui_scale();

    Modal::new(
        "Repository Picker",
        "Search or type a path to a git repository.",
        lucide::FOLDER_OPEN,
        Sz::MODAL_XL * scale,
        width,
        height,
    )
    .height(Sz::REPO_PICKER_HEIGHT)
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
            .h(Sz::INPUT_LABELED * scale),
    )
    .body_child(picker_list_no_scrollbar(
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
