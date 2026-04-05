use halogen::view;

use crate::ui::actions::Action;
use crate::ui::components::button::{Button, ButtonStyle};
use crate::ui::components::modal::Modal;
use crate::ui::design::{Ico, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::AppState;
use crate::ui::style::Styled;

pub fn auth_modal(
    state: &AppState,
    theme: &crate::ui::theme::Theme,
    width: f32,
    height: f32,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let (status_icon, status_text) = if state.github.auth.token_present {
        (lucide::CHECK, "Token stored")
    } else if state.github.auth.device_flow.is_some() {
        (lucide::LOADER, "Waiting for authorization")
    } else {
        (lucide::SHIELD, "Not authenticated")
    };

    let (action_icon, action_label, action) = if state.github.auth.device_flow.is_some() {
        (
            lucide::EXTERNAL_LINK,
            "Open Browser",
            Action::OpenDeviceFlowBrowser,
        )
    } else {
        (
            lucide::KEY,
            "Start Device Flow",
            Action::StartGitHubDeviceFlow,
        )
    };

    let mut modal = Modal::new(
        "GitHub Device Flow",
        "Authenticate with GitHub to access private repositories and PRs.",
        lucide::SHIELD,
        Sz::CARD_AUTH * scale,
        width,
        height,
    )
    .height(Sz::AUTH_MODAL_HEIGHT)
    .body_child(view! { scale,
        <div class="flex-row shrink-0 items-center" gap={Sp::SM}>
            <icon svg={status_icon} size={Ico::SM} color={tc.text_muted} />
            <text class="text-sm" color={tc.text_muted}>{status_text}</text>
        </div>
    });

    if let Some(flow) = state.github.auth.device_flow.as_ref() {
        modal = modal.body_child(view! { scale,
            <div class="flex-col" gap={Sp::MD} p={Sp::MD} rounded_md bg={tc.surface}>
                <div class="flex-row shrink-0 items-center" gap={Sp::SM}>
                    <icon svg={lucide::COPY} size={Ico::SM} color={tc.text_muted} />
                    <text class="font-mono font-medium" color={tc.text_strong}>{format!("User code: {}", flow.user_code)}</text>
                </div>
                <div class="flex-row shrink-0 items-center" gap={Sp::SM}>
                    <icon svg={lucide::EXTERNAL_LINK} size={Ico::SM} color={tc.text_accent} />
                    <text class="text-sm" color={tc.text_accent}>{&flow.verification_uri}</text>
                </div>
            </div>
        });
    }

    modal = modal.footer_child(
        Button::new(action)
            .icon(action_icon)
            .label(action_label)
            .style(ButtonStyle::Filled),
    );

    modal.into_any()
}
