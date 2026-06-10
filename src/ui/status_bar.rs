use crate::core::compare::RendererKind;
use crate::core::review::ReviewSessionStatus;
use crate::core::vcs::model::RefKind;
use crate::ui::design::{Alpha, Ico, Rad, Sp};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{ActiveReviewStatus, AppState, AsyncStatus, WorkspaceSource};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;
use crate::ui::vcs::{
    PublishHintUi, PublishRefChipUi, RepositoryIdentityLabelStyle, RepositoryIdentityUi,
};
use halogen::view;

pub(crate) fn status_bar(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let workspace_source = state.workspace.source.get(&state.store);
    let status = if workspace_source == WorkspaceSource::TextCompare {
        state.text_compare.status
    } else {
        state.repository.status.get(&state.store)
    };
    let (status_icon, status_color, status_text) = match status {
        AsyncStatus::Ready => (lucide::CHECK, tc.line_add_text, "ready"),
        AsyncStatus::Loading => (lucide::LOADER, tc.text_muted, "loading"),
        AsyncStatus::Failed => (lucide::ALERT_CIRCLE, tc.status_error, "error"),
        AsyncStatus::Idle => (lucide::INFO, tc.text_muted, "idle"),
    };

    let profile = state.repository.location.with(&state.store, |location| {
        crate::ui::vcs::profile(location.as_ref())
    });
    let refs = state
        .repository
        .refs
        .with(&state.store, |refs| refs.clone());
    let changes = state
        .repository
        .changes
        .with(&state.store, |changes| changes.clone());
    let head_branch_info = refs
        .iter()
        .find(|reference| reference.active && reference.kind == RefKind::Branch)
        .map(|reference| {
            (
                reference.name.clone(),
                reference.upstream.clone(),
                reference.ahead_behind,
            )
        });
    let vcs_identity = profile.repository_identity_from_changes(&changes);
    let publish_status = state.repository.publish_plan.with(&state.store, |plan| {
        profile.publish_status_ui(&changes, &refs, plan.as_ref())
    });

    let branch_children = if workspace_source == WorkspaceSource::TextCompare {
        None
    } else if let Some(identity) = vcs_identity {
        let icon_color = repository_identity_icon_color(&identity, tc);
        let label = repository_identity_label(&identity, tc);
        let ref_chips = publish_ref_chips(&publish_status.ref_chips, tc, scale);
        let remote_button = publish_status
            .show_menu
            .then(|| remote_menu_button(publish_status.hint, tc, scale));
        Some(view! { scale,
            <div class="flex-row items-center overflow-hidden" gap={Sp::XS} min_w={0.0}>
                <text class="text-xs" color={tc.text_muted}>{"\u{00b7}"}</text>
                <icon svg={identity.icon} size={Ico::XS} color={icon_color} />
                {label}
                {?ref_chips}
                {?remote_button}
            </div>
        })
    } else {
        head_branch_info.map(|(branch, upstream, ahead_behind)| {
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
        })
    };

    let hunk_child = state.editor_current_hunk_index().map(|(idx, total)| {
        view! {
            <text class="text-xs" color={tc.text_muted}>{format!("Hunk {}/{}", idx + 1, total)}</text>
        }
    });
    let review_child = state
        .active_pr_review_status()
        .map(|summary| review_status(summary, theme, scale));
    let syntax_pack_child = state
        .ui
        .syntax_pack_installs
        .with(&state.store, |active| !active.is_empty())
        .then(|| syntax_pack_status(state.clock_ms, theme, scale));

    let right_text = match workspace_source {
        WorkspaceSource::Status => {
            profile.status_view_label(state.workspace.selected_change_bucket.get(&state.store))
        }
        WorkspaceSource::TextCompare => format!(
            "Text Compare  \u{00b7}  {}",
            renderer_label(state.compare.renderer.get(&state.store)),
        ),
        _ => format!(
            "{}  \u{00b7}  {}",
            profile
                .compare_mode_ui(state.compare.mode.get(&state.store))
                .label,
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
            <div class="flex-row shrink-0 items-center" gap={Sp::MD}>
                {?review_child}
                {?hunk_child}
                <text class="text-xs" color={tc.text_muted}>{right_text}</text>
                {?syntax_pack_child}
            </div>
        </div>
    }
}

fn review_status(summary: ActiveReviewStatus, theme: &Theme, scale: f32) -> AnyElement {
    let tc = &theme.colors;
    let (icon, color) = match summary.status {
        ReviewSessionStatus::Loading => (lucide::LOADER, tc.text_muted),
        ReviewSessionStatus::Failed => (lucide::ALERT_CIRCLE, tc.status_error),
        ReviewSessionStatus::Idle => (lucide::GIT_PULL_REQUEST, tc.text_muted),
        ReviewSessionStatus::Ready => {
            if summary.unresolved_threads > 0 || summary.failed_drafts > 0 {
                (lucide::GIT_PULL_REQUEST, tc.status_warning)
            } else {
                (lucide::GIT_PULL_REQUEST, tc.line_add_text)
            }
        }
    };
    let label = review_status_label(&summary);
    view! { scale,
        <div class="flex-row items-center overflow-hidden" gap={Sp::XS} min_w={0.0}>
            <icon svg={icon} size={Ico::XS} color={color} />
            <text class="text-xs truncate" color={tc.text_muted}>{label}</text>
        </div>
    }
    .into_any()
}

fn review_status_label(summary: &ActiveReviewStatus) -> String {
    match summary.status {
        ReviewSessionStatus::Loading => "reviews loading".to_owned(),
        ReviewSessionStatus::Failed => summary
            .message
            .as_deref()
            .filter(|message| !message.trim().is_empty())
            .map(|message| format!("reviews failed: {message}"))
            .unwrap_or_else(|| "reviews failed".to_owned()),
        ReviewSessionStatus::Idle => "reviews idle".to_owned(),
        ReviewSessionStatus::Ready => {
            let mut parts = Vec::with_capacity(4);
            if summary.unresolved_threads > 0 {
                parts.push(count_label(summary.unresolved_threads, "unresolved"));
            } else if summary.resolved_threads > 0 {
                parts.push(count_label(summary.resolved_threads, "resolved"));
            } else {
                parts.push("no review threads".to_owned());
            }

            if summary.pending_drafts > 0 {
                parts.push(count_noun(summary.pending_drafts, "draft", "drafts"));
            }
            if summary.failed_drafts > 0 {
                parts.push(count_noun(
                    summary.failed_drafts,
                    "failed draft",
                    "failed drafts",
                ));
            }
            if summary.outdated_threads > 0 && parts.len() < 3 {
                parts.push(count_label(summary.outdated_threads, "outdated"));
            }
            if parts.len() < 3
                && let Some(label) = summary
                    .review_decision
                    .as_deref()
                    .or(summary.viewer_latest_review_state.as_deref())
                    .and_then(review_decision_label)
            {
                parts.push(label.to_owned());
            }
            parts.join(" / ")
        }
    }
}

fn review_decision_label(value: &str) -> Option<&'static str> {
    match value {
        "APPROVED" => Some("approved"),
        "CHANGES_REQUESTED" => Some("changes requested"),
        "REVIEW_REQUIRED" => Some("review required"),
        "COMMENTED" => Some("commented"),
        "PENDING" => Some("pending"),
        _ => None,
    }
}

fn count_label(count: usize, noun: &str) -> String {
    format!("{count} {noun}")
}

fn count_noun(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

fn repository_identity_icon_color(
    identity: &RepositoryIdentityUi,
    tc: &crate::ui::theme::ThemeColors,
) -> crate::ui::theme::Color {
    match identity.label_style {
        RepositoryIdentityLabelStyle::Plain => tc.text_muted,
        RepositoryIdentityLabelStyle::ChangeId { .. } => {
            tc.syntax_keyword.lerp(tc.text_strong, 0.28)
        }
    }
}

fn repository_identity_label(
    identity: &RepositoryIdentityUi,
    tc: &crate::ui::theme::ThemeColors,
) -> AnyElement {
    match identity.label_style {
        RepositoryIdentityLabelStyle::Plain => text(identity.label.clone())
            .text_xs()
            .truncate()
            .color(tc.text_muted)
            .into_any(),
        RepositoryIdentityLabelStyle::ChangeId {
            change_id_prefix_len,
        } => change_id_identity_label(&identity.label, change_id_prefix_len, tc),
    }
}

fn change_id_identity_label(
    label: &str,
    change_id_prefix_len: usize,
    tc: &crate::ui::theme::ThemeColors,
) -> AnyElement {
    let mut parts = label.splitn(3, ' ');
    let Some(marker) = parts.next() else {
        return text(label.to_owned())
            .text_xs()
            .truncate()
            .color(tc.text_muted)
            .into_any();
    };
    let Some(change_id) = parts.next() else {
        return text(label.to_owned())
            .text_xs()
            .truncate()
            .color(tc.text_muted)
            .into_any();
    };
    let Some(revision) = parts.next() else {
        return text(label.to_owned())
            .text_xs()
            .truncate()
            .color(tc.text_muted)
            .into_any();
    };

    let split = change_id_prefix_len.min(change_id.len());
    let split = if change_id.is_char_boundary(split) {
        split
    } else {
        0
    };
    let (prefix, rest) = change_id.split_at(split);
    let prefix_color = tc.syntax_keyword.lerp(tc.text_strong, 0.28);

    view! {
        <div class="flex-row items-center overflow-hidden" min_w={0.0}>
            {text(marker.to_owned()).text_xs().color(tc.text_muted)}
            <text class="text-xs" color={tc.text_muted}>{" "}</text>
            {text(prefix.to_owned()).text_xs().bold().color(prefix_color)}
            {text(rest.to_owned()).text_xs().color(tc.text_muted)}
            <text class="text-xs" color={tc.text_muted}>{" "}</text>
            {text(revision.to_owned()).text_xs().truncate().color(tc.syntax_type)}
        </div>
    }
}

fn syntax_pack_status(clock_ms: u64, theme: &Theme, scale: f32) -> AnyElement {
    let tc = &theme.colors;
    view! { scale,
        <div class="flex-row shrink-0 items-center"
             gap={Sp::XS}
             px={Sp::SM}
             py={2.0}>
            {circular_loader(clock_ms, tc.accent)}
            <text class="text-xs" color={tc.text_muted}>{"Installing Tree-sitter languages"}</text>
        </div>
    }
    .into_any()
}

fn circular_loader(clock_ms: u64, color: crate::ui::theme::Color) -> AnyElement {
    let size = Ico::SM;
    let dot = 2.5;
    let positions = [
        (5.75, 0.0),
        (9.75, 1.5),
        (11.5, 5.75),
        (9.75, 9.75),
        (5.75, 11.5),
        (1.5, 9.75),
        (0.0, 5.75),
        (1.5, 1.5),
    ];
    let dot_count = positions.len();
    let head = ((clock_ms / 100) % dot_count as u64) as usize;
    let alphas = [
        Alpha::STRONG,
        Alpha::PLACEHOLDER,
        Alpha::MEDIUM,
        Alpha::MUTED,
        Alpha::SOFT,
        Alpha::DIM,
        Alpha::FAINT,
        Alpha::TINT,
    ];

    let mut loader = div().relative().w(size).h(size).flex_shrink_0();
    for (index, (left, top)) in positions.iter().copied().enumerate() {
        let age = (index + dot_count - head) % dot_count;
        loader = loader.child(
            div()
                .absolute()
                .left(left)
                .top(top)
                .w(dot)
                .h(dot)
                .rounded_full()
                .bg(color.with_alpha(alphas[age])),
        );
    }
    loader.into_any()
}

fn remote_menu_button(
    hint: Option<PublishHintUi>,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
) -> AnyElement {
    let tooltip = match hint.as_ref() {
        Some(h) => h.tooltip.clone(),
        None => "Publish, fetch, or pull".to_owned(),
    };
    let label = match hint.as_ref() {
        Some(h) => render_publish_label(h, tc),
        None => text("Push".to_owned())
            .text_xs()
            .medium()
            .color(tc.text)
            .into_any(),
    };

    view! { scale,
        <div class="flex-row shrink-0 items-center"
            gap={Sp::XS}
            px={Sp::SM}
            py={2.0}
            rounded={Rad::SM}
            hover_bg={tc.ghost_element_hover}
            cursor={CursorHint::Pointer}
            tooltip={tooltip}
            on_click={crate::actions::RepositoryAction::OpenPublishMenu.into()}
        >
            <icon svg={lucide::ARROW_UP} size={Ico::XS} color={tc.text_muted} />
            {label}
            <icon svg={lucide::CHEVRON_DOWN} size={Ico::XS} color={tc.text_muted} />
        </div>
    }
    .into_any()
}

fn render_publish_label(hint: &PublishHintUi, tc: &crate::ui::theme::ThemeColors) -> AnyElement {
    // Inter-span spacing comes from a trailing space on "Push " — flex `gap`
    // between text elements doesn't render reliably across runs in halogen,
    // so we use explicit whitespace the same way the identity label does.
    let push_word = text("Push ".to_owned())
        .text_xs()
        .medium()
        .color(tc.text)
        .into_any();
    let Some(token) = hint.change_id_token.as_ref() else {
        return view! {
            <div class="flex-row items-center">
                {push_word}
                {text(hint.label.clone()).text_xs().medium().color(tc.text)}
            </div>
        }
        .into_any();
    };
    let split = token.prefix_len.min(token.text.len()).max(1);
    let split = if token.text.is_char_boundary(split) {
        split
    } else {
        0
    };
    let (prefix, rest) = token.text.split_at(split);
    let prefix_color = tc.syntax_keyword.lerp(tc.text_strong, 0.28);

    view! {
        <div class="flex-row items-center">
            {push_word}
            {text(prefix.to_owned()).text_xs().bold().color(prefix_color)}
            {text(rest.to_owned()).text_xs().color(tc.text_muted)}
        </div>
    }
    .into_any()
}

fn publish_ref_chips(
    chips: &[PublishRefChipUi],
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
) -> Option<AnyElement> {
    if chips.is_empty() {
        return None;
    }
    let mut rows: Vec<AnyElement> = Vec::with_capacity(chips.len());
    for chip in chips.iter().take(3) {
        let dot_color = if chip.tracked {
            tc.line_add_text
        } else {
            tc.status_warning
        };
        let row = view! { scale,
            <div class="flex-row items-center shrink-0"
                 gap={Sp::XS}
                 px={Sp::XS + Sp::XXS}
                 py={1.0}
                 rounded={Rad::SM}
                 bg={tc.ghost_element_hover}>
                <div class="shrink-0" w={6.0} h={6.0} rounded={3.0} bg={dot_color} />
                <text class="text-xs truncate" color={tc.text}>{chip.name.clone()}</text>
            </div>
        }
        .into_any();
        rows.push(row);
    }

    Some(
        view! { scale,
            <div class="flex-row items-center shrink-0" gap={Sp::XS}>
                {...rows}
            </div>
        }
        .into_any(),
    )
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
        (a, 0) if a > 0 => crate::actions::RepositoryAction::PushCurrentBranch {
            force_with_lease: false,
        }
        .into(),
        (0, b) if b > 0 => crate::actions::RepositoryAction::PullCurrentBranch.into(),
        _ => crate::actions::RepositoryAction::FetchRemote(remote).into(),
    };

    // Halves brighten when their count is non-zero. Using text_strong / text_muted
    // keeps the chip on-theme instead of borrowing diff-body colors.
    let ahead_color = if ahead > 0 {
        tc.text_strong
    } else {
        tc.text_muted
    };
    let behind_color = if behind > 0 {
        tc.text_strong
    } else {
        tc.text_muted
    };

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

pub(crate) fn renderer_label(renderer: RendererKind) -> &'static str {
    match renderer {
        RendererKind::Builtin => "built-in",
        RendererKind::Difftastic => "difftastic",
    }
}

const PULL_REQUEST_REF_PREFIX: &str = "refs/diffy/pr/";

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn display_ref(value: &str) -> &str {
    if value.is_empty() {
        return "?";
    }
    if let Some(rest) = value.strip_prefix(PULL_REQUEST_REF_PREFIX)
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
