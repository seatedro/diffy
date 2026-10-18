use quark::view;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::NavigationController;
use crate::app::icons;
use crate::app::tokens::{self, Ico, Layout, Sp, TextStyle};
use crate::contracts::commands::CommandsAction;
use crate::contracts::navigation::{CompareField, NavigationAction};
use crate::core::ref_catalog::WORKING_TREE_LABEL;
use crate::core::vcs::model::RefKind;

pub fn repo_chip(nav: &NavigationController, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let name = nav
        .repo_path()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Open repository".to_owned());
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
            aria-label={format!("Repository {name}")}
            on:click={CommandsAction::OpenRepoPicker}
        >
            <icon svg={lucide::FOLDER} size={Ico::SM} color={tc.icon} />
            <text size={TextStyle::Body.font_size(theme)} class="font-medium truncate" color={tc.text_strong}>{name}</text>
            <icon svg={lucide::CHEVRON_DOWN} size={Ico::XS} color={tc.text_muted} />
        </div>
    }
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
            border={tc.control_border}
            accessibility_role={AccessibilityRole::Toolbar}
            aria-label="Compare"
            test_id="title.compare"
        >
            {base}
            <div class="flex-row items-center" px={margin}>
                <icon svg={lucide::ARROW_RIGHT} size={Ico::SM} color={tc.text_muted} />
            </div>
            {head}
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
        None if label.len() >= 7 && label.bytes().all(|b| b.is_ascii_hexdigit()) => icons::COMMIT,
        None => lucide::GIT_BRANCH,
    }
}
