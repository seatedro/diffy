use crate::core::compare::CompareMode;
use crate::ui::actions::Action;
use crate::ui::components::Button;
use crate::ui::design::{Ico, Rad, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{AppState, AsyncStatus, CompareField, OverlaySurface, WorkspaceMode};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

pub(crate) fn title_bar(state: &AppState, theme: &Theme, sidebar_visible: f32) -> Div {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let has_repo = state.compare.repo_path.is_some();
    let repo_loaded = state.repository.status == AsyncStatus::Ready;
    let is_ready = state.workspace_mode == WorkspaceMode::Ready;

    let repo_label = state
        .compare
        .repo_path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("diffy");

    let mut left = div()
        .flex_row()
        .flex_shrink_0()
        .min_w(0.0)
        .items_center()
        .gap(Sp::SM);

    if is_ready {
        left = left.child(
            Button::new(Action::ToggleSidebar)
                .icon(lucide::PANEL_LEFT)
                .active(sidebar_visible > 0.5),
        );
    }

    if has_repo {
        left = left.child(ref_selector_button(
            repo_label,
            lucide::FOLDER,
            false,
            Action::OpenRepoPicker,
            tc,
            scale,
        ));
    } else {
        left = left
            .child(svg_icon(lucide::GIT_COMPARE, Ico::LG).color(tc.accent))
            .child(
                div()
                    .min_w(0.0)
                    .child(text("diffy").semibold().color(tc.text_strong).truncate()),
            );
    }

    let mut center = div()
        .flex_1()
        .min_w(0.0)
        .flex_row()
        .items_center()
        .justify_center()
        .gap(Sp::XS);

    if has_repo && repo_loaded {
        let left_label = if state.compare.left_ref.is_empty() {
            "base"
        } else {
            &state.compare.left_ref
        };
        let right_label = if state.compare.right_ref.is_empty() {
            "head"
        } else if state.compare.right_ref == crate::core::vcs::git::service::WORKDIR_REF {
            "working copy"
        } else {
            &state.compare.right_ref
        };

        let mode_symbol = match state.compare.mode {
            CompareMode::SingleCommit => "\u{00b7}",
            CompareMode::TwoDot => "\u{00b7}\u{00b7}",
            CompareMode::ThreeDot => "\u{00b7}\u{00b7}\u{00b7}",
        };

        center = center
            .child(ref_selector_button(
                left_label,
                lucide::GIT_BRANCH,
                state.compare.left_ref.is_empty(),
                Action::OpenRefPicker(CompareField::Left),
                tc,
                scale,
            ))
            .child(
                div()
                    .px(Sp::XS * scale)
                    .py(Sp::XS * scale)
                    .rounded(Rad::MD)
                    .hover_bg(tc.ghost_element_hover)
                    .on_click(Action::CycleCompareMode)
                    .cursor(CursorHint::Pointer)
                    .child(text(mode_symbol).text_sm().medium().color(tc.text_muted)),
            )
            .child(ref_selector_button(
                right_label,
                lucide::GIT_BRANCH,
                state.compare.right_ref.is_empty(),
                Action::OpenRefPicker(CompareField::Right),
                tc,
                scale,
            ));
    } else if state.workspace_mode == WorkspaceMode::Loading {
        center = center.child(text("Comparing\u{2026}").text_sm().color(tc.text_muted));
    }

    let pr_active = state.overlays.top() == Some(OverlaySurface::PullRequestModal);

    let mut right = div()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap_1();

    if is_ready {
        let file_count = state.workspace.files.len();
        right = right.child(
            text(format!("{file_count} files"))
                .text_sm()
                .color(tc.text_muted),
        );
        right = right.child(toolbar_separator(tc));
    }

    right = right.child(
        Button::new(Action::OpenPullRequestModal)
            .icon(lucide::GIT_PULL_REQUEST)
            .label("PR")
            .active(pr_active),
    );

    div()
        .flex_row()
        .items_center()
        .min_w(0.0)
        .h(theme.metrics.title_bar_height)
        .w_full()
        .px(Sp::XL)
        .bg(tc.title_bar_background)
        .border_b(tc.border_variant)
        .child(left)
        .child(center)
        .child(right)
}

fn ref_selector_button(
    label: &str,
    icon: &'static str,
    is_placeholder: bool,
    action: Action,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
) -> Div {
    let text_color = if is_placeholder {
        tc.text_muted
    } else {
        tc.text_strong
    };
    div()
        .flex_row()
        .items_center()
        .gap(Sp::XS * scale)
        .px(Sp::SM * scale)
        .py(Sp::XS * scale)
        .rounded(Rad::MD)
        .hover_bg(tc.ghost_element_hover)
        .on_click(action)
        .cursor(CursorHint::Pointer)
        .min_w(Sz::REF_SELECTOR_MIN_W * scale)
        .child(svg_icon(icon, Ico::SM).color(tc.text_muted))
        .child(
            div()
                .min_w(0.0)
                .flex_1()
                .child(text(label).text_sm().medium().color(text_color).truncate()),
        )
        .child(svg_icon(lucide::CHEVRON_DOWN, Ico::XS).color(tc.text_muted))
}

fn toolbar_separator(tc: &crate::ui::theme::ThemeColors) -> Div {
    div().w(Sz::SEPARATOR_W).h(Sz::SEPARATOR_H).bg(tc.border_variant)
}
