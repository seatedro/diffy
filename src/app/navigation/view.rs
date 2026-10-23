use quark::view;
use quark_app::ViewContext;
use quark_components::{TabItem, progress_bar, search_field, tab_bar};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::design::Sz;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};
use quark_ui::virtual_list::virtual_list_window;

use super::NavigationController;
use super::row_actions::{file_row_actions, group_action, selection_actions};
use super::rows::{self, Row};
use super::smart_sort::SortStatus;
use super::state::{ListMetrics, RepoStatus, commit_matches};
use crate::app::commands::keymap::workspace_context;
use crate::app::icons;
use crate::app::mutations::{
    Chrome, Control, RowAction, Tone, change_actions, context_menu, operation_log_actions,
    render_control,
};
use crate::app::review::{ReviewController, author_avatar};
use crate::app::shell::{Slot, empty_state};
use crate::app::text_targets::{SIDEBAR, SIDEBAR_FILTER};
use crate::app::tokens::{self, Ico, Layout, Sp, TextStyle, Tint};
use crate::app::viewport::{ViewportController, outline};
use crate::contracts::commands::CommandsAction;
use crate::contracts::navigation::{
    NavigationAction, SelectedFile, SelectionOrigin, SidebarMode, SidebarTab,
};
use crate::contracts::shared::SharedContext;
use crate::contracts::shared::SourceTarget;
use crate::contracts::shell::{Action, LogicalFocus, ShellAction};
use crate::contracts::viewport::files::{FileAt, FilesAction, revision_label};
use crate::contracts::viewport::interaction::InteractionAction;
use crate::contracts::viewport::pipeline::{CompareManifestSnapshot, FileStats, ManifestFile};
use crate::core::compare::CompareMode;
use crate::core::ref_catalog::relative_age;
use crate::core::vcs::git::WORKDIR_REF;
use crate::core::vcs::model::{ChangeBucket, RevisionId, VcsChange};
use diffy_diff::FileStatus;
use diffy_diff::ItemChange;
use quark_components::kbd;

const OVERSCAN_ROWS: usize = 8;
const MAX_TAG_PILLS: usize = 2;
const FILE_ROW_GROUP: &str = "sidebar.file-row";

pub fn view(
    nav: &mut NavigationController,
    review: &ReviewController,
    viewport: &ViewportController,
    slot: &Slot,
    commit_box: Option<(AnyElement, f32)>,
    cx: &mut ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let scale = tokens::ui_scale(theme);
    let filter_focused = cx.is_focused(SIDEBAR_FILTER);
    let focus = cx.focus();
    let header = header(nav, theme);
    let tab = nav.tab();
    let filter = matches!(tab, SidebarTab::Files | SidebarTab::Commits)
        .then(|| filter_field(nav, tab, filter_focused, theme));
    let progress = (tab == SidebarTab::Files)
        .then(|| progress(nav, theme))
        .flatten();
    let selection = (tab == SidebarTab::Files)
        .then(|| selection_bar(nav, slot.shared, theme))
        .flatten();
    let commit_box = commit_box.filter(|_| tab == SidebarTab::Files);
    let row_height = theme.metrics.ui_row_height.round();
    let offset = tokens::scaled(Sz::SIDEBAR_LIST_OFFSET, scale);
    let chrome = offset
        + if filter.is_some() { offset } else { 0.0 }
        + if progress.is_some() {
            progress_height(theme)
        } else {
            0.0
        }
        + if selection.is_some() { offset } else { 0.0 }
        + commit_box.as_ref().map_or(0.0, |(_, h)| *h);
    let list_h = (slot.height - chrome).max(0.0);
    nav.set_metrics(ListMetrics {
        row_height,
        viewport: list_h,
        history_row: tokens::scaled(Layout::HISTORY_ROW_H, scale),
    });
    let body = match tab {
        SidebarTab::Files => files(nav, slot.shared, list_h, focus, theme),
        SidebarTab::Commits => history(nav, review, list_h, focus, theme),
        SidebarTab::OperationLog => operations(nav, list_h, theme),
        SidebarTab::Outline => outline::view(viewport, slot.shared, slot.width, list_h, cx),
    };
    let commit_box = commit_box.map(|(element, _)| element);
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
            {?progress}
            <div class="w-full" h={list_h}>{body}</div>
            {?selection}
            {?commit_box}
        </div>
    }
}

fn header(nav: &NavigationController, theme: &Theme) -> AnyElement {
    let scale = tokens::ui_scale(theme);
    let tc = &theme.colors;
    let tab = nav.tab();
    let count = nav.manifest().map(|m| m.files.len()).unwrap_or(0);
    let mut changes = TabItem::new(
        SidebarTab::Files.label(),
        NavigationAction::SetSidebarTab(SidebarTab::Files),
    )
    .id("sidebar.files")
    .active(tab == SidebarTab::Files);
    if count > 0 {
        changes = changes.count(count.to_string());
    }
    let mut items = vec![
        changes,
        TabItem::new(
            SidebarTab::Commits.label(),
            NavigationAction::SetSidebarTab(SidebarTab::Commits),
        )
        .id("sidebar.history")
        .active(tab == SidebarTab::Commits),
        TabItem::new(
            SidebarTab::Outline.label(),
            NavigationAction::SetSidebarTab(SidebarTab::Outline),
        )
        .id("sidebar.outline")
        .active(tab == SidebarTab::Outline),
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
    let control = tokens::scaled(Layout::CONTROL_SM_H, scale);
    view! {
        <div
            class="flex-row items-end w-full shrink-0"
            h={tokens::scaled(Sz::SIDEBAR_LIST_OFFSET, scale)}
        >
            <div class="flex-1 h-full" min_w={0.0}>
                {tab_bar(items).label("Sidebar").full_width().full_height()}
            </div>
            <div
                class="flex-row items-center shrink-0 h-full"
                pr={tokens::scaled(Sp::SM, scale)}
                border_b={tc.border_variant}
            >
                if tab == SidebarTab::Files {
                    <div
                        class="flex-row items-center justify-center"
                        w={control}
                        h={control}
                        rounded={theme.metrics.control_radius}
                        hover_bg={tc.ghost_element_hover}
                        cursor={CursorHint::Pointer}
                        id="sidebar.mode"
                        role="button"
                        aria-label={mode_label}
                        tooltip={mode_label}
                        on:click={NavigationAction::ToggleSidebarMode}
                    >
                        <icon svg={mode_icon} size={Ico::SM} color={tc.icon} />
                    </div>
                }
            </div>
        </div>
    }
}

fn filter_field(
    nav: &NavigationController,
    tab: SidebarTab,
    focused: bool,
    theme: &Theme,
) -> AnyElement {
    let m = &theme.metrics;
    let scale = tokens::ui_scale(theme);
    let filter = nav.filter();
    let has_text = !filter.text().is_empty();
    let label = if tab == SidebarTab::Commits {
        "Filter commits"
    } else {
        "Filter files"
    };
    let input = text_input(label, "")
        .field(filter)
        .placeholder(label)
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
        (!focused).then_some("/"),
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
    let sort = (tab == SidebarTab::Files).then(|| smart_sort_toggle(nav, theme));
    view! {
        <div
            class="flex-row items-center w-full shrink-0"
            px={tokens::scaled(Sp::SM, scale)}
            py={tokens::scaled(Sp::XS, scale)}
            gap={tokens::scaled(Sp::XS, scale)}
        >
            <div class="relative flex-1" min_w={0.0} on:key:escape={escape}>
                {field}
                {?ring}
            </div>
            {?sort}
        </div>
    }
}

fn smart_sort_toggle(nav: &NavigationController, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let active = nav.smart_sort_active();
    let status = nav.smart_sort_status();
    let tooltip = if active {
        format!("Smart sort: {}", status.describe())
    } else {
        "Smart sort: most important files first".to_owned()
    };
    let (svg, color) = match (active, status) {
        (true, SortStatus::Ranking) => (lucide::LOADER, tc.text_muted),
        (true, _) => (lucide::SPARKLES, tc.accent),
        (false, _) => (lucide::SPARKLES, tc.icon),
    };
    let control = tokens::scaled(Layout::CONTROL_SM_H, scale);
    view! {
        <div
            class="flex-row items-center justify-center shrink-0"
            w={control}
            h={control}
            rounded={theme.metrics.control_radius}
            bg={if active { tc.ghost_element_selected } else { Color::TRANSPARENT }}
            hover_bg={tc.ghost_element_hover}
            cursor={CursorHint::Pointer}
            id="sidebar.smart-sort"
            test_id="sidebar.smart-sort"
            role="checkbox"
            aria-label="Smart sort"
            aria-checked={active}
            tooltip={tooltip}
            on:click={NavigationAction::ToggleSmartSort}
        >
            <icon svg={svg} size={Ico::SM} color={color} />
        </div>
    }
}

fn progress_height(theme: &Theme) -> f32 {
    tokens::scaled(Sp::XL, tokens::ui_scale(theme))
}

fn progress(nav: &NavigationController, theme: &Theme) -> Option<AnyElement> {
    let (viewed, total) = nav.viewed_count();
    if total == 0 {
        return None;
    }
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let fraction = viewed as f32 / total as f32;
    let label = format!("{viewed} of {total} viewed");
    Some(view! {
        <div
            class="flex-row items-center w-full shrink-0"
            h={progress_height(theme)}
            px={tokens::scaled(Sp::MD, scale)}
            gap={tokens::scaled(Sp::SM, scale)}
            accessibility_role={AccessibilityRole::ProgressIndicator}
            aria-label={label.clone()}
            test_id="sidebar.progress"
        >
            <div class="flex-1" min_w={0.0}>{progress_bar(fraction)}</div>
            <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{label}</text>
        </div>
    })
}

fn selection_bar(
    nav: &NavigationController,
    cx: &SharedContext,
    theme: &Theme,
) -> Option<AnyElement> {
    let marked = nav.marked();
    if marked.len() < 2 {
        return None;
    }
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let actions = selection_actions(marked, cx);
    let buttons: Vec<AnyElement> = actions.iter().map(|a| text_action(a, theme)).collect();
    Some(view! {
        <div
            class="flex-row items-center w-full shrink-0"
            h={tokens::scaled(Sz::SIDEBAR_LIST_OFFSET, scale)}
            px={tokens::scaled(Sp::MD, scale)}
            gap={tokens::scaled(Sp::SM, scale)}
            border_t={tc.border_variant}
            accessibility_role={AccessibilityRole::Toolbar}
            aria-label="Selected files"
        >
            <text size={TextStyle::BodySmall.font_size(theme)} class="font-medium" color={tc.text}>
                {format!("{} files selected", marked.len())}
            </text>
            <div class="flex-1" />
            {...buttons}
        </div>
    })
}

fn files(
    nav: &mut NavigationController,
    cx: &SharedContext,
    height: f32,
    focus: Option<FocusId>,
    theme: &Theme,
) -> AnyElement {
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
        } else if let Some((title, detail)) = nav.merge_base_hint() {
            empty_state(&title, &detail, theme)
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
    let active = within(
        focus,
        selected.map(|key| format!("file:{}", key.0)).as_deref(),
    );
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
                let highlights = nav.highlights(*file);
                let file = &manifest.files[*file as usize];
                let path = file.paths.display_path();
                let primary = selected == Some(file.key);
                let state = RowState {
                    tree,
                    selected: primary || nav.is_marked(file.key),
                    focused: active,
                    viewed: nav.is_viewed(&path),
                    highlights,
                };
                let actions = file_row_actions(file, cx);
                let menu = view_file_actions(file, &manifest, cx);
                file_row(file, *depth, state, &actions, menu, theme)
            }
            Row::Section { bucket, files } => section_row(
                rows::section_label(*bucket),
                *files,
                group_action(*bucket, cx),
                theme,
            ),
            Row::Pending { files } => section_row("To review", *files, None, theme),
            Row::Viewed { files, expanded } => viewed_row(*files, *expanded, theme),
            Row::Item { file, item } => {
                let file = &manifest.files[*file as usize];
                item_row(file, *item, tree, theme)
            }
        })
        .collect();
    view! {
        <div
            class="w-full flex-col overflow-hidden scrollbar-auto-hide"
            h={height}
            px={theme.metrics.spacing_sm}
            scroll_y={scroll}
            scroll_total={window.total_extent}
            scrollbar_visibility={nav.files_scrollbar()}
            on:scroll={ScrollActionBuilder::new(|lines| Action::from(NavigationAction::ScrollFiles(lines)).into())
                .with_to_px(|px| Action::from(NavigationAction::ScrollFilesTo(px)).into())}
            focus_ring={SIDEBAR}
            key_context={workspace_context(cx.review.is_some())}
            on:key:arrowdown={NavigationAction::SelectNextFile}
            on:key:arrowup={NavigationAction::SelectPreviousFile}
            accessibility_role={if tree { AccessibilityRole::Tree } else { AccessibilityRole::ListBox }}
            aria-label="Changed files"
            test_id="file-list"
        >
            <div class="w-full shrink-0" h={window.top_spacer} />
            {...visible}
            <div class="w-full shrink-0" h={window.bottom_spacer} />
        </div>
    }
}

fn within(focus: Option<FocusId>, row: Option<&str>) -> bool {
    focus.is_some_and(|focus| {
        focus == SIDEBAR || row.is_some_and(|row| focus == FocusId::from_key(row))
    })
}

fn status_badge(file: &ManifestFile, theme: &Theme) -> (&'static str, Color) {
    let tc = &theme.colors;
    if file.binary {
        return ("B", tc.text_muted);
    }
    match file.status {
        FileStatus::Added if file.buckets.contains(&ChangeBucket::Untracked) => {
            ("?", tc.text_muted)
        }
        FileStatus::Added => ("A", tc.line_add_text),
        FileStatus::Deleted => ("D", tc.line_del_text),
        FileStatus::Modified => ("M", tc.status_warning),
        FileStatus::Renamed => ("R", tc.status_info),
        FileStatus::Copied => ("C", tc.status_info),
    }
}

struct RowState<'a> {
    tree: bool,
    selected: bool,
    focused: bool,
    viewed: bool,
    highlights: &'a [(usize, usize)],
}

fn view_file_actions(
    file: &ManifestFile,
    manifest: &CompareManifestSnapshot,
    cx: &SharedContext,
) -> Vec<RowAction> {
    let Some(repository) = cx.repository.as_ref() else {
        return Vec::new();
    };
    let backend = repository.snapshot.location.kind;
    let head = if manifest.working_tree {
        WORKDIR_REF.to_owned()
    } else {
        manifest.resolved_right.clone()
    };
    let sides = [
        ("View Full File", file.paths.new_path(), head, "file.view"),
        (
            "View Base File",
            file.paths.old_path(),
            manifest.resolved_left.clone(),
            "file.view-base",
        ),
    ];
    sides
        .into_iter()
        .filter_map(|(label, path, id, test_id)| {
            let path = path?.into_owned();
            (!id.is_empty()).then(|| RowAction {
                label: label.to_owned(),
                action: FilesAction::Open(FileAt {
                    path,
                    label: revision_label(&id),
                    revision: RevisionId { backend, id },
                })
                .into(),
                destructive: false,
                icon: Some(lucide::FILE),
                shortcut: None,
                test_id: test_id.to_owned(),
            })
        })
        .collect()
}

fn file_row(
    file: &ManifestFile,
    depth: u16,
    state: RowState,
    actions: &[RowAction],
    menu: Vec<RowAction>,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let path = file.paths.display_path().into_owned();
    let (dir, name) = rows::file_name(&path);
    let name_start = path.len() - name.len();
    let name_ranges: Vec<(usize, usize)> = state
        .highlights
        .iter()
        .filter(|(start, _)| *start >= name_start)
        .map(|(start, end)| (start - name_start, end - name_start))
        .collect();
    let dir_ranges: Vec<(usize, usize)> = state
        .highlights
        .iter()
        .filter(|(_, end)| *end <= dir.len())
        .copied()
        .collect();
    let show_dir = !state.tree && !dir.is_empty();
    let (mark, mark_color) = status_badge(file, theme);
    let mark_color = if state.viewed {
        tc.text_muted
    } else {
        mark_color
    };
    let stats = match file.stats {
        FileStats::Known {
            additions,
            deletions,
        } => Some((additions, deletions)),
        FileStats::Pending => None,
    };
    let deleted = file.status == FileStatus::Deleted;
    let key = file.key;
    let indent = tokens::scaled(Sp::LG, scale) * f32::from(depth);
    let small = TextStyle::BodySmall.font_size(theme);
    let mono = TextStyle::MonoSmall.font_size(theme);
    let label = row_label(&path, file, stats);
    let viewed = state.viewed;
    let viewed_label = if viewed {
        "Mark unviewed"
    } else {
        "Mark viewed"
    };
    let badge = tokens::scaled(Layout::STATUS_BADGE, scale);
    let base = tc.sidebar_background;
    let paint = tokens::row_paint(theme, base, state.selected, state.focused);
    let name_color = if viewed || deleted {
        paint.muted
    } else {
        paint.text
    };
    let (dir_color, add_color, del_color) = if viewed && !state.selected {
        (tc.placeholder, paint.muted, paint.muted)
    } else if viewed {
        (paint.muted, paint.muted, paint.muted)
    } else {
        (paint.muted, tc.line_add_text, tc.line_del_text)
    };
    let row_bg = paint.bg;
    let hover_bg = if state.selected {
        row_bg
    } else {
        tc.sidebar_row_hover
    };
    let group = GroupId::new(FILE_ROW_GROUP);
    let buttons: Vec<AnyElement> = actions.iter().map(|a| icon_action(a, theme)).collect();
    let viewed_toggle = view! {
        <div
            class="flex-row items-center justify-center shrink-0"
            w={tokens::scaled(Layout::CONTROL_SM_H, scale)}
            h={tokens::scaled(Layout::CONTROL_SM_H, scale)}
            rounded={theme.metrics.control_radius}
            hover_bg={tc.ghost_element_hover}
            id={format!("file:{}:viewed", key.0)}
            role="checkbox"
            aria-label={viewed_label}
            aria-checked={viewed}
            tooltip={viewed_label}
            on:click={NavigationAction::ToggleFileViewed(key)}
        >
            <icon
                svg={if viewed { lucide::EYE_OFF } else { lucide::EYE }}
                size={Ico::XS}
                color={paint.icon}
            />
        </div>
    };
    let tools = div()
        .absolute()
        .right(tokens::scaled(Sp::XS, scale))
        .top(0.0)
        .bottom(0.0)
        .flex_row()
        .items_center()
        .gap(tokens::scaled(Sp::XXS, scale))
        .pl(tokens::scaled(Sp::SM, scale))
        .bg(hover_bg)
        .show_when(GroupCondition::HoveredOrFocusWithin(group))
        .children(buttons)
        .child(viewed_toggle);
    view! {
        <div
            class="relative flex-row items-center w-full shrink-0"
            h={theme.metrics.ui_row_height.round()}
            pl={tokens::scaled(Sp::SM, scale) + indent}
            pr={tokens::scaled(Sp::SM, scale)}
            gap={tokens::scaled(Sp::SM, scale)}
            rounded={theme.metrics.control_radius}
            bg={row_bg}
            hover_bg={hover_bg}
            interaction_group={group}
            cursor={CursorHint::Pointer}
            key={format!("file:{}", key.0)}
            id={format!("file:{}", key.0)}
            test_id={format!("file:{}", key.0)}
            accessibility_role={if state.tree { AccessibilityRole::TreeItem } else { AccessibilityRole::ListBoxOption }}
            aria-label={label}
            aria-selected={state.selected}
            on:click={NavigationAction::SelectFile(SelectedFile { key, origin: SelectionOrigin::Sidebar })}
            on:context_menu={row_menu(menu)}
        >
            <div
                class="flex-row items-center justify-center shrink-0"
                w={badge}
                h={badge}
                rounded={theme.metrics.control_radius * 0.6}
                bg={base.lerp(mark_color, Tint::BADGE)}
            >
                <text size={mono} class="font-mono font-semibold" color={mark_color}>{mark}</text>
            </div>
            <div class="flex-1 flex-row items-center overflow-hidden" min_w={0.0} gap={tokens::scaled(Sp::SM - Sp::XXS, scale)}>
                if deleted {
                    <text size={TextStyle::Body.font_size(theme)} class="truncate font-medium" strikethrough color={name_color}>{name.to_owned()}</text>
                } else {
                    {highlighted_text(name, &name_ranges, name_color, TextStyle::Body.font_size(theme), theme)}
                }
                if show_dir {
                    {highlighted_text(dir, &dir_ranges, dir_color, small, theme)}
                }
            </div>
            {?stats.map(|(add, del)| view! {
                <div class="flex-row shrink-0" gap={tokens::scaled(Sp::XS, scale)}>
                    if add > 0 { <text size={mono} class="font-mono" color={add_color}>{format!("+{add}")}</text> }
                    if del > 0 { <text size={mono} class="font-mono" color={del_color}>{format!("\u{2212}{del}")}</text> }
                </div>
            })}
            {tools}
        </div>
    }
}

fn item_row(file: &ManifestFile, index: u32, tree: bool, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let Some(item) = file.outline.get(index as usize) else {
        return view! { <div h={theme.metrics.ui_row_height.round()} /> };
    };
    let summary = match item.change {
        ItemChange::Added => "added".to_owned(),
        ItemChange::Removed => "removed".to_owned(),
        ItemChange::Renamed => "rename".to_owned(),
        ItemChange::Moved => "moved".to_owned(),
        ItemChange::Edited(1) => "1 edit".to_owned(),
        ItemChange::Edited(n) => format!("{n} edits"),
    };
    let removed = item.change == ItemChange::Removed;
    let target = SourceTarget {
        file: file.key,
        revision: file.revision,
        side: item.side,
        lines: item.line..item.line + 1,
    };
    let mono = TextStyle::MonoSmall.font_size(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let indent = tokens::scaled(Sp::LG, scale) + tokens::scaled(Layout::STATUS_BADGE, scale);
    let label = format!("{} {}, {summary}", item.kind, item.name);
    view! {
        <div
            class="relative flex-row items-center w-full shrink-0"
            h={theme.metrics.ui_row_height.round()}
            pl={tokens::scaled(Sp::SM, scale) + indent}
            pr={tokens::scaled(Sp::SM, scale)}
            gap={tokens::scaled(Sp::SM, scale)}
            rounded={theme.metrics.control_radius}
            hover_bg={tc.sidebar_row_hover}
            cursor={CursorHint::Pointer}
            key={format!("file:{}:item:{index}", file.key.0)}
            id={format!("file:{}:item:{index}", file.key.0)}
            accessibility_role={if tree { AccessibilityRole::TreeItem } else { AccessibilityRole::ListBoxOption }}
            aria-label={label}
            on:click={Action::from(InteractionAction::SetSourceSelection(Some(target)))}
        >
            <div class="flex-1 flex-row items-center overflow-hidden" min_w={0.0} gap={tokens::scaled(Sp::XS, scale)}>
                <text size={mono} class="font-mono" color={tc.syntax_keyword}>{item.kind.to_string()}</text>
                if removed {
                    <text size={mono} class="font-mono truncate" strikethrough color={tc.text_muted}>{item.name.to_string()}</text>
                } else {
                    <text size={mono} class="font-mono truncate" color={tc.text}>{item.name.to_string()}</text>
                }
            </div>
            <div class="shrink-0">
                <text size={small} color={tc.text_muted}>{summary}</text>
            </div>
        </div>
    }
}

fn highlighted_text(
    content: &str,
    ranges: &[(usize, usize)],
    color: Color,
    size: f32,
    theme: &Theme,
) -> AnyElement {
    if ranges.is_empty() {
        return view! { <text size={size} class="truncate" color={color}>{content.to_owned()}</text> };
    }
    let accent = theme.colors.accent;
    let mut spans: Vec<AnyElement> = Vec::new();
    let mut cursor = 0;
    for &(start, end) in ranges {
        if start < cursor
            || end > content.len()
            || !content.is_char_boundary(start)
            || !content.is_char_boundary(end)
        {
            continue;
        }
        if cursor < start {
            spans.push(view! { <text size={size} color={color}>{content[cursor..start].to_owned()}</text> });
        }
        spans.push(view! { <text size={size} class="font-semibold" color={accent}>{content[start..end].to_owned()}</text> });
        cursor = end;
    }
    if cursor < content.len() {
        spans.push(view! { <text size={size} class="truncate" color={color}>{content[cursor..].to_owned()}</text> });
    }
    view! { <div class="flex-row items-center overflow-hidden" min_w={0.0}>{...spans}</div> }
}

fn section_row(
    label: &'static str,
    files: u32,
    action: Option<RowAction>,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let noun = if files == 1 { "file" } else { "files" };
    let caption = TextStyle::Caption.font_size(theme);
    let button = action.map(|a| text_action(&a, theme));
    view! {
        <div
            class="flex-row items-center w-full shrink-0"
            h={theme.metrics.ui_row_height.round()}
            px={tokens::scaled(Sp::SM, scale)}
            gap={tokens::scaled(Sp::XS, scale)}
            key={format!("section:{label}")}
            accessibility_role={AccessibilityRole::Heading}
            aria-label={format!("{label}, {files} {noun}")}
        >
            <text size={caption} class="font-semibold" color={tc.text_muted}>{label.to_uppercase()}</text>
            <text size={caption} color={tc.text_muted}>{files.to_string()}</text>
            <div class="flex-1" />
            {?button}
        </div>
    }
}

fn viewed_row(files: u32, expanded: bool, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let noun = if files == 1 { "file" } else { "files" };
    let caption = TextStyle::Caption.font_size(theme);
    view! {
        <div
            class="flex-row items-center w-full shrink-0"
            h={theme.metrics.ui_row_height.round()}
            px={tokens::scaled(Sp::SM, scale)}
            gap={tokens::scaled(Sp::XS, scale)}
            rounded={theme.metrics.control_radius}
            hover_bg={tc.sidebar_row_hover}
            cursor={CursorHint::Pointer}
            key="section:viewed"
            id="files.viewed"
            test_id="files.viewed"
            role="button"
            aria-label={format!("Viewed, {files} {noun}")}
            aria-expanded={expanded}
            on:click={NavigationAction::ToggleViewedGroup}
        >
            <icon svg={if expanded { lucide::CHEVRON_DOWN } else { lucide::CHEVRON_RIGHT }} size={Ico::XS} color={tc.text_muted} />
            <text size={caption} class="font-semibold" color={tc.text_muted}>"VIEWED"</text>
            <text size={caption} color={tc.text_muted}>{files.to_string()}</text>
        </div>
    }
}

fn text_action(action: &RowAction, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let color = if action.destructive {
        tc.status_error
    } else {
        tc.text_muted
    };
    view! {
        <div
            class="flex-row items-center shrink-0"
            h={theme.metrics.ui_row_height.round() - tokens::scaled(Sp::SM, scale)}
            px={tokens::scaled(Sp::XS, scale)}
            gap={tokens::scaled(Sp::XS, scale)}
            rounded={theme.metrics.control_radius}
            hover_bg={tc.ghost_element_hover}
            cursor={CursorHint::Pointer}
            id={format!("row-action:{}", action.test_id)}
            test_id={action.test_id.clone()}
            role="button"
            aria-label={action.label.clone()}
            on:click={action.action.clone()}
        >
            {?action.icon.map(|svg| view! { <icon svg={svg} size={Ico::XS} color={color} /> })}
            <text size={TextStyle::BodySmall.font_size(theme)} color={color}>{action.label.clone()}</text>
        </div>
    }
}

fn icon_action(action: &RowAction, theme: &Theme) -> AnyElement {
    let Some(svg) = action.icon else {
        return text_action(action, theme);
    };
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let side = theme.metrics.ui_row_height.round() - tokens::scaled(Sp::SM, scale);
    let tooltip = match action.shortcut {
        Some(key) => format!("{} {key}", action.label),
        None => action.label.clone(),
    };
    view! {
        <div
            class="flex-row items-center justify-center shrink-0"
            w={side}
            h={side}
            rounded={theme.metrics.control_radius}
            hover_bg={tc.ghost_element_hover}
            cursor={CursorHint::Pointer}
            id={format!("row-action:{}", action.test_id)}
            test_id={action.test_id.clone()}
            role="button"
            aria-label={action.label.clone()}
            tooltip={tooltip}
            on:click={action.action.clone()}
        >
            <icon svg={svg} size={Ico::XS} color={if action.destructive { tc.status_error } else { tc.icon }} />
        </div>
    }
}

fn row_label(path: &str, file: &ManifestFile, stats: Option<(u32, u32)>) -> String {
    let status = if file.buckets.contains(&ChangeBucket::Untracked) {
        "untracked"
    } else if file.binary {
        "binary"
    } else {
        match file.status {
            FileStatus::Added => "added",
            FileStatus::Deleted => "deleted",
            FileStatus::Modified => "modified",
            FileStatus::Renamed => "renamed",
            FileStatus::Copied => "copied",
        }
    };
    match stats {
        Some((add, del)) => format!("{path}, {status}, +{add} \u{2212}{del}"),
        None => format!("{path}, {status}"),
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

pub(super) use crate::core::ref_catalog::now_secs;

fn history(
    nav: &mut NavigationController,
    review: &ReviewController,
    height: f32,
    focus: Option<FocusId>,
    theme: &Theme,
) -> AnyElement {
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
    let Some(all) = nav.history_commits() else {
        return empty_state("No history", "Open a repository to see commits", theme);
    };
    let query = nav.filter().text().to_owned();
    let commits: Vec<&VcsChange> = all.iter().filter(|c| commit_matches(c, &query)).collect();
    if commits.is_empty() {
        return if query.trim().is_empty() {
            empty_state("No commits", "This comparison has no commits", theme)
        } else {
            empty_state("No matching commits", "Try a different filter", theme)
        };
    }
    let tags = nav.tags_by_target();
    let capabilities = nav.repository().map(|repo| repo.snapshot.capabilities);
    let row_height = tokens::scaled(Layout::HISTORY_ROW_H, scale);
    let window = virtual_list_window(
        commits.len(),
        scroll,
        height,
        row_height,
        0.0,
        OVERSCAN_ROWS,
    );
    let now = now_secs();
    let small = TextStyle::BodySmall.font_size(theme);
    let rows: Vec<AnyElement> = commits[window.range.clone()]
        .iter()
        .map(|commit| {
            let revision = commit.revision.id.clone();
            let is_selected = selected
                .as_deref()
                .is_some_and(|s| revision.starts_with(s) || s == commit.short_revision);
            let mut actions = vec![
                RowAction {
                    label: "Copy SHA".to_owned(),
                    action: ShellAction::CopyText(revision.clone()).into(),
                    destructive: false,
                    icon: Some(lucide::COPY),
                    shortcut: Some("\u{2318}C"),
                    test_id: "history.copy-sha".to_owned(),
                },
                RowAction {
                    label: "Compare with working tree".to_owned(),
                    action: NavigationAction::CompareWithWorkingTree(revision.clone()).into(),
                    destructive: false,
                    icon: Some(lucide::GIT_COMPARE),
                    shortcut: None,
                    test_id: "history.compare-working-tree".to_owned(),
                },
            ];
            if let Some(caps) = capabilities {
                actions.extend(change_actions(commit, caps));
            }
            let menu = row_menu(actions);
            let names = tags.get(&revision).map_or(&[][..], Vec::as_slice);
            let mut pills: Vec<AnyElement> = names
                .iter()
                .take(MAX_TAG_PILLS)
                .map(|name| pill(name, theme))
                .collect();
            if names.len() > MAX_TAG_PILLS {
                pills.push(pill(&format!("+{}", names.len() - MAX_TAG_PILLS), theme));
            }
            let avatar = author_avatar(
                review,
                &commit.author_name,
                commit.author_email.as_deref().unwrap_or_default(),
            )
            .size(Layout::AVATAR_SM);
            let age = relative_age(now, commit.timestamp);
            let meta = [commit.author_name.as_str(), age.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .fold(String::new(), |mut meta, part| {
                    meta.push_str(" \u{00B7} ");
                    meta.push_str(part);
                    meta
                });
            let base = tc.sidebar_background;
            let focused = is_selected && within(focus, Some(&format!("commit:{revision}")));
            let paint = tokens::row_paint(theme, base, is_selected, focused);
            view! {
                <div
                    class="flex-row items-start w-full shrink-0"
                    h={row_height}
                    px={tokens::scaled(Sp::SM, scale)}
                    pt={tokens::scaled(Sp::SM - Sp::XXS, scale)}
                    gap={tokens::scaled(Sp::SM, scale)}
                    rounded={theme.metrics.control_radius}
                    bg={paint.bg}
                    hover_bg={if is_selected { paint.bg } else { tc.sidebar_row_hover }}
                    cursor={CursorHint::Pointer}
                    key={revision.clone()}
                    id={format!("commit:{revision}")}
                    test_id={format!("commit:{revision}")}
                    accessibility_role={AccessibilityRole::ListBoxOption}
                    aria-label={commit.summary.clone()}
                    aria-selected={is_selected}
                    on:click={NavigationAction::SelectHistoryCommit(revision.clone())}
                    on:context_menu={menu}
                >
                    <div class="shrink-0" pt={tokens::scaled(Sp::XXS, scale)}>{avatar}</div>
                    <div class="flex-1 flex-col overflow-hidden" min_w={0.0} gap={tokens::scaled(Sp::XXS, scale)}>
                        <div class="flex-row overflow-hidden" min_w={0.0}>
                            <text size={TextStyle::Body.font_size(theme)} class="truncate font-medium" color={paint.text}>{commit.summary.clone()}</text>
                        </div>
                        <div class="flex-row items-center overflow-hidden" min_w={0.0}>
                            <text size={TextStyle::MonoSmall.font_size(theme)} class="font-mono" color={paint.muted}>{commit.short_revision.clone()}</text>
                            <text size={small} class="truncate" color={paint.muted}>{meta}</text>
                        </div>
                    </div>
                    {...pills}
                </div>
            }
        })
        .collect();
    view! {
        <div
            class="w-full flex-col overflow-hidden scrollbar-auto-hide"
            h={height}
            scroll_y={scroll}
            scroll_total={window.total_extent}
            px={theme.metrics.spacing_sm}
            scrollbar_visibility={nav.history_scrollbar()}
            on:scroll={ScrollActionBuilder::new(|lines| Action::from(NavigationAction::ScrollHistory(lines)).into())}
            focus_ring={SIDEBAR}
            key_context={workspace_context(review.active_pull_request().is_some())}
            on:key:arrowdown={NavigationAction::MoveSelection(1)}
            on:key:arrowup={NavigationAction::MoveSelection(-1)}
            accessibility_role={AccessibilityRole::ListBox}
            aria-label="Commits"
        >
            <div class="w-full shrink-0" h={window.top_spacer} />
            {...rows}
            <div class="w-full shrink-0" h={window.bottom_spacer} />
        </div>
    }
}

pub(super) fn pill(label: &str, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    view! {
        <div
            class="flex-row items-center shrink-0"
            px={tokens::scaled(Sp::XS, scale)}
            py={tokens::scaled(Sp::XXS, scale)}
            rounded={theme.metrics.control_radius * 0.7}
            bg={tc.element_background}
            accessibility_role={AccessibilityRole::Label}
            aria-label={format!("Tag {label}")}
        >
            <text size={TextStyle::Caption.font_size(theme)} class="font-medium" color={tc.text_muted}>{label.to_owned()}</text>
        </div>
    }
}

pub fn viewing_status(nav: &NavigationController, theme: &Theme) -> Option<AnyElement> {
    let selected = nav.history().selected.as_deref()?;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let short = &selected[..selected.len().min(7)];
    Some(view! {
        <div class="flex-row items-center" gap={tokens::scaled(Sp::MD, scale)}>
            <text size={small} color={tc.text_muted}>{format!("Viewing {short}")}</text>
            <div
                class="flex-row items-center"
                gap={tokens::scaled(Sp::XS, scale)}
                px={tokens::scaled(Sp::SM, scale)}
                rounded={theme.metrics.control_radius}
                hover_bg={tc.ghost_element_hover}
                cursor={CursorHint::Pointer}
                id="status.back-to-worktree"
                role="button"
                aria-label="Back to working tree"
                on:click={NavigationAction::ClearHistoryCommit}
            >
                <icon svg={icons::UNDO} size={Ico::XS} color={tc.text_muted} />
                <text size={small} color={tc.text_muted}>"Back to working tree"</text>
                {kbd("esc", theme)}
            </div>
        </div>
    })
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
    let caps = repository.snapshot.capabilities;
    let bar = log
        .iter()
        .enumerate()
        .find(|(_, entry)| selected == Some(entry.operation_id.as_str()))
        .and_then(|(index, entry)| {
            action_bar(&operation_log_actions(entry, index == 0, caps), theme)
        });
    let list_h = (height - if bar.is_some() { row_height } else { 0.0 }).max(0.0);
    let rows: Vec<AnyElement> = log
        .iter()
        .enumerate()
        .take((list_h / row_height.max(1.0)).ceil() as usize + OVERSCAN_ROWS)
        .map(|(index, entry)| {
            let is_selected = selected == Some(entry.operation_id.as_str());
            let menu = row_menu(operation_log_actions(entry, index == 0, caps));
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
                    accessibility_role={AccessibilityRole::ListBoxOption}
                    aria-label={entry.description.clone()}
                    aria-selected={is_selected}
                    on:click={NavigationAction::SelectOperation(entry.operation_id.clone())}
                    on:context_menu={menu}
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
        <div class="w-full flex-col" h={height}>
            {?bar}
            <div
                class="w-full flex-col overflow-hidden"
                h={list_h}
                accessibility_role={AccessibilityRole::ListBox}
                aria-label="Operations"
            >
                {...rows}
            </div>
        </div>
    }
}

fn row_menu(actions: Vec<RowAction>) -> impl Fn(ContextMenuEvent) -> quark_ui::Action + 'static {
    let entries = context_menu(actions);
    move |event| {
        Action::from(CommandsAction::OpenContextMenu {
            entries: entries.clone(),
            x: event.x.round() as i32,
            y: event.y.round() as i32,
        })
        .into()
    }
}

fn action_bar(actions: &[RowAction], theme: &Theme) -> Option<AnyElement> {
    if actions.is_empty() {
        return None;
    }
    let scale = tokens::ui_scale(theme);
    let buttons: Vec<AnyElement> = actions.iter().map(|a| text_action(a, theme)).collect();
    Some(view! {
        <div
            class="flex-row items-center w-full shrink-0 overflow-hidden"
            h={theme.metrics.ui_row_height.round()}
            px={tokens::scaled(Sp::SM, scale)}
            gap={tokens::scaled(Sp::XS, scale)}
            accessibility_role={AccessibilityRole::Toolbar}
            aria-label="Row actions"
        >
            {...buttons}
        </div>
    })
}

pub fn merge_base_empty(nav: &NavigationController, theme: &Theme) -> Option<AnyElement> {
    let (title, detail) = nav.merge_base_hint()?;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let chrome = Chrome::from_theme(theme);
    let direct = render_control(
        Control::new(
            "diff.empty.direct",
            "Compare directly",
            NavigationAction::SetCompareMode(CompareMode::TwoDot),
            Tone::Secondary,
        ),
        &chrome,
    );
    let swap = render_control(
        Control::new(
            "diff.empty.swap",
            "Swap sides",
            NavigationAction::SwapRefs,
            Tone::Ghost,
        )
        .icon(lucide::ARROW_LEFT_RIGHT),
        &chrome,
    );
    Some(view! {
        <div
            class="w-full h-full flex-col items-center justify-center"
            gap={tokens::scaled(Sp::XS, scale)}
            p={tokens::scaled(Sp::LG, scale)}
            bg={tc.editor_surface}
            test_id="diff.empty"
        >
            <text size={TextStyle::Body.font_size(theme)} class="font-medium" color={tc.text}>{title}</text>
            <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{detail}</text>
            <div class="flex-row items-center" gap={tokens::scaled(Sp::SM, scale)} pt={tokens::scaled(Sp::MD, scale)}>
                {direct}
                {swap}
            </div>
        </div>
    })
}
