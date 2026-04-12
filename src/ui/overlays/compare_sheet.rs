use halogen::view;

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::actions::Action;
use crate::ui::components::button::{Button, ButtonStyle};
use crate::ui::components::modal::Modal;
use crate::ui::components::segmented::{SegmentedControl, SegmentedItem};
use crate::ui::design::{Ico, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::{AppState, AsyncStatus, FocusTarget};
use crate::ui::style::Styled;

fn ui_scale(theme: &crate::ui::theme::Theme) -> f32 {
    theme.metrics.ui_scale()
}

pub fn compare_sheet(
    state: &AppState,
    theme: &crate::ui::theme::Theme,
    width: f32,
    height: f32,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = ui_scale(theme);

    let left_input = text_input("Left ref", &state.compare.left_ref)
        .placeholder("main")
        .focused(state.focus.current == Some(FocusTarget::CompareLeftRef))
        .on_click(Action::SetFocus(Some(FocusTarget::CompareLeftRef)))
        .cursor(state.text_edit.cursor)
        .anchor(state.text_edit.anchor)
        .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
        .focus_target(FocusTarget::CompareLeftRef)
        .w_full()
        .h(Sz::INPUT_LABELED * scale)
        .flex_1();

    let right_input = text_input("Right ref", &state.compare.right_ref)
        .placeholder("feature")
        .focused(state.focus.current == Some(FocusTarget::CompareRightRef))
        .on_click(Action::SetFocus(Some(FocusTarget::CompareRightRef)))
        .cursor(state.text_edit.cursor)
        .anchor(state.text_edit.anchor)
        .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
        .focus_target(FocusTarget::CompareRightRef)
        .w_full()
        .h(Sz::INPUT_LABELED * scale)
        .flex_1();

    let refs_row = view! { scale,
        <div class="flex-row" gap={Sp::MD}>
            {left_input}
            {right_input}
        </div>
    };

    let options = view! { scale,
        <div class="flex-col" gap={Sp::MD}>
            <div class="flex-row items-center" gap={Sp::MD}>
                <text class="text-sm font-medium" color={tc.text_muted}>{"Mode"}</text>
                {SegmentedControl::new(vec![
                    SegmentedItem::new("Single", Action::SetCompareMode(CompareMode::SingleCommit), state.compare.mode == CompareMode::SingleCommit),
                    SegmentedItem::new("Two Dot", Action::SetCompareMode(CompareMode::TwoDot), state.compare.mode == CompareMode::TwoDot),
                    SegmentedItem::new("Three Dot", Action::SetCompareMode(CompareMode::ThreeDot), state.compare.mode == CompareMode::ThreeDot),
                ])}
            </div>
            <div class="flex-row flex-wrap" gap={Sp::MD}>
                <div class="flex-row items-center" gap={Sp::MD}>
                    <text class="text-sm font-medium" color={tc.text_muted}>{"Layout"}</text>
                    {SegmentedControl::new(vec![
                        SegmentedItem::new("Unified", Action::SetLayoutMode(LayoutMode::Unified), state.compare.layout == LayoutMode::Unified),
                        SegmentedItem::new("Split", Action::SetLayoutMode(LayoutMode::Split), state.compare.layout == LayoutMode::Split),
                    ])}
                </div>
                <div class="flex-row items-center" gap={Sp::MD}>
                    <text class="text-sm font-medium" color={tc.text_muted}>{"Engine"}</text>
                    {SegmentedControl::new(vec![
                        SegmentedItem::new("Built-in", Action::SetRenderer(RendererKind::Builtin), state.compare.renderer == RendererKind::Builtin),
                        SegmentedItem::new("Difftastic", Action::SetRenderer(RendererKind::Difftastic), state.compare.renderer == RendererKind::Difftastic),
                    ])}
                </div>
            </div>
        </div>
    };

    let validation = state.overlays.compare_sheet.validation_message.as_deref();
    let validation_row: AnyElement = if let Some(msg) = validation {
        view! { scale,
            <div class="w-full flex-row shrink-0 items-center" gap={Sp::SM}>
                <icon svg={lucide::ALERT_CIRCLE} size={Ico::SM} color={tc.status_error} />
                <div class="flex-1" min_w={0.0}>
                    <text class="text-sm truncate" color={tc.status_error}>{msg}</text>
                </div>
            </div>
        }
    } else {
        div().into_any()
    };

    Modal::new(
        "Compare Setup",
        "Pick a repository, refs, compare mode, and renderer.",
        lucide::GIT_COMPARE,
        Sz::MODAL_MD * scale,
        width,
        height,
    )
    .gap(Sp::XL)
    .body_child(
        Button::new(Action::OpenRepoPicker)
            .icon(lucide::FOLDER)
            .label(
                state
                    .compare
                    .repo_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "Choose repository\u{2026}".into()),
            )
            .tooltip("Switch repository")
            .style(ButtonStyle::Subtle),
    )
    .body_child(refs_row)
    .body_child(options)
    .body_child(validation_row)
    .footer_child(
        Button::new(Action::StartCompare)
            .icon(lucide::PLAY)
            .label(if state.workspace.status == AsyncStatus::Loading {
                "Comparing\u{2026}"
            } else {
                "Start Compare"
            })
            .tooltip("Run diff comparison")
            .style(ButtonStyle::Filled),
    )
    .into_any()
}
