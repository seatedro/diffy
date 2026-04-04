use crate::ui::actions::Action;
use crate::ui::components::modal::Modal;
use crate::ui::components::picker::picker_list_no_scrollbar;
use crate::ui::design::Sz;
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::{AppState, CompareField, FocusTarget};
use crate::ui::style::Styled;

pub fn ref_picker(
    state: &AppState,
    theme: &crate::ui::theme::Theme,
    field: CompareField,
    width: f32,
    height: f32,
) -> AnyElement {
    let scale = (theme.metrics.ui_font_size / 16.0).max(0.7);
    let (title, icon) = match field {
        CompareField::Left => ("Pick Left Ref", lucide::GIT_BRANCH),
        CompareField::Right => ("Pick Right Ref", lucide::GIT_BRANCH),
    };
    let current_value = match field {
        CompareField::Left => &state.compare.left_ref,
        CompareField::Right => &state.compare.right_ref,
    };

    Modal::new(title, "Search branches, tags, or commits.", icon, Sz::MODAL_SM * scale, width, height)
        .height(Sz::PICKER_HEIGHT)
        .body_child(
            text_input("Filter refs", current_value)
                .placeholder("Search branches, tags, commits")
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
        .into_any()
}
