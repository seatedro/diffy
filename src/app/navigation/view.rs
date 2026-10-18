use quark::view;
use quark_app::ViewContext;
use quark_components::{TabItem, search_field, tab_bar};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::design::Sz;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};
use quark_ui::virtual_list::virtual_list_window;

use super::NavigationController;
use super::rows::{self, Row};
use super::state::{ListMetrics, RepoStatus};
use crate::app::commands::keymap::WORKSPACE_CONTEXT;
use crate::app::shell::{Slot, empty_state};
use crate::app::text_targets::{SIDEBAR, SIDEBAR_FILTER};
use crate::app::tokens::{self, Ico, Sp, TextStyle};
use crate::contracts::navigation::{
    NavigationAction, SelectedFile, SelectionOrigin, SidebarMode, SidebarTab,
};
use crate::contracts::shell::{Action, LogicalFocus, ShellAction};
use crate::contracts::viewport::pipeline::{FileStats, ManifestFile};
use diffy_diff::FileStatus;

const OVERSCAN_ROWS: usize = 8;

pub fn view(nav: &mut NavigationController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let theme = cx.theme;
    let scale = tokens::ui_scale(theme);
    let filter_focused = cx.is_focused(SIDEBAR_FILTER);
    let list_focused = cx.is_focused(SIDEBAR);
    let header = header(nav, theme);
    let tab = nav.tab();
    let filter = (tab == SidebarTab::Files).then(|| filter_field(nav, filter_focused, theme));
    let row_height = theme.metrics.ui_row_height.round();
    let chrome =
        tokens::scaled(Sz::SIDEBAR_LIST_OFFSET, scale) * if filter.is_some() { 2.0 } else { 1.0 };
    let list_h = (slot.height - chrome).max(0.0);
    nav.set_metrics(ListMetrics {
        row_height,
        viewport: list_h,
    });
    let body = match tab {
        SidebarTab::Files => files(nav, list_h, list_focused, theme),
        SidebarTab::Commits => history(nav, list_h, theme),
        SidebarTab::OperationLog => operations(nav, list_h, theme),
    };
    view! {
        <div
            w={slot.width}
            h={slot.height}
            class="flex-col"
            bg={theme.colors.sidebar_background}
            accessibility_role={AccessibilityRole::Navigation}
            aria-label="Files"
            test_id="files"
        >
            {header}
            {?filter}
            <div class="flex-1 w-full" min_h={0.0}>{body}</div>
        </div>
    }
}

fn header(nav: &NavigationController, theme: &Theme) -> AnyElement {
    let scale = tokens::ui_scale(theme);
    let tc = &theme.colors;
    let tab = nav.tab();
    let mut items = vec![
        TabItem::new(
            SidebarTab::Files.label(),
            NavigationAction::SetSidebarTab(SidebarTab::Files),
        )
        .id("sidebar.files")
        .active(tab == SidebarTab::Files),
        TabItem::new(
            SidebarTab::Commits.label(),
            NavigationAction::SetSidebarTab(SidebarTab::Commits),
        )
        .id("sidebar.history")
        .active(tab == SidebarTab::Commits),
    ];
    if nav.operation_log_available() {
        items.push(
            TabItem::new(
                SidebarTab::OperationLog.label(),
                NavigationAction::SetSidebarTab(SidebarTab::OperationLog),
            )
            .id("sidebar.operations")
            .active(tab == SidebarTab::OperationLog),
        );
    }
    let mode_label = match nav.mode() {
        SidebarMode::List => "Show file tree",
        SidebarMode::Tree => "Show file list",
    };
    let mode_icon = match nav.mode() {
        SidebarMode::List => lucide::FOLDER,
        SidebarMode::Tree => lucide::LIST,
    };
    view! {
        <div
            class="flex-row items-end w-full shrink-0"
            h={tokens::scaled(Sz::SIDEBAR_LIST_OFFSET, scale)}
            px={tokens::scaled(Sp::SM, scale)}
            gap={tokens::scaled(Sp::XS, scale)}
        >
            <div class="flex-1" min_w={0.0}>{tab_bar(items).label("Sidebar")}</div>
            if tab == SidebarTab::Files {
                <div
                    class="flex-row items-center justify-center shrink-0"
                    w={theme.metrics.ui_row_height}
                    h={theme.metrics.ui_row_height}

                    rounded={theme.metrics.control_radius}
                    hover_bg={tc.ghost_element_hover}
                    cursor={CursorHint::Pointer}
                    id="sidebar.mode"
                    role="button"
                    aria-label={mode_label}
                    on:click={NavigationAction::ToggleSidebarMode}
                >
                    <icon svg={mode_icon} size={Ico::SM} color={tc.icon} />
                </div>
            }
        </div>
    }
}

fn filter_field(nav: &NavigationController, focused: bool, theme: &Theme) -> AnyElement {
    let m = &theme.metrics;
    let scale = tokens::ui_scale(theme);
    let filter = nav.filter();
    let has_text = !filter.text().is_empty();
    let input = text_input("Filter files", "")
        .field(filter)
        .placeholder("Filter files")
        .focus_target(SIDEBAR_FILTER)
        .focused(focused)
        .search(true)
        .bare()
        .w_full()
        .h((m.ui_small_font_size * 1.5).round());
    let field = search_field(
        input,
        has_text,
        Some(NavigationAction::ClearFilter.into()),
        Some("/"),
        theme,
    );
    let ring = focused.then(|| {
        let reach = tokens::scaled(Sz::FOCUS_RING_GAP, scale);
        let mut ring = div()
            .absolute()
            .left(-reach)
            .top(-reach)
            .right(-reach)
            .bottom(-reach)
            .rounded(m.control_radius + reach)
            .border(theme.colors.focus_border);
        ring.element_style_mut().border_widths = [tokens::scaled(Sz::FOCUS_RING_W, scale); 4];
        ring.into_any()
    });
    let escape: Action = if has_text {
        NavigationAction::EscapeFilter.into()
    } else {
        ShellAction::FocusLogical(Some(LogicalFocus::Sidebar)).into()
    };
    view! {
        <div class="w-full shrink-0" px={tokens::scaled(Sp::SM, scale)} py={tokens::scaled(Sp::XS, scale)}>
            <div class="relative w-full" on:key:escape={escape}>
                {field}
                {?ring}
            </div>
        </div>
    }
}

fn files(nav: &mut NavigationController, height: f32, focused: bool, theme: &Theme) -> AnyElement {
    let status = nav.status().clone();
    let has_manifest = nav.manifest().is_some();
    let filtering = !nav.filter().text().trim().is_empty();
    let selected = nav.selected_key();
    let scroll = nav.scroll_px();
    let tree = nav.mode() == SidebarMode::Tree && !filtering;
    let Some(manifest) = nav.manifest().cloned() else {
        return match status {
            RepoStatus::Closed => {
                empty_state("No files", "Open a repository to list its changes", theme)
            }
            RepoStatus::Opening => {
                empty_state("Opening repository", "Reading repository state", theme)
            }
            RepoStatus::Ready => empty_state("Loading changes", "Comparing revisions", theme),
            RepoStatus::Failed(message) => empty_state("Repository unavailable", &message, theme),
        };
    };
    nav.rows();
    let nav = &*nav;
    let rows = nav.cached_rows();
    if rows.is_empty() {
        return if filtering {
            empty_state("No matching files", "Try a different filter", theme)
        } else if has_manifest {
            empty_state(
                "No changes",
                "Nothing differs between these revisions",
                theme,
            )
        } else {
            empty_state("No files", "Open a repository to list its changes", theme)
        };
    }
    let row_height = theme.metrics.ui_row_height.round();
    let window = virtual_list_window(rows.len(), scroll, height, row_height, 0.0, OVERSCAN_ROWS);
    let visible: Vec<AnyElement> = window
        .range
        .clone()
        .map(|index| match &rows[index] {
            Row::Folder {
                path,
                depth,
                expanded,
            } => folder_row(path, *depth, *expanded, theme),
            Row::File { file, depth } => {
                let file = &manifest.files[*file as usize];
                let path = file.paths.display_path();
                file_row(
                    file,
                    *depth,
                    tree,
                    selected == Some(file.key),
                    nav.is_viewed(&path),
                    theme,
                )
            }
        })
        .collect();
    let tc = &theme.colors;
    view! {
        <div
            class="w-full flex-col overflow-hidden"
            h={height}
            scroll_y={scroll}
            scroll_total={window.total_extent}
            on:scroll={ScrollActionBuilder::new(|lines| Action::from(NavigationAction::ScrollFiles(lines)).into())
                .with_to_px(|px| Action::from(NavigationAction::ScrollFilesTo(px)).into())}
            focus_ring={SIDEBAR}
            key_context={WORKSPACE_CONTEXT}
            on:key:arrowdown={NavigationAction::SelectNextFile}
            on:key:arrowup={NavigationAction::SelectPreviousFile}
            accessibility_role={if tree { AccessibilityRole::Tree } else { AccessibilityRole::List }}
            aria-label="Changed files"
            test_id="file-list"
            @when {focused} { border={tc.focus_border} }
        >
            <div class="w-full shrink-0" h={window.top_spacer} />
            {...visible}
            <div class="w-full shrink-0" h={window.bottom_spacer} />
        </div>
    }
}

fn status_mark(file: &ManifestFile, theme: &Theme) -> (&'static str, Color) {
    let tc = &theme.colors;
    if file.binary {
        return ("B", tc.text_muted);
    }
    match file.status {
        FileStatus::Added => ("A", tc.line_add_text),
        FileStatus::Deleted => ("D", tc.line_del_text),
        FileStatus::Modified => ("M", tc.status_warning),
        FileStatus::Renamed => ("R", tc.status_info),
        FileStatus::Copied => ("C", tc.status_info),
    }
}

fn file_row(
    file: &ManifestFile,
    depth: u16,
    tree: bool,
    selected: bool,
    viewed: bool,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let path = file.paths.display_path().into_owned();
    let (dir, name) = rows::file_name(&path);
    let dir = (!tree && !dir.is_empty()).then(|| dir.to_owned());
    let name = name.to_owned();
    let (mark, mark_color) = status_mark(file, theme);
    let stats = match file.stats {
        FileStats::Known {
            additions,
            deletions,
        } => Some((additions, deletions)),
        FileStats::Pending => None,
    };
    let key = file.key;
    let indent = tokens::scaled(Sp::LG, scale) * f32::from(depth);
    let small = TextStyle::BodySmall.font_size(theme);
    let viewed_label = if viewed {
        "Mark unviewed"
    } else {
        "Mark viewed"
    };
    view! {
        <div
            class="flex-row items-center w-full shrink-0"
            h={theme.metrics.ui_row_height.round()}
            pl={tokens::scaled(Sp::SM, scale) + indent}
            pr={tokens::scaled(Sp::XS, scale)}
            gap={tokens::scaled(Sp::SM, scale)}
            bg={if selected { tc.sidebar_row_selected } else { Color::TRANSPARENT }}
            hover_bg={if selected { tc.sidebar_row_selected } else { tc.sidebar_row_hover }}
            cursor={CursorHint::Pointer}
            key={format!("file:{}", key.0)}
            id={format!("file:{}", key.0)}
            test_id={format!("file:{}", key.0)}
            accessibility_role={if tree { AccessibilityRole::TreeItem } else { AccessibilityRole::ListItem }}
            aria-label={path.clone()}
            aria-selected={selected}
            on:click={NavigationAction::SelectFile(SelectedFile { key, origin: SelectionOrigin::Sidebar })}
        >
            <text size={small} class="font-mono font-semibold" color={mark_color}>{mark}</text>
            <div class="flex-1 flex-row items-center overflow-hidden" min_w={0.0} gap={tokens::scaled(Sp::XS, scale)}>
                <text
                    size={TextStyle::Body.font_size(theme)}
                    class="truncate"
                    color={if viewed { tc.text_muted } else { tc.text }}
                >
                    {name}
                </text>
                {?dir.map(|dir| view! { <text size={small} class="truncate" color={tc.text_muted}>{dir}</text> })}
            </div>
            {?stats.map(|(add, del)| view! {
                <div class="flex-row shrink-0" gap={tokens::scaled(Sp::XS, scale)}>
                    if add > 0 { <text size={small} color={tc.line_add_text}>{format!("+{add}")}</text> }
                    if del > 0 { <text size={small} color={tc.line_del_text}>{format!("-{del}")}</text> }
                </div>
            })}
            <div
                class="flex-row items-center justify-center shrink-0"
                w={tokens::scaled(Sp::XL, scale)}
                h={tokens::scaled(Sp::XL, scale)}
                rounded={theme.metrics.control_radius}
                hover_bg={tc.ghost_element_hover}
                id={format!("file:{}:viewed", key.0)}
                role="checkbox"
                aria-label={viewed_label}
                aria-checked={viewed}
                on:click={NavigationAction::ToggleFileViewed(key)}
            >
                <icon
                    svg={if viewed { lucide::CHECK } else { lucide::EYE }}
                    size={Ico::XS}
                    color={if viewed { tc.accent } else { tc.text_muted }}
                />
            </div>
        </div>
    }
}

fn folder_row(path: &str, depth: u16, expanded: bool, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let name = path.rsplit('/').next().unwrap_or(path).to_owned();
    let indent = tokens::scaled(Sp::LG, scale) * f32::from(depth);
    view! {
        <div
            class="flex-row items-center w-full shrink-0"
            h={theme.metrics.ui_row_height.round()}
            pl={tokens::scaled(Sp::SM, scale) + indent}
            gap={tokens::scaled(Sp::XS, scale)}
            hover_bg={tc.sidebar_row_hover}
            cursor={CursorHint::Pointer}
            id={format!("folder:{path}")}
            accessibility_role={AccessibilityRole::TreeItem}
            aria-label={path.to_owned()}
            aria-expanded={expanded}
            on:click={NavigationAction::ToggleFolder(path.to_owned())}
        >
            <icon svg={if expanded { lucide::CHEVRON_DOWN } else { lucide::CHEVRON_RIGHT }} size={Ico::XS} color={tc.icon} />
            <icon svg={if expanded { lucide::FOLDER_OPEN } else { lucide::FOLDER }} size={Ico::XS} color={tc.text_muted} />
            <text size={TextStyle::Body.font_size(theme)} class="truncate" color={tc.text}>{name}</text>
        </div>
    }
}

fn history(nav: &mut NavigationController, height: f32, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let state = nav.history();
    if state.loading {
        return empty_state("Loading history", "Reading commits in this range", theme);
    }
    if let Some(error) = &state.error {
        return empty_state("History unavailable", error, theme);
    }
    let selected = state.selected.clone();
    let scroll = state.scroll_px;
    let drilled = selected.is_some();
    let Some(commits) = nav.history_commits() else {
        return empty_state("No history", "Open a repository to see commits", theme);
    };
    if commits.is_empty() {
        return empty_state("No commits", "This comparison has no commits", theme);
    }
    let row_height = theme.metrics.ui_row_height.round();
    let back_h = if drilled { row_height } else { 0.0 };
    let list_h = (height - back_h).max(0.0);
    let window = virtual_list_window(
        commits.len(),
        scroll,
        list_h,
        row_height,
        0.0,
        OVERSCAN_ROWS,
    );
    let rows: Vec<AnyElement> = commits[window.range.clone()]
        .iter()
        .map(|commit| {
            let revision = commit.revision.id.clone();
            let is_selected = selected.as_deref().is_some_and(|s| revision.starts_with(s) || s == commit.short_revision);
            view! {
                <div
                    class="flex-row items-center w-full shrink-0"
                    h={row_height}
                    px={tokens::scaled(Sp::SM, scale)}
                    gap={tokens::scaled(Sp::SM, scale)}
                    bg={if is_selected { tc.sidebar_row_selected } else { Color::TRANSPARENT }}
                    hover_bg={tc.sidebar_row_hover}
                    cursor={CursorHint::Pointer}
                    key={revision.clone()}
                    id={format!("commit:{revision}")}
                    accessibility_role={AccessibilityRole::ListItem}
                    aria-label={commit.summary.clone()}
                    aria-selected={is_selected}
                    on:click={NavigationAction::SelectHistoryCommit(revision.clone())}
                >
                    <text size={TextStyle::MonoSmall.font_size(theme)} color={tc.text_accent}>{commit.short_revision.clone()}</text>
                    <text size={TextStyle::Body.font_size(theme)} class="truncate" color={tc.text}>{commit.summary.clone()}</text>
                </div>
            }
        })
        .collect();
    let back = drilled.then(|| {
        view! {
            <div
                class="flex-row items-center w-full shrink-0"
                h={row_height}
                px={tokens::scaled(Sp::SM, scale)}
                gap={tokens::scaled(Sp::XS, scale)}
                hover_bg={tc.sidebar_row_hover}
                cursor={CursorHint::Pointer}
                id="history.back"
                role="button"
                aria-label="Back to comparison"
                on:click={NavigationAction::ClearHistoryCommit}
            >
                <icon svg={lucide::CORNER_UP_LEFT} size={Ico::XS} color={tc.icon} />
                <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>"Back to comparison"</text>
            </div>
        }
    });
    view! {
        <div class="w-full flex-col" h={height}>
            {?back}
            <div
                class="w-full flex-col overflow-hidden"
                h={list_h}
                scroll_y={scroll}
                scroll_total={window.total_extent}
                on:scroll={ScrollActionBuilder::new(|lines| Action::from(NavigationAction::ScrollHistory(lines)).into())}
                key_context={WORKSPACE_CONTEXT}
                accessibility_role={AccessibilityRole::List}
                aria-label="Commits"
            >
                <div class="w-full shrink-0" h={window.top_spacer} />
                {...rows}
                <div class="w-full shrink-0" h={window.bottom_spacer} />
            </div>
        </div>
    }
}

fn operations(nav: &NavigationController, height: f32, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let Some(repository) = nav.repository() else {
        return empty_state(
            "No operations",
            "Open a repository to see its operation log",
            theme,
        );
    };
    let log = &repository.snapshot.operation_log;
    if log.is_empty() {
        return empty_state(
            "No operations",
            "This repository has no recorded operations",
            theme,
        );
    }
    let selected = nav.selected_operation();
    let row_height = theme.metrics.ui_row_height.round();
    let rows: Vec<AnyElement> = log
        .iter()
        .take((height / row_height.max(1.0)).ceil() as usize + OVERSCAN_ROWS)
        .map(|entry| {
            let is_selected = selected == Some(entry.operation_id.as_str());
            let detail = [entry.user.as_str(), entry.time.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" - ");
            view! {
                <div
                    class="flex-row items-center w-full shrink-0"
                    h={row_height}
                    px={tokens::scaled(Sp::SM, scale)}
                    gap={tokens::scaled(Sp::SM, scale)}
                    bg={if is_selected { tc.sidebar_row_selected } else { Color::TRANSPARENT }}
                    hover_bg={tc.sidebar_row_hover}
                    cursor={CursorHint::Pointer}
                    key={entry.operation_id.clone()}
                    id={format!("operation:{}", entry.operation_id)}
                    accessibility_role={AccessibilityRole::ListItem}
                    aria-label={entry.description.clone()}
                    aria-selected={is_selected}
                    on:click={NavigationAction::SelectOperation(entry.operation_id.clone())}
                >
                    <text size={TextStyle::MonoSmall.font_size(theme)} color={tc.text_accent}>{entry.short_operation_id.clone()}</text>
                    <div class="flex-1 flex-col" min_w={0.0}>
                        <text size={TextStyle::Body.font_size(theme)} class="truncate" color={tc.text}>{entry.description.clone()}</text>
                        <text size={TextStyle::Caption.font_size(theme)} class="truncate" color={tc.text_muted}>{detail}</text>
                    </div>
                </div>
            }
        })
        .collect();
    view! {
        <div
            class="w-full flex-col overflow-hidden"
            h={height}
            accessibility_role={AccessibilityRole::List}
            aria-label="Operations"
        >
            {...rows}
        </div>
    }
}
