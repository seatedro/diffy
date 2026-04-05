use halogen::view;

use crate::ui::actions::Action;
use crate::ui::components::button::{Button, ButtonStyle};
use crate::ui::components::modal::Modal;
use crate::ui::design::{Ico, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::{AppState, AsyncStatus, FocusTarget};
use crate::ui::style::Styled;

pub fn pull_request_modal(
    state: &AppState,
    theme: &crate::ui::theme::Theme,
    width: f32,
    height: f32,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let mut modal = Modal::new(
        "GitHub Pull Request",
        "Paste a PR URL to load its base and head refs for diffing.",
        lucide::GIT_PULL_REQUEST,
        Sz::MODAL_LG * scale,
        width,
        height,
    )
    .height(Sz::PR_MODAL_HEIGHT)
    .body_child(
        text_input("Pull request URL", &state.github.pull_request.url_input)
            .placeholder("https://github.com/owner/repo/pull/42")
            .focused(state.focus.current == Some(FocusTarget::PullRequestInput))
            .on_click(Action::SetFocus(Some(FocusTarget::PullRequestInput)))
            .cursor(state.text_edit.cursor)
            .anchor(state.text_edit.anchor)
            .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
            .focus_target(FocusTarget::PullRequestInput)
            .w_full()
            .h(Sz::INPUT_LABELED * scale),
    );

    if let Some(info) = state.github.pull_request.info.as_ref() {
        modal = modal.body_child(view! { scale,
            <div class="flex-col" gap={Sp::SM} p={Sp::MD} rounded_md bg={tc.surface}>
                <div class="flex-row shrink-0 items-center" gap={Sp::SM}>
                    <icon svg={lucide::GIT_PULL_REQUEST} size={Ico::SM} color={tc.accent} />
                    <text class="font-medium" color={tc.text_strong}>{format!("#{} {}", info.number, info.title)}</text>
                </div>
                <text class="text-sm" color={tc.text_muted}>{"Use this compare to apply the PR base/head refs and start diffing."}</text>
            </div>
        });
    }

    modal = modal.footer_child(
        Button::new(Action::SubmitPullRequest)
            .icon(lucide::PLAY)
            .label(
                if state.github.pull_request.status == AsyncStatus::Loading {
                    "Loading\u{2026}"
                } else {
                    "Load PR"
                },
            )
            .style(ButtonStyle::Filled),
    );

    if state.github.pull_request.info.is_some() {
        modal = modal.footer_child(
            Button::new(Action::UsePullRequestCompare)
                .icon(lucide::GIT_COMPARE)
                .label("Use Compare")
                .style(ButtonStyle::Subtle),
        );
    }

    modal.into_any()
}
