use crate::core::compare::{CompareMode, RendererKind};
use crate::ui::design::{Ico, Sp};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::{AppState, AsyncStatus};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

pub(crate) fn status_bar(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let (status_icon, status_color, status_text) = match state.repository.status {
        AsyncStatus::Ready => (lucide::CHECK, tc.line_add_text, "ready"),
        AsyncStatus::Loading => (lucide::LOADER, tc.text_muted, "loading"),
        AsyncStatus::Failed => (lucide::ALERT_CIRCLE, tc.status_error, "error"),
        AsyncStatus::Idle => (lucide::INFO, tc.text_muted, "idle"),
    };

    let head_branch = state
        .repository
        .branches
        .iter()
        .find(|b| b.is_head)
        .map(|b| b.name.as_str());

    let mut left = div()
        .flex_row()
        .items_center()
        .gap(Sp::SM)
        .min_w(0.0)
        .child(svg_icon(status_icon, Ico::XS).color(status_color))
        .child(text(status_text).text_xs().color(tc.text_muted));

    if let Some(branch) = head_branch {
        left = left
            .child(text("\u{00b7}").text_xs().color(tc.text_muted))
            .child(svg_icon(lucide::GIT_BRANCH, Ico::XS).color(tc.text_muted))
            .child(text(branch).text_xs().color(tc.text_muted).truncate());
    }

    let mut center = div().flex_row().items_center().gap(Sp::SM);
    if let Some((idx, total)) = state.editor.current_hunk_index() {
        center = center.child(
            text(format!("Hunk {}/{}", idx + 1, total))
                .text_xs()
                .color(tc.text_muted),
        );
    }

    let right_text = format!(
        "{}  \u{00b7}  {}",
        compare_mode_label(state.compare.mode),
        renderer_label(state.compare.renderer),
    );

    div()
        .flex_row()
        .items_center()
        .h(theme.metrics.status_bar_height)
        .w_full()
        .px_4()
        .bg(tc.status_bar_background)
        .border_t(tc.border_variant)
        .child(left)
        .child(spacer())
        .child(center)
        .child(spacer())
        .child(text(right_text).text_xs().color(tc.text_muted))
}

pub(crate) fn compare_mode_label(mode: CompareMode) -> &'static str {
    match mode {
        CompareMode::SingleCommit => "single-commit",
        CompareMode::TwoDot => "two-dot",
        CompareMode::ThreeDot => "three-dot",
    }
}

pub(crate) fn renderer_label(renderer: RendererKind) -> &'static str {
    match renderer {
        RendererKind::Builtin => "built-in",
        RendererKind::Difftastic => "difftastic",
    }
}

pub(crate) fn display_ref(value: &str) -> &str {
    if value.is_empty() { "?" } else { value }
}
