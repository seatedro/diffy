use halogen::view;

use crate::actions::Action;
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

    let action_tooltip = if state.github.auth.device_flow.is_some() {
        "Open GitHub in your browser"
    } else {
        "Begin GitHub authentication"
    };

    view! { scale,
        <Modal title={"GitHub Device Flow"}
               subtitle={"Authenticate with GitHub to access private repositories and PRs."}
               icon={lucide::SHIELD}
               max_width={Sz::CARD_AUTH * scale}
               window_width={width}
               window_height={height}
               height={Sz::AUTH_MODAL_HEIGHT}>
            <Body>
                <div class="flex-row shrink-0 items-center" gap={Sp::SM}>
                    <icon svg={status_icon} size={Ico::SM} color={tc.text_muted} />
                    <text class="text-sm" color={tc.text_muted}>{status_text}</text>
                </div>
            </Body>
            <Body>
                if let Some(flow) = state.github.auth.device_flow.as_ref() {
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
                }
            </Body>
            <Footer>
                <Button action={action}
                        tooltip={action_tooltip}
                        style={ButtonStyle::Filled}>
                    <Icon>{action_icon}</Icon>
                    <Label>{action_label}</Label>
                </Button>
            </Footer>
        </Modal>
    }
}
