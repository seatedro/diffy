use quark::view;
use quark_app::ViewContext;
use quark_ui::element::*;
use quark_ui::style::Styled;

use super::MutationsController;
use super::presentation::PresentationStyle;
use crate::app::shell::Slot;
use crate::app::tokens::TextStyle;

pub fn view(mutations: &mut MutationsController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let theme = cx.theme;
    mutations.sync_style(PresentationStyle::from_theme(theme));
    view! {
        <div h={slot.height} class="flex-row items-center shrink-0" test_id="status.changes">
            <text size={TextStyle::BodySmall.font_size(theme)} color={theme.colors.text_muted}>
                "No pending changes"
            </text>
        </div>
    }
}

pub fn panel(
    _mutations: &mut MutationsController,
    _slot: &Slot,
    _cx: &mut ViewContext,
) -> Option<AnyElement> {
    None
}
