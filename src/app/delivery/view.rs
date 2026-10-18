use quark::view;
use quark_app::ViewContext;
use quark_ui::element::*;
use quark_ui::style::Styled;

use super::DeliveryController;
use crate::app::shell::Slot;
use crate::app::tokens::TextStyle;

const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

pub fn view(_delivery: &mut DeliveryController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let theme = cx.theme;
    view! {
        <div h={slot.height} class="flex-row items-center shrink-0" test_id="status.delivery">
            <text size={TextStyle::Caption.font_size(theme)} color={theme.colors.text_muted}>
                {VERSION}
            </text>
        </div>
    }
}
