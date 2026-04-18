use crate::core::compare::{CompareMode, RendererKind};
use crate::ui::design::{Ico, Sp};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::WorkspaceSource;
use crate::ui::state::{AppState, AsyncStatus};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;
use halogen::view;

pub(crate) fn status_bar(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let (status_icon, status_color, status_text) = match state.repository.status.get(&state.store) {
        AsyncStatus::Ready => (lucide::CHECK, tc.line_add_text, "ready"),
        AsyncStatus::Loading => (lucide::LOADER, tc.text_muted, "loading"),
        AsyncStatus::Failed => (lucide::ALERT_CIRCLE, tc.status_error, "error"),
        AsyncStatus::Idle => (lucide::INFO, tc.text_muted, "idle"),
    };

    let head_branch = state.repository.branches.with(&state.store, |branches| {
        branches.iter().find(|b| b.is_head).map(|b| b.name.clone())
    });

    let branch_children = head_branch.map(|branch| {
        view! { scale,
            <div class="flex-row items-center" gap={Sp::SM}>
                <text class="text-xs" color={tc.text_muted}>{"\u{00b7}"}</text>
                <icon svg={lucide::GIT_BRANCH} size={Ico::XS} color={tc.text_muted} />
                <text class="text-xs truncate" color={tc.text_muted}>{branch}</text>
            </div>
        }
    });

    let hunk_child = state.editor_current_hunk_index().map(|(idx, total)| {
        view! {
            <text class="text-xs" color={tc.text_muted}>{format!("Hunk {}/{}", idx + 1, total)}</text>
        }
    });

    let right_text = match state.workspace.source.get(&state.store) {
        WorkspaceSource::Status => state
            .workspace
            .selected_status_scope
            .get(&state.store)
            .map(|scope| format!("working tree  \u{00b7}  {}", scope.label()))
            .unwrap_or_else(|| "working tree".to_owned()),
        _ => format!(
            "{}  \u{00b7}  {}",
            compare_mode_label(state.compare.mode.get(&state.store)),
            renderer_label(state.compare.renderer.get(&state.store)),
        ),
    };

    view! { scale,
        <div class="flex-row items-center w-full"
             h={theme.metrics.status_bar_height}
             px={Sp::LG}
             bg={tc.status_bar_background}
             border_t={tc.border_variant}>
            <div class="flex-row items-center" gap={Sp::SM} min_w={0.0}>
                <icon svg={status_icon} size={Ico::XS} color={status_color} />
                <text class="text-xs" color={tc.text_muted}>{status_text}</text>
                {?branch_children}
            </div>
            <spacer />
            <div class="flex-row items-center" gap={Sp::SM}>
                {?hunk_child}
            </div>
            <spacer />
            <text class="text-xs" color={tc.text_muted}>{right_text}</text>
        </div>
    }
}

pub(crate) fn compare_mode_label(mode: CompareMode) -> &'static str {
    match mode {
        CompareMode::SingleCommit => "commit",
        CompareMode::TwoDot => "diff",
        CompareMode::ThreeDot => "merge",
    }
}

pub(crate) fn renderer_label(renderer: RendererKind) -> &'static str {
    match renderer {
        RendererKind::Builtin => "built-in",
        RendererKind::Difftastic => "difftastic",
    }
}

pub(crate) fn display_ref(value: &str) -> &str {
    if value.is_empty() {
        return "?";
    }
    if let Some(rest) = value.strip_prefix(crate::core::vcs::git::service::PR_REF_PREFIX)
        && let Some(idx) = rest.find('/')
    {
        return &rest[idx + 1..];
    }
    value
}

#[cfg(test)]
mod tests {
    use super::display_ref;

    #[test]
    fn strips_pr_prefix_and_number_leaving_branch() {
        assert_eq!(display_ref("refs/diffy/pr/12/main"), "main");
    }

    #[test]
    fn preserves_slashes_in_branch_names() {
        assert_eq!(
            display_ref("refs/diffy/pr/77/feat/new-thing"),
            "feat/new-thing"
        );
    }

    #[test]
    fn passes_through_non_pr_refs() {
        assert_eq!(display_ref("main"), "main");
        assert_eq!(display_ref("refs/heads/main"), "refs/heads/main");
    }

    #[test]
    fn empty_renders_placeholder() {
        assert_eq!(display_ref(""), "?");
    }
}
