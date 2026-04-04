use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::ui::actions::Action;
use crate::ui::components::button::{Button, ButtonStyle};
use crate::ui::components::modal::Modal;
use crate::ui::components::segmented::{SegmentedControl, SegmentedItem};
use crate::ui::design::{Ico, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::{AppState, AsyncStatus, FocusTarget};
use crate::ui::style::Styled;

fn ui_scale(theme: &crate::ui::theme::Theme) -> f32 {
    (theme.metrics.ui_font_size / 16.0).max(0.7)
}

pub fn compare_sheet(state: &AppState, theme: &crate::ui::theme::Theme, width: f32, height: f32) -> AnyElement {
    let tc = &theme.colors;
    let scale = ui_scale(theme);

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
            .style(ButtonStyle::Subtle)
    )
    .body_child(
        div()
            .flex_row()
            .gap(Sp::MD)
            .child(
                text_input("Left ref", &state.compare.left_ref)
                    .placeholder("main")
                    .focused(state.focus.current == Some(FocusTarget::CompareLeftRef))
                    .on_click(Action::SetFocus(Some(FocusTarget::CompareLeftRef)))
                    .cursor(state.text_edit.cursor)
                    .anchor(state.text_edit.anchor)
                    .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                    .focus_target(FocusTarget::CompareLeftRef)
                    .w_full()
                    .h(Sz::INPUT_LABELED * scale)
                    .flex_1(),
            )
            .child(
                text_input("Right ref", &state.compare.right_ref)
                    .placeholder("feature")
                    .focused(state.focus.current == Some(FocusTarget::CompareRightRef))
                    .on_click(Action::SetFocus(Some(FocusTarget::CompareRightRef)))
                    .cursor(state.text_edit.cursor)
                    .anchor(state.text_edit.anchor)
                    .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                    .focus_target(FocusTarget::CompareRightRef)
                    .w_full()
                    .h(Sz::INPUT_LABELED * scale)
                    .flex_1(),
            ),
    )
    .body_child(
        div()
            .flex_col()
            .gap(Sp::MD)
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(Sp::MD)
                    .child(text("Mode").text_sm().medium().color(tc.text_muted))
                    .child(SegmentedControl::new(vec![
                        SegmentedItem::new("Single", Action::SetCompareMode(CompareMode::SingleCommit), state.compare.mode == CompareMode::SingleCommit),
                        SegmentedItem::new("Two Dot", Action::SetCompareMode(CompareMode::TwoDot), state.compare.mode == CompareMode::TwoDot),
                        SegmentedItem::new("Three Dot", Action::SetCompareMode(CompareMode::ThreeDot), state.compare.mode == CompareMode::ThreeDot),
                    ])),
            )
            .child(
                div()
                    .flex_row()
                    .gap(Sp::XL)
                    .child(
                        div()
                            .flex_row()
                            .items_center()
                            .gap(Sp::MD)
                            .child(text("Layout").text_sm().medium().color(tc.text_muted))
                            .child(SegmentedControl::new(vec![
                                SegmentedItem::new("Unified", Action::SetLayoutMode(LayoutMode::Unified), state.compare.layout == LayoutMode::Unified),
                                SegmentedItem::new("Split", Action::SetLayoutMode(LayoutMode::Split), state.compare.layout == LayoutMode::Split),
                            ])),
                    )
                    .child(
                        div()
                            .flex_row()
                            .items_center()
                            .gap(Sp::MD)
                            .child(text("Engine").text_sm().medium().color(tc.text_muted))
                            .child(SegmentedControl::new(vec![
                                SegmentedItem::new("Built-in", Action::SetRenderer(RendererKind::Builtin), state.compare.renderer == RendererKind::Builtin),
                                SegmentedItem::new("Difftastic", Action::SetRenderer(RendererKind::Difftastic), state.compare.renderer == RendererKind::Difftastic),
                            ])),
                    ),
            ),
    )
    .body_child({
        let validation = state.overlays.compare_sheet.validation_message.as_deref();
        if let Some(msg) = validation {
            div()
                .w_full()
                .flex_row()
                .flex_shrink_0()
                .items_center()
                .gap(Sp::SM)
                .child(svg_icon(lucide::ALERT_CIRCLE, Ico::SM).color(tc.status_error))
                .child(
                    div()
                        .flex_1()
                        .min_w(0.0)
                        .child(text(msg).text_sm().color(tc.status_error).truncate()),
                )
        } else {
            div()
        }
    })
    .footer_child(
        Button::new(Action::StartCompare)
            .icon(lucide::PLAY)
            .label(if state.workspace.status == AsyncStatus::Loading {
                "Comparing\u{2026}"
            } else {
                "Start Compare"
            })
            .style(ButtonStyle::Filled),
    )
    .into_any()
}
