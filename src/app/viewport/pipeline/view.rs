use quark::view;
use quark_app::ViewContext;
use quark_ui::element::*;
use quark_ui::style::Styled;

use super::PipelineController;
use crate::app::shell::Slot;
use crate::app::tokens::TextStyle;

pub fn view(_pipeline: &mut PipelineController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let theme = cx.theme;
    view! {
        <div h={slot.height} class="flex-row items-center" test_id="status.compare">
            <text size={TextStyle::BodySmall.font_size(theme)} color={theme.colors.text_muted}>
                "No comparison"
            </text>
        </div>
    }
}
