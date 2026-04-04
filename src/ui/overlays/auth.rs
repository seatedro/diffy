use crate::ui::actions::Action;
use crate::ui::components::button::{Button, ButtonStyle};
use crate::ui::components::modal::Modal;
use crate::ui::design::{Ico, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::AppState;
use crate::ui::style::Styled;

pub fn auth_modal(state: &AppState, theme: &crate::ui::theme::Theme, width: f32, height: f32) -> AnyElement {
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
        (lucide::EXTERNAL_LINK, "Open Browser", Action::OpenDeviceFlowBrowser)
    } else {
        (lucide::KEY, "Start Device Flow", Action::StartGitHubDeviceFlow)
    };

    let mut modal = Modal::new(
        "GitHub Device Flow",
        "Authenticate with GitHub to access private repositories and PRs.",
        lucide::SHIELD,
        Sz::CARD_AUTH * scale,
        width,
        height,
    )
    .height(320.0)
    .body_child(
        div()
            .flex_row()
            .flex_shrink_0()
            .items_center()
            .gap((Sp::SM * scale).round())
            .child(svg_icon(status_icon, Ico::SM).color(tc.text_muted))
            .child(text(status_text).text_sm().color(tc.text_muted)),
    );

    if let Some(flow) = state.github.auth.device_flow.as_ref() {
        modal = modal.body_child(
            div()
                .flex_col()
                .gap((Sp::MD * scale).round())
                .p((Sp::MD * scale).round())
                .rounded_md()
                .bg(tc.surface)
                .child(
                    div()
                        .flex_row()
                        .flex_shrink_0()
                        .items_center()
                        .gap((Sp::SM * scale).round())
                        .child(svg_icon(lucide::COPY, Ico::SM).color(tc.text_muted))
                        .child(
                            text(format!("User code: {}", flow.user_code))
                                .mono()
                                .medium()
                                .color(tc.text_strong),
                        ),
                )
                .child(
                    div()
                        .flex_row()
                        .flex_shrink_0()
                        .items_center()
                        .gap((Sp::SM * scale).round())
                        .child(svg_icon(lucide::EXTERNAL_LINK, Ico::SM).color(tc.text_accent))
                        .child(text(&flow.verification_uri).text_sm().color(tc.text_accent)),
                ),
        );
    }

    modal = modal.footer_child(
        Button::new(action)
            .icon(action_icon)
            .label(action_label)
            .style(ButtonStyle::Filled),
    );

    modal.into_any()
}
