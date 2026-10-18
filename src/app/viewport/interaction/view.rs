use diffy_ui::diff_view::decorator::DiffDecorator;
use quark::view;
use quark_app::ViewContext;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::style::Styled;

use super::InteractionController;
use crate::app::shell::{Slot, empty_state};

pub fn view(
    _interaction: &mut InteractionController,
    slot: &Slot,
    _decorator: &dyn DiffDecorator,
    cx: &mut ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    view! {
        <div
            w={slot.width}
            h={slot.height}
            class="flex-col"
            bg={theme.colors.editor_surface}
            accessibility_role={AccessibilityRole::Main}
            aria-label="Diff"
            test_id="diff"
        >
            {empty_state("Nothing to compare", "Open a repository to review its changes", theme)}
        </div>
    }
}
