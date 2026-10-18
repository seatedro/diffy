use quark::view;
use quark_app::ViewContext;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::text_input::text_editor_element;

use super::MutationsController;
use super::controls::{self, Chrome, Control, Tone};
use super::presentation::PresentationStyle;
use super::state::working_tree;
use crate::app::shell::Slot;
use crate::app::text_targets::COMMIT_MESSAGE;
use crate::app::tokens::{self, Shadow, Sp, TextStyle};
use crate::contracts::mutations::MutationsAction;
use crate::core::vcs::model::ChangeBucket;

const PANEL_WIDTH: f32 = 420.0;
const EDITOR_MIN_LINES: f32 = 3.0;
const EDITOR_MAX_LINES: f32 = 10.0;
const SUBJECT_LIMIT: usize = 72;
const MENU_WIDTH: f32 = 220.0;

pub fn view(mutations: &mut MutationsController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let theme = cx.theme;
    mutations.sync_style(PresentationStyle::from_theme(theme));
    let publish = working_tree(slot.shared)
        && slot
            .shared
            .repository
            .as_ref()
            .is_some_and(|repo| repo.snapshot.capabilities.remotes);
    let chrome = Chrome::from_theme(theme);
    let control = publish.then(|| {
        controls::render(
            Control::new(
                "mutations.status.publish",
                "Publish",
                MutationsAction::OpenPublishMenu,
                Tone::Ghost,
            )
            .icon(lucide::ARROW_UP),
            &chrome,
        )
    });
    view! {
        <div h={slot.height} class="flex-row items-center shrink-0" test_id="status.changes">
            {?control}
        </div>
    }
}

pub fn commit_box(
    mutations: &mut MutationsController,
    slot: &Slot,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let repo = slot.shared.repository.as_ref()?;
    let capabilities = repo.snapshot.capabilities;
    if !working_tree(slot.shared) || !capabilities.create_commit {
        return None;
    }
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let gap = tokens::scaled(Sp::SM, scale);
    let pad = tokens::scaled(Sp::MD, scale);
    let inset = tokens::scaled(Sp::SM, scale);
    let body = TextStyle::Body.font_size(theme);
    let small = TextStyle::Caption.font_size(theme);
    let chrome = Chrome::from_theme(theme);
    let inner = (slot.width - 2.0 * pad - 2.0 * inset).max(inset);
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
    let message = mutations.commit_text();
    let empty = message.trim().is_empty();
    let subject_len = message
        .lines()
        .next()
        .map_or(0, |line| line.chars().count());
    let generating = mutations.ai().active;
    let amending = mutations.amending().map(|amend| amend.short.clone());
    let menu_open = mutations.commit_menu_open();
    let focused = cx.is_focused(COMMIT_MESSAGE);

    let editor = text_editor_element(
        COMMIT_MESSAGE,
        ScrollActionBuilder::new(|lines| MutationsAction::ScrollCommitMessage(lines).into()),
    )
    .editor_snapshot(mutations.commit_editor())
    .label("Commit message")
    .placeholder("Commit message")
    .focused(focused)
    .font_size(body)
    .text_color(tc.text)
    .w(inner)
    .h(height);

    let banner = amending.clone().map(|short| {
        let stop = controls::render(
            Control::new(
                "mutations.commit.stop-amend",
                "Stop amending",
                MutationsAction::CancelAmend,
                Tone::Ghost,
            )
            .icon_only(lucide::X),
            &Chrome {
                muted: tc.status_warning,
                height: line,
                ..chrome
            },
        );
        view! {
            <div
                class="flex-row items-center"
                gap={inset}
                px={inset}
                rounded={theme.metrics.control_radius}
                bg={tc.status_warning.with_alpha(quark_ui::design::Alpha::TINT)}
                test_id="mutations.commit.amending"
            >
                <icon svg={lucide::CORNER_UP_LEFT} size={chrome.icon} color={tc.status_warning} />
                <text size={chrome.font} color={tc.status_warning}>"Amending"</text>
                <text size={chrome.font} class="font-mono" color={tc.status_warning}>{short}</text>
                <div class="flex-1" />
                {stop}
            </div>
        }
    });

    let counter = (subject_len > SUBJECT_LIMIT).then(|| {
        view! {
            <text size={small} class="font-mono" color={tc.status_warning}>
                {format!("{subject_len}/{SUBJECT_LIMIT}")}
            </text>
        }
    });

    let primary_label = if amending.is_some() {
        "Amend"
    } else {
        "Commit"
    };
    let mut commit_menu: Option<AnyElement> = None;
    let actions = if generating {
        controls::render(
            Control::new(
                "mutations.commit.generating",
                "Generating\u{2026}",
                MutationsAction::CancelGeneration,
                Tone::Secondary,
            )
            .icon(lucide::LOADER)
            .shortcut("esc"),
            &chrome,
        )
    } else if empty && amending.is_none() {
        controls::render(
            Control::new(
                "mutations.commit.generate",
                "Generate",
                MutationsAction::GenerateCommitMessage,
                Tone::Secondary,
            )
            .icon(lucide::SPARKLES)
            .shortcut("\u{2318}G"),
            &chrome,
        )
    } else {
        let commit = controls::render(
            Control::new(
                "mutations.commit.submit",
                primary_label,
                MutationsAction::SubmitCommit,
                Tone::Primary,
            )
            .shortcut("\u{2318}\u{21b5}"),
            &chrome,
        );
        let more = controls::render(
            Control::new(
                "mutations.commit.more",
                "More commit actions",
                MutationsAction::ToggleCommitMenu,
                Tone::Primary,
            )
            .icon_only(lucide::CHEVRON_DOWN),
            &chrome,
        );
        let menu = menu_open.then(|| {
            let mut items = vec![
                Control::new(
                    "mutations.commit.menu.commit",
                    primary_label,
                    MutationsAction::SubmitCommit,
                    Tone::Ghost,
                )
                .shortcut("\u{2318}\u{21b5}"),
            ];
            if capabilities.staging_area && amending.is_none() {
                items.push(
                    Control::new(
                        "mutations.commit.menu.amend",
                        "Amend last commit",
                        MutationsAction::StartAmend,
                        Tone::Ghost,
                    )
                    .shortcut("\u{2325}\u{2318}\u{21b5}"),
                );
            }
            items.push(
                Control::new(
                    "mutations.commit.menu.regenerate",
                    "Regenerate",
                    MutationsAction::GenerateCommitMessage,
                    Tone::Ghost,
                )
                .icon(lucide::SPARKLES)
                .shortcut("\u{2318}G"),
            );
            let rows: Vec<AnyElement> = items
                .into_iter()
                .map(|item| controls::menu_item(item, &chrome))
                .collect();
            let width = tokens::scaled(MENU_WIDTH, scale).min(slot.width - 2.0 * pad);
            let bottom = pad + chrome.height + tokens::scaled(Sp::XS, scale);
            view! {
                <div
                    class="absolute flex-col"
                    right={pad}
                    bottom={bottom}
                    w={width}
                    p={tokens::scaled(Sp::XS, scale)}
                    gap={tokens::scaled(Sp::XXS, scale)}
                    z_index={200}
                    bg={tc.elevated_surface}
                    border={tc.border}
                    rounded={theme.metrics.panel_radius}
                    shadow_preset={Shadow::POPOVER}
                    role="menu"
                    aria-label="Commit actions"
                    test_id="mutations.commit.menu"
                    on:key:escape={MutationsAction::CloseCommitMenu}
                >
                    {...rows}
                </div>
            }
        });
        commit_menu = menu;
        view! {
            <div class="flex-row items-center" gap={tokens::scaled(Sp::XXS, scale)}>
                {commit}
                {more}
            </div>
        }
    };

    let escape = if generating {
        MutationsAction::CancelGeneration
    } else if menu_open {
        MutationsAction::CloseCommitMenu
    } else {
        MutationsAction::CancelAmend
    };
    Some(view! {
        <div
            class="flex-col shrink-0 relative"
            w={slot.width}
            p={pad}
            gap={gap}
            border_t={tc.border_variant}
            bg={tc.sidebar_background}
            accessibility_role={AccessibilityRole::Group}
            aria-label="Commit"
            test_id="mutations.commit"
            on:key:mod+enter={MutationsAction::SubmitCommit}
            on:key:alt+mod+enter={MutationsAction::StartAmend}
            on:key:mod+g={MutationsAction::GenerateCommitMessage}
            on:key:escape={escape}
        >
            <div
                class="flex-col"
                rounded={theme.metrics.panel_radius}
                bg={tc.editor_surface}
                border={if focused { tc.focus_border } else { tc.border }}
            >
                <div class="flex-col" p={inset} gap={inset}>
                    {?banner}
                    {editor}
                </div>
            </div>
            <div class="flex-row items-center" gap={gap}>
                {?counter}
                <div class="flex-1" />
                {actions}
            </div>
            {?commit_menu}
        </div>
    })
}

pub fn selection_bar(
    mutations: &mut MutationsController,
    slot: &Slot,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let first = *mutations.selected_lines().iter().next()?;
    if !working_tree(slot.shared) {
        return None;
    }
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let gap = tokens::scaled(Sp::SM, scale);
    let chrome = Chrome::from_theme(theme);
    let count = mutations.selected_lines().len();
    let entry = slot
        .shared
        .manifest
        .as_ref()
        .and_then(|manifest| manifest.files.iter().find(|entry| entry.key == first.file));
    let name = entry
        .map(|entry| {
            let path = entry.paths.display_path();
            path.rsplit('/').next().unwrap_or(&path).to_owned()
        })
        .unwrap_or_default();
    let buckets = entry.map(|entry| entry.buckets.clone()).unwrap_or_default();
    let mut actions = Vec::new();
    let staged_only = buckets.iter().all(|bucket| *bucket == ChangeBucket::Staged);
    if staged_only {
        actions.push(
            Control::new(
                "mutations.selection.unstage",
                "Unstage lines",
                MutationsAction::UnstageSelectedLines,
                Tone::Primary,
            )
            .shortcut("U"),
        );
    } else {
        if buckets
            .iter()
            .any(|bucket| matches!(bucket, ChangeBucket::Unstaged | ChangeBucket::Untracked))
        {
            actions.push(
                Control::new(
                    "mutations.selection.stage",
                    "Stage lines",
                    MutationsAction::StageSelectedLines,
                    Tone::Primary,
                )
                .shortcut("S"),
            );
        }
        actions.push(
            Control::new(
                "mutations.selection.discard",
                "Discard",
                MutationsAction::DiscardSelectedLines,
                Tone::Destructive,
            )
            .shortcut("X"),
        );
    }
    actions.push(
        Control::new(
            "mutations.selection.clear",
            "Clear selection",
            MutationsAction::ClearLineSelection,
            Tone::Ghost,
        )
        .icon_only(lucide::X),
    );
    let lines = if count == 1 {
        "1 line".to_owned()
    } else {
        format!("{count} lines")
    };
    let offset = tokens::scaled(Sp::XL, scale);
    let width = slot.width;
    Some(view! {
        <div
            class="absolute left-0 flex-row justify-center"
            w={width}
            bottom={offset}
            z_index={140}
        >
            <div
                class="flex-row items-center"
                gap={gap}
                px={tokens::scaled(Sp::MD, scale)}
                py={tokens::scaled(Sp::XS, scale)}
                rounded={theme.metrics.panel_radius}
                bg={tc.elevated_surface}
                border={tc.border}
                shadow_preset={Shadow::POPOVER}
                accessibility_role={AccessibilityRole::Toolbar}
                aria-label="Selected lines"
                test_id="mutations.selection"
                on:key:escape={MutationsAction::ClearLineSelection}
            >
                <text size={chrome.font} class="font-semibold" color={tc.text}>{lines}</text>
                <text size={chrome.font} color={tc.text_muted}>{format!("selected in {name}")}</text>
                {controls::row(actions, &chrome)}
            </div>
        </div>
    })
}

pub fn panel(
    mutations: &mut MutationsController,
    slot: &Slot,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let menu = mutations.publish_menu()?;
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let gap = tokens::scaled(Sp::SM, scale);
    let pad = tokens::scaled(Sp::MD, scale);
    let width = tokens::scaled(PANEL_WIDTH, scale).min((slot.width - 2.0 * gap).max(0.0));
    let body = TextStyle::Body.font_size(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let chrome = Chrome::from_theme(theme);
    let no_remote = menu
        .plan
        .as_ref()
        .filter(|plan| !plan.has_remote())
        .map(|plan| {
            let hint = plan.primary.disabled_reason.clone().unwrap_or_default();
            let description = plan.primary.description.clone();
            view! {
                <div
                    class="flex-row"
                    gap={gap}
                    p={gap}
                    rounded={theme.metrics.control_radius}
                    bg={tc.element_background}
                    accessibility_role={AccessibilityRole::Group}
                    aria-label="No remote"
                    test_id="mutations.publish.no-remote"
                >
                    <icon svg={lucide::INFO} size={chrome.icon} color={tc.text_muted} />
                    <div class="flex-col flex-1 min-w-0" gap={tokens::scaled(Sp::XXS, scale)}>
                        <text size={body} class="font-medium" color={tc.text}>"No remote"</text>
                        <text size={small} color={tc.text_muted}>{description}</text>
                        <text size={small} color={tc.text_muted}>{hint}</text>
                    </div>
                </div>
            }
        });
    let actions: Vec<_> = menu
        .plan
        .iter()
        .filter(|plan| plan.has_remote())
        .flat_map(|plan| std::iter::once(&plan.primary).chain(&plan.alternatives))
        .cloned()
        .collect();
    let loading = menu.loading && actions.is_empty();
    let rows: Vec<AnyElement> = actions
        .into_iter()
        .enumerate()
        .map(|(index, action)| {
            let detail = action
                .disabled_reason
                .clone()
                .unwrap_or_else(|| action.description.clone());
            let enabled = action.is_enabled();
            let control = Control::new(
                format!("mutations.publish.{index}"),
                action.label.clone(),
                MutationsAction::Publish(Box::new(action)),
                if index == 0 && enabled {
                    Tone::Primary
                } else {
                    Tone::Secondary
                },
            )
            .icon(lucide::ARROW_UP)
            .disabled(!enabled);
            view! {
                <div class="flex-col" gap={tokens::scaled(Sp::XXS, scale)}>
                    {controls::render(control, &chrome)}
                    <text size={small} color={tc.text_muted}>{detail}</text>
                </div>
            }
        })
        .collect();
    let close = controls::render(
        Control::new(
            "mutations.publish.close",
            "Close publish menu",
            MutationsAction::ClosePublishMenu,
            Tone::Ghost,
        )
        .icon_only(lucide::X),
        &chrome,
    );
    let bottom = theme.metrics.status_bar_height + gap;
    Some(view! {
        <div
            class="absolute flex-col"
            right={gap}
            bottom={bottom}
            w={width}
            p={pad}
            gap={gap}
            z_index={150}
            bg={tc.elevated_surface}
            border={tc.border}
            rounded={theme.metrics.panel_radius}
            shadow_preset={Shadow::POPOVER}
            accessibility_role={AccessibilityRole::Dialog}
            aria-label="Publish"
            test_id="mutations.publish"
            on:key:escape={MutationsAction::ClosePublishMenu}
        >
            <div class="flex-row items-center">
                <text size={body} class="font-semibold" color={tc.text_strong}>"Publish"</text>
                <div class="flex-1" />
                {close}
            </div>
            if loading {
                <text size={small} color={tc.text_muted}>"Loading publish options\u{2026}"</text>
            }
            {?no_remote}
            {...rows}
        </div>
    })
}
