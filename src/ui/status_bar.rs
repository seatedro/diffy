use crate::actions::Action;
use crate::core::compare::{CompareMode, RendererKind};
use crate::ui::design::{Ico, Rad, Sp};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
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

    let head_branch_info = state.repository.branches.with(&state.store, |branches| {
        branches
            .iter()
            .find(|b| b.is_head)
            .map(|b| (b.name.clone(), b.upstream.clone(), b.ahead_behind))
    });

    let branch_children = head_branch_info.map(|(branch, upstream, ahead_behind)| {
        let sync_chip = ahead_behind.and_then(|counts| {
            let remote = upstream
                .as_deref()
                .and_then(|u| u.split_once('/').map(|(r, _)| r.to_owned()));
            remote.map(|remote| sync_chip(tc, scale, counts, remote))
        });
        view! { scale,
            <div class="flex-row items-center" gap={Sp::SM}>
                <text class="text-xs" color={tc.text_muted}>{"\u{00b7}"}</text>
                <icon svg={lucide::GIT_BRANCH} size={Ico::XS} color={tc.text_muted} />
                <text class="text-xs truncate" color={tc.text_muted}>{branch}</text>
                {?sync_chip}
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

/// Clickable ahead/behind indicator next to the branch name. Colors the
/// ahead/behind halves independently and dispatches the "obvious" action on
/// click:
/// - ahead only → push
/// - behind only → fast-forward pull
/// - both zero or both non-zero → fetch (safe refresh)
fn sync_chip(
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
    counts: (usize, usize),
    remote: String,
) -> AnyElement {
    let (ahead, behind) = counts;

    let action = match (ahead, behind) {
        (a, 0) if a > 0 => Action::PushCurrentBranch {
            force_with_lease: false,
        },
        (0, b) if b > 0 => Action::PullCurrentBranch,
        _ => Action::FetchRemote(remote),
    };

    // Halves brighten when their count is non-zero. Using text_strong / text_muted
    // keeps the chip on-theme instead of borrowing diff-body colors.
    let ahead_color = if ahead > 0 { tc.text_strong } else { tc.text_muted };
    let behind_color = if behind > 0 { tc.text_strong } else { tc.text_muted };

    view! { scale,
        <div class="flex-row items-center"
            gap={Sp::XS}
            px={Sp::SM}
            py={2.0}
            rounded={Rad::SM}
            hover_bg={tc.ghost_element_hover}
            cursor={CursorHint::Pointer}
            on_click={action}
        >
            <icon svg={lucide::ARROW_UP} size={Ico::XS} color={ahead_color} />
            <text class="text-xs" color={ahead_color}>{ahead.to_string()}</text>
            <icon svg={lucide::ARROW_DOWN} size={Ico::XS} color={behind_color} />
            <text class="text-xs" color={behind_color}>{behind.to_string()}</text>
        </div>
    }
    .into_any()
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
