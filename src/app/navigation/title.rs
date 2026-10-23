use std::path::Path;

use quark::view;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::NavigationController;
use crate::app::commands::repo_picker::tilde;
use crate::app::icons;
use crate::app::tokens::{self, Ico, Layout, Sp, TextStyle};
use crate::contracts::commands::CommandsAction;
use crate::contracts::navigation::{CompareField, NavigationAction};
use crate::core::ref_catalog::WORKING_TREE_LABEL;
use crate::core::vcs::model::RefKind;

pub fn repo_chip(nav: &NavigationController, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let (name, worktree) = repo_names(nav);
    let label = match &worktree {
        Some(worktree) => format!("Repository {name}, worktree {worktree}"),
        None => format!("Repository {name}"),
    };
    let home = dirs::home_dir();
    let path = nav
        .repo_path()
        .map(|path| tilde(path, home.as_deref()))
        .unwrap_or_default();
    view! {
        <div
            class="flex-row items-center shrink-0"
            h={tokens::scaled(Layout::COMPARE_CHIP_H, scale)}
            px={tokens::scaled(Sp::SM, scale)}
            gap={tokens::scaled(Sp::XS, scale)}
            rounded={theme.metrics.control_radius}
            hover_bg={tc.ghost_element_hover}
            cursor={CursorHint::Pointer}
            id="title.repo"
            test_id="title.repo"
            role="button"
            aria-label={label}
            @when {!path.is_empty()} { tooltip={path} }
            on:click={CommandsAction::OpenRepoPicker { anchored: true }}
        >
            <icon svg={lucide::FOLDER} size={Ico::SM} color={tc.icon} />
            <text size={TextStyle::Body.font_size(theme)} class="font-medium truncate" color={tc.text_strong}>{name}</text>
            {?worktree.map(|worktree| view! {
                <div class="flex-row items-center overflow-hidden" gap={tokens::scaled(Sp::XS, scale)} max_w={tokens::scaled(Layout::BRANCH_MAX_W, scale)}>
                    <icon svg={lucide::CHEVRON_RIGHT} size={Ico::XS} color={tc.text_muted} />
                    <icon svg={lucide::GIT_BRANCH} size={Ico::XS} color={tc.accent} />
                    <text size={TextStyle::Body.font_size(theme)} class="truncate" color={tc.accent}>{worktree}</text>
                </div>
            })}
            <icon svg={lucide::CHEVRON_DOWN} size={Ico::XS} color={tc.text_muted} />
        </div>
    }
}

fn repo_names(nav: &NavigationController) -> (String, Option<String>) {
    let Some(path) = nav.repo_path() else {
        return ("Open repository".to_owned(), None);
    };
    let trees = nav
        .repository()
        .map(|repo| repo.snapshot.worktrees.as_slice())
        .unwrap_or_default();
    match trees.iter().find(|tree| tree.current && !tree.main) {
        Some(current) => {
            let main = trees
                .iter()
                .find(|tree| tree.main)
                .map_or(path, |tree| tree.path.as_path());
            (folder_name(main), Some(folder_name(&current.path)))
        }
        None => (folder_name(path), None),
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub fn compare_bar(
    nav: &NavigationController,
    editing: Option<CompareField>,
    theme: &Theme,
) -> Option<AnyElement> {
    nav.repo_path()?;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let (left, right) = nav.compare_labels();
    let read_only = !nav.compare().working_copy;
    let uncommitted = nav
        .head_worktree()
        .map(|_| uncommitted_toggle(!nav.compare().committed_only, theme));
    let base = chip(nav, CompareField::Left, &left, editing, theme);
    let head = chip(nav, CompareField::Right, &right, editing, theme);
    let margin = tokens::scaled(Layout::COMPARE_ARROW_MARGIN, scale);
    Some(view! {
        <div
            class="flex-row items-center shrink-0"
            h={tokens::scaled(Layout::COMPARE_BAR_H, scale)}
            px={tokens::scaled(Sp::XXS, scale)}
            rounded={theme.metrics.control_radius * 1.5}
            bg={tc.element_background}
            border={tc.border_variant}
            accessibility_role={AccessibilityRole::Toolbar}
            aria-label="Compare"
            test_id="title.compare"
        >
            {base}
            <div class="flex-row items-center" px={margin}>
                <icon svg={lucide::ARROW_RIGHT} size={Ico::SM} color={tc.text_muted} />
            </div>
            {head}
            if let Some(toggle) = uncommitted {
                <div class="shrink-0" w={tokens::scaled(Sp::XS, scale)} />
                {toggle}
            }
            if read_only {
                <div class="shrink-0" w={tokens::scaled(Sp::XS, scale)} />
                <div
                    class="flex-row items-center"
                    gap={tokens::scaled(Sp::XS, scale)}
                    pl={tokens::scaled(Sp::SM, scale)}
                    pr={tokens::scaled(Sp::SM, scale)}
                    border_l={tc.border_variant}
                    accessibility_role={AccessibilityRole::Status}
                    aria-label="Read-only"
                >
                    <icon svg={icons::LOCK} size={Ico::XS} color={tc.text_muted} />
                    <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>"Read-only"</text>
                </div>
            }
        </div>
    })
}

fn uncommitted_toggle(on: bool, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let (icon, label, color) = if on {
        (lucide::PENCIL, "Uncommitted", tc.text_strong)
    } else {
        (icons::COMMIT, "Committed", tc.text_muted)
    };
    view! {
        <div
            class="flex-row items-center"
            h={tokens::scaled(Layout::COMPARE_CHIP_H, scale)}
            gap={tokens::scaled(Sp::XS, scale)}
            px={tokens::scaled(Sp::SM, scale)}
            border_l={tc.border_variant}
            rounded={theme.metrics.control_radius}
            hover_bg={tc.ghost_element_hover}
            cursor={CursorHint::Pointer}
            id="title.compare.uncommitted"
            test_id="title.compare.uncommitted"
            accessibility_role={AccessibilityRole::Button}
            accessibility_toggled={on}
            aria-label="Include uncommitted changes"
            tooltip={if on { "Showing the worktree's files, uncommitted changes included" } else { "Showing commits only" }}
            on:click={NavigationAction::ToggleCommittedOnly}
        >
            <icon svg={icon} size={Ico::XS} color={color} />
            <text size={TextStyle::BodySmall.font_size(theme)} color={color}>{label}</text>
        </div>
    }
}

fn chip(
    nav: &NavigationController,
    side: CompareField,
    label: &str,
    editing: Option<CompareField>,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let active = editing == Some(side);
    let (role, id) = match side {
        CompareField::Left => ("Base", "title.compare.base"),
        CompareField::Right => ("Head", "title.compare.head"),
    };
    let shown = if label.is_empty() {
        "Choose ref"
    } else {
        label
    };
    let mono = ref_icon(nav, label) == icons::COMMIT;
    view! {
        <div
            class="flex-row items-center"
            h={tokens::scaled(Layout::COMPARE_CHIP_H, scale)}
            pl={tokens::scaled(Sp::SM, scale)}
            pr={tokens::scaled(Sp::XS, scale)}
            gap={tokens::scaled(Sp::XS, scale)}
            rounded={theme.metrics.control_radius}
            bg={if active { tc.ghost_element_selected } else { Color::TRANSPARENT }}
            border={if active { tc.accent } else { Color::TRANSPARENT }}
            hover_bg={tc.ghost_element_hover}
            cursor={CursorHint::Pointer}
            id={id}
            test_id={id}
            role="button"
            aria-label={format!("{role} ref: {shown}")}
            on:click={NavigationAction::ChooseRef(side)}
        >
            <icon svg={ref_icon(nav, label)} size={Ico::SM} color={tc.text_muted} />
            if mono {
                <text size={TextStyle::Mono.font_size(theme)} class="font-mono font-medium truncate" color={tc.text_strong}>{shown.to_owned()}</text>
            } else {
                <text size={TextStyle::Body.font_size(theme)} class="font-medium truncate" color={tc.text_strong}>{shown.to_owned()}</text>
            }
            <icon svg={lucide::CHEVRON_DOWN} size={Ico::XS} color={tc.text_muted} />
        </div>
    }
}

fn ref_icon(nav: &NavigationController, label: &str) -> &'static str {
    if label == WORKING_TREE_LABEL {
        return lucide::PENCIL;
    }
    let kind = nav.repository().and_then(|repo| {
        repo.snapshot
            .refs
            .iter()
            .find(|r| r.name == label)
            .map(|r| r.kind)
    });
    match kind {
        Some(RefKind::Tag) => icons::TAG,
        Some(_) => lucide::GIT_BRANCH,
        None if is_sha(label.trim_end_matches('^')) => icons::COMMIT,
        None => lucide::GIT_BRANCH,
    }
}

fn is_sha(label: &str) -> bool {
    label.len() >= 7 && label.bytes().all(|b| b.is_ascii_hexdigit())
}
