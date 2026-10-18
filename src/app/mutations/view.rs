use quark::view;
use quark_app::ViewContext;
use quark_components::{Button, ButtonSize, ButtonStyle};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::text_input::text_editor_element;

use super::MutationsController;
use super::presentation::PresentationStyle;
use crate::app::shell::Slot;
use crate::app::text_targets::COMMIT_MESSAGE;
use crate::app::tokens::{self, Shadow, Sp, TextStyle};
use crate::contracts::mutations::MutationsAction;
use crate::contracts::navigation::RepositoryContext;
use crate::core::vcs::model::ChangeBucket;

const PANEL_WIDTH: f32 = 420.0;
const EDITOR_MIN_LINES: f32 = 3.0;
const EDITOR_MAX_LINES: f32 = 10.0;

fn summary(repo: Option<&RepositoryContext>) -> String {
    let Some(repo) = repo else {
        return "No repository".to_owned();
    };
    let changes = &repo.snapshot.file_changes;
    if changes.is_empty() {
        return "No pending changes".to_owned();
    }
    if !repo.snapshot.capabilities.staging_area {
        return format!("{} changed", changes.len());
    }
    let staged = changes
        .iter()
        .filter(|change| change.bucket == ChangeBucket::Staged)
        .count();
    let unstaged = changes.len() - staged;
    match (staged, unstaged) {
        (0, n) => format!("{n} unstaged"),
        (n, 0) => format!("{n} staged"),
        (s, u) => format!("{s} staged \u{00b7} {u} unstaged"),
    }
}

fn button(label: &str, action: MutationsAction) -> Button {
    Button::new(action).label(label).size(ButtonSize::Compact)
}

pub fn view(mutations: &mut MutationsController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let theme = cx.theme;
    let scale = tokens::ui_scale(theme);
    mutations.sync_style(PresentationStyle::from_theme(theme));
    let repo = slot.shared.repository.as_deref();
    let label = summary(repo);
    let capabilities = repo.map(|repo| repo.snapshot.capabilities);
    let commit = capabilities.is_some_and(|c| c.create_commit);
    let publish = capabilities.is_some_and(|c| c.remotes);
    view! {
        <div
            h={slot.height}
            class="flex-row items-center shrink-0"
            gap={tokens::scaled(Sp::SM, scale)}
            test_id="status.changes"
        >
            <text size={TextStyle::BodySmall.font_size(theme)} color={theme.colors.text_muted}>
                {label}
            </text>
            if commit {
                <div>{button("Commit", MutationsAction::OpenCommitComposer)}</div>
            }
            if publish {
                <div>{button("Publish", MutationsAction::OpenPublishMenu)}</div>
            }
        </div>
    }
}

pub fn panel(
    mutations: &mut MutationsController,
    slot: &Slot,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let selected = mutations.selected_lines().len();
    let composing = mutations.composer_open();
    let publishing = mutations.publish_menu().is_some();
    if selected == 0 && !composing && !publishing {
        return None;
    }
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let gap = tokens::scaled(Sp::SM, scale);
    let pad = tokens::scaled(Sp::MD, scale);
    let width = tokens::scaled(PANEL_WIDTH, scale).min((slot.width - 2.0 * gap).max(0.0));
    let inner = (width - 2.0 * pad).max(pad);
    let staging = slot
        .shared
        .repository
        .as_ref()
        .is_some_and(|repo| repo.snapshot.capabilities.staging_area);
    let body = TextStyle::Body.font_size(theme);
    let small = TextStyle::BodySmall.font_size(theme);

    let selection = (selected > 0).then(|| {
        let label = if selected == 1 {
            "1 line selected".to_owned()
        } else {
            format!("{selected} lines selected")
        };
        view! {
            <div class="flex-col" gap={gap} test_id="mutations.selection">
                <text size={small} color={tc.text_muted}>{label}</text>
                <div class="flex-row items-center flex-wrap" gap={gap}>
                    if staging {
                        <div>{button("Stage selected lines", MutationsAction::StageSelectedLines)}</div>
                        <div>{button("Unstage selected lines", MutationsAction::UnstageSelectedLines)}</div>
                    }
                    <div>{button("Discard selected lines", MutationsAction::DiscardSelectedLines).style(ButtonStyle::Danger)}</div>
                    <div>{button("Clear selection", MutationsAction::ClearLineSelection)}</div>
                </div>
            </div>
        }
    });

    let composer = composing.then(|| {
        let line = body * 1.35;
        let now = cx.frame.elapsed().as_millis() as u64;
        let editor = mutations.commit_editor_mut();
        editor.set_font_size(body);
        editor.set_clock(now);
        let height = editor.flush_fit(
            &mut cx.frame.text().system,
            inner,
            line * EDITOR_MIN_LINES,
            line * EDITOR_MAX_LINES,
        );
        let ai = mutations.ai();
        let status = if ai.active {
            Some("Generating\u{2026}".to_owned())
        } else {
            ai.error.clone()
        };
        let generating = ai.active;
        let focused = cx.is_focused(COMMIT_MESSAGE);
        let editor = text_editor_element(
            COMMIT_MESSAGE,
            ScrollActionBuilder::new(|lines| MutationsAction::ScrollCommitMessage(lines).into()),
        )
        .editor_snapshot(mutations.commit_editor())
        .label("Commit message")
        .placeholder("Describe the change")
        .focused(focused)
        .font_size(body)
        .text_color(tc.text)
        .w(inner)
        .h(height);
        view! {
            <div class="flex-col" gap={gap} test_id="mutations.composer">
                <text size={body} class="font-semibold" color={tc.text_strong}>"Commit"</text>
                <div
                    class="rounded-md"
                    bg={tc.editor_surface}
                    border={if focused { tc.focus_border } else { tc.border }}
                >
                    {editor}
                </div>
                {?status.map(|status| view! {
                    <text size={small} color={tc.text_muted}>{status}</text>
                })}
                <div class="flex-row items-center" gap={gap}>
                    <div>{button("Generate commit message", MutationsAction::GenerateCommitMessage).disabled(generating)}</div>
                    <div class="flex-1"></div>
                    <div>{button("Cancel", MutationsAction::CloseCommitComposer)}</div>
                    <div>{button("Create commit", MutationsAction::SubmitCommit).style(ButtonStyle::Filled)}</div>
                </div>
            </div>
        }
    });

    let publish = mutations.publish_menu().map(|menu| {
        let actions: Vec<_> = menu
            .plan
            .iter()
            .flat_map(|plan| std::iter::once(&plan.primary).chain(&plan.alternatives))
            .cloned()
            .collect();
        let loading = menu.loading && actions.is_empty();
        view! {
            <div class="flex-col" gap={gap} test_id="mutations.publish" role="menu" aria-label="Publish">
                <text size={body} class="font-semibold" color={tc.text_strong}>"Publish"</text>
                if loading {
                    <text size={small} color={tc.text_muted}>"Loading publish options\u{2026}"</text>
                }
                for action in actions key={action.label.clone()} {
                    <div class="flex-col">
                        {Button::new(MutationsAction::Publish(Box::new(action.clone())))
                            .label(action.label.clone())
                            .disabled(!action.is_enabled())}
                        <text size={small} color={tc.text_muted}>
                            {action.disabled_reason.clone().unwrap_or_else(|| action.description.clone())}
                        </text>
                    </div>
                }
                <div class="flex-row">
                    <div class="flex-1"></div>
                    <div>{button("Close", MutationsAction::ClosePublishMenu)}</div>
                </div>
            </div>
        }
    });

    let bottom = theme.metrics.status_bar_height + gap;
    Some(view! {
        <div
            class="absolute flex-col"
            right={gap}
            bottom={bottom}
            w={width}
            p={pad}
            gap={pad}
            z_index={150}
            bg={tc.elevated_surface}
            border={tc.border}
            rounded={theme.metrics.panel_radius * scale}
            shadow_preset={Shadow::POPOVER}
            accessibility_role={AccessibilityRole::Dialog}
            aria-label="Changes"
            test_id="mutations.panel"
        >
            {?selection}
            {?composer}
            {?publish}
        </div>
    })
}
