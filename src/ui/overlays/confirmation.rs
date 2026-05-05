use halogen::view;

use crate::actions::OverlayAction;
use crate::ui::components::{self, Button, ButtonStyle, Modal};
use crate::ui::design::{Rad, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::AppState;
use crate::ui::style::Styled;

pub fn confirmation_dialog(
    state: &AppState,
    theme: &crate::ui::theme::Theme,
    width: f32,
    height: f32,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let title = state.overlays.confirmation.title.get(&state.store);
    let message = state.overlays.confirmation.message.get(&state.store);
    let confirm_label = state.overlays.confirmation.confirm_label.get(&state.store);

    let body = view! { scale,
        <div class="flex-col" gap={Sp::LG}>
            <div class="flex-row" gap={Sp::SM}
                 p={Sp::MD}
                 rounded={Rad::XL}
                 bg={tc.surface}
                 border_b={tc.status_error}>
                <text class="text-sm" color={tc.text}>{&message}</text>
            </div>
            <div class="flex-row items-center" gap={Sp::XS}>
                {components::kbd("Enter", theme)}
                <text class="text-xs" color={tc.text_muted}>{"or"}</text>
                {components::kbd("y", theme)}
                <text class="text-xs" color={tc.text_muted}>{"to confirm"}</text>
                <spacer />
                {components::kbd("Esc", theme)}
                <text class="text-xs" color={tc.text_muted}>{"or"}</text>
                {components::kbd("n", theme)}
                <text class="text-xs" color={tc.text_muted}>{"to cancel"}</text>
            </div>
        </div>
    };

    view! { scale,
        <Modal title={title}
               subtitle={"This operation rewrites repository state."}
               icon={lucide::ALERT_CIRCLE}
               max_width={Sz::MODAL_MD * scale}
               window_width={width}
               window_height={height}>
            <Body>{body}</Body>
            <Footer>
                {Button::new(OverlayAction::CloseOverlay.into())
                    .label("Cancel")
                    .style(ButtonStyle::Ghost)}
                {Button::new(OverlayAction::ConfirmOverlaySelection.into())
                    .label(confirm_label)
                    .style(ButtonStyle::Danger)}
            </Footer>
        </Modal>
    }
}
