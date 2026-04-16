use halogen::view;

use crate::actions::Action;
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

    view! { scale,
        <Modal title={"GitHub Pull Request"}
               subtitle={"Paste a PR URL to load its base and head refs for diffing."}
               icon={lucide::GIT_PULL_REQUEST}
               max_width={Sz::MODAL_LG * scale}
               window_width={width}
               window_height={height}
               height={Sz::PR_MODAL_HEIGHT}>
            <Body>
                {text_input("Pull request URL", &state.github.pull_request.url_input)
                    .placeholder("https://github.com/owner/repo/pull/42")
                    .focused(state.focus.get(&state.store) == Some(FocusTarget::PullRequestInput))
                    .on_click(Action::SetFocus(Some(FocusTarget::PullRequestInput)))
                    .cursor(state.text_edit.cursor)
                    .anchor(state.text_edit.anchor)
                    .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                    .focus_target(FocusTarget::PullRequestInput)
                    .w_full()
                    .h(Sz::INPUT_LABELED * scale)}
            </Body>
            <Body>
                if let Some(info) = state.github.pull_request.info.as_ref() {
                    <div class="flex-col" gap={Sp::SM} p={Sp::MD} rounded_md bg={tc.surface}>
                        <div class="flex-row shrink-0 items-center" gap={Sp::SM}>
                            <icon svg={lucide::GIT_PULL_REQUEST} size={Ico::SM} color={tc.accent} />
                            <text class="font-medium" color={tc.text_strong}>{format!("#{} {}", info.number, info.title)}</text>
                        </div>
                        <text class="text-sm" color={tc.text_muted}>{"Use this compare to apply the PR base/head refs and start diffing."}</text>
                    </div>
                }
            </Body>
            <Footer>
                <Button action={Action::SubmitPullRequest}
                        tooltip={"Fetch pull request details"}
                        style={ButtonStyle::Filled}>
                    <Icon>{lucide::PLAY}</Icon>
                    <Label>{
                        if state.github.pull_request.status == AsyncStatus::Loading {
                            "Loading\u{2026}"
                        } else {
                            "Load PR"
                        }
                    }</Label>
                </Button>
            </Footer>
            <Footer>
                if state.github.pull_request.info.is_some() {
                    <Button action={Action::UsePullRequestCompare}
                            tooltip={"Apply PR refs to compare"}
                            style={ButtonStyle::Subtle}>
                        <Icon>{lucide::GIT_COMPARE}</Icon>
                        <Label>{"Use Compare"}</Label>
                    </Button>
                }
            </Footer>
        </Modal>
    }
}
