use quark::view;
use quark_app::ViewContext;
use quark_components::{kbd, kbd_icon};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::Theme;

use super::CommandsController;
use super::keymap;
use super::launcher::{LauncherChoice, LauncherRow, MAX_REPO_ROWS, Section, pull_request_row};
use super::list;
use crate::app::icons;
use crate::app::shell::Slot;
use crate::app::text_targets::LAUNCHER;
use crate::app::tokens::{self, Ico, Layout, Rad, Shadow, Sp, Sz, TextStyle};
use crate::contracts::commands::CommandsAction;
use crate::contracts::shell::Action;
use crate::core::keymap::ShortcutCommand;

const FIXED_ROWS: f32 = 4.0;
const HEADINGS: f32 = 3.0;
const MIN_ROWS_WITH_HEADER: f32 = 3.0;

pub fn launcher(
    commands: &mut CommandsController,
    slot: &Slot,
    signed_in: bool,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let focused = cx.is_focused(LAUNCHER);
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let (width, height) = (slot.width, slot.height);
    let row_h = theme.metrics.ui_row_height.round();
    let head_h = tokens::scaled(Layout::CONTROL_SM_H, scale);
    let input_h = tokens::scaled(Sz::INPUT, scale);
    let pad = tokens::scaled(Sp::XS, scale);
    let foot_h = tokens::scaled(Layout::CONTROL_H, scale) + pad * 2.0;
    let gap = tokens::scaled(Sp::MD, scale);
    let margin = tokens::scaled(Sp::XL, scale);
    let mark = tokens::scaled(Layout::LAUNCHER_MARK, scale);
    let header_h = mark + gap + row_h + gap;
    let fixed = input_h + HEADINGS * head_h + FIXED_ROWS * row_h + pad * 2.0 + foot_h;
    let room = height - margin * 2.0 - fixed;
    let header = room - header_h >= MIN_ROWS_WITH_HEADER * row_h;
    let room = if header { room - header_h } else { room };
    let capacity = ((room / row_h).floor().max(1.0) as usize).min(MAX_REPO_ROWS);
    let total = fixed + capacity as f32 * row_h + if header { header_h } else { 0.0 };
    let top = ((height - total) / 2.0).max(margin.min(height));
    let (_, scanning) = commands.repo_scan_status();
    let saved = commands.settings().settings.keymap_overrides.clone();
    let launcher = commands.launcher_mut()?;
    launcher.set_capacity(capacity);
    let launcher = &*launcher;
    let box_w = tokens::scaled(Layout::LAUNCHER_W, scale)
        .min(width - margin * 2.0)
        .max(1.0);
    let input = text_input("Search repositories", "")
        .field(&launcher.query)
        .placeholder("Search repos, paste a PR link, or type a path")
        .focus_target(LAUNCHER)
        .focused(focused)
        .search(true)
        .bare()
        .w_full()
        .h(input_h);
    let mut children: Vec<AnyElement> = Vec::new();
    let mut section: Option<(Section, Vec<AnyElement>)> = None;
    let mut groups: Vec<AnyElement> = Vec::new();
    for (index, row) in launcher.rows.iter().enumerate() {
        if section
            .as_ref()
            .is_none_or(|(current, _)| *current != row.section)
        {
            if let Some((done, rows)) = section.take() {
                groups.push(group(done, rows));
            }
            let mut rows = Vec::new();
            if row.section != Section::Typed {
                let hint = match row.section {
                    Section::Repositories if scanning => Some("Scanning\u{2026}".to_owned()),
                    Section::Repositories => {
                        Some(launcher.roots.join(" \u{b7} ")).filter(|hint| !hint.is_empty())
                    }
                    _ => None,
                };
                rows.push(heading(row.section, hint, theme));
            }
            section = Some((row.section, rows));
        }
        let selected = index == launcher.selected;
        let element = if let LauncherChoice::Repository(path) = &row.choice {
            let count = launcher.count(path);
            let continues =
                row.nested && launcher.rows.get(index + 1).is_some_and(|next| next.nested);
            repo_row(row, index, selected, count, continues, theme)
        } else {
            launcher_row(row, index, selected, signed_in, &saved, theme)
        };
        if let Some((_, rows)) = section.as_mut() {
            rows.push(element);
        }
    }
    if let Some((done, rows)) = section.take() {
        groups.push(group(done, rows));
    }
    if header {
        children.push(view! {
            <div class="flex-col items-center shrink-0" gap={gap} pb={gap}>
                <div class="flex-row items-center justify-center" w={mark} h={mark} aria-label="Diffy">
                    {?crate::app::brand::logo(Layout::LAUNCHER_MARK)}
                </div>
                <text size={TextStyle::Heading.font_size(theme)} class="font-semibold" color={tc.text_strong}>
                    "What do you want to review?"
                </text>
            </div>
        });
    }
    let shortcuts = keymap::display_binding(&saved, ShortcutCommand::ShowKeymaps);
    let small = TextStyle::BodySmall.font_size(theme);
    children.push(view! {
        <div
            class="flex-col overflow-hidden shrink-0"
            w={box_w}
            bg={tc.elevated_surface}
            border={tc.border}
            rounded={tokens::scaled(Rad::XXL, scale)}
            shadow_preset={Shadow::POPOVER}
            test_id="launcher.box"
        >
            <div
                class="flex-row items-center"
                gap={tokens::scaled(Sp::SM, scale)}
                px={tokens::scaled(Sp::MD, scale)}
                border_b={tc.border_variant}
            >
                <icon svg={lucide::SEARCH} size={Ico::SM} color={tc.icon} />
                <div class="flex-1 min-w-0">{input}</div>
            </div>
            <div
                class="flex-col w-full"
                p={pad}
                accessibility_role={AccessibilityRole::ListBox}
                aria-label="Results"
                test_id="launcher.results"
            >
                {...groups}
            </div>
            <div
                class="flex-row items-center w-full"
                h={foot_h}
                px={tokens::scaled(Sp::MD, scale)}
                gap={tokens::scaled(Sp::MD, scale)}
                border_t={tc.border_variant}
                bg={tc.panel}
                test_id="launcher.footer"
            >
                <div class="flex-row items-center" gap={tokens::scaled(Sp::XS, scale)}>
                    {kbd_icon(lucide::ARROW_UP, theme)}
                    {kbd_icon(lucide::ARROW_DOWN, theme)}
                    <text size={small} color={tc.text_muted}>"move"</text>
                </div>
                <div class="flex-row items-center" gap={tokens::scaled(Sp::XS, scale)}>
                    {kbd_icon(icons::CORNER_DOWN_LEFT, theme)}
                    <text size={small} color={tc.text_muted}>"open"</text>
                </div>
                <div class="flex-1" />
                {?shortcuts.map(|binding| view! {
                    <div class="flex-row items-center" gap={tokens::scaled(Sp::XS, scale)}>
                        {kbd(binding, theme)}
                        <text size={small} color={tc.text_muted}>"shortcuts"</text>
                    </div>
                })}
            </div>
        </div>
    });
    Some(view! {
        <div
            w={width}
            h={height}
            class="flex-col items-center overflow-hidden"
            pt={top}
            bg={tc.editor_surface}
            accessibility_role={AccessibilityRole::Main}
            aria-label="Launcher"
            test_id="launcher"
        >
            {...children}
        </div>
    })
}

fn group(section: Section, rows: Vec<AnyElement>) -> AnyElement {
    let id = section.title().to_lowercase().replace(' ', "-");
    view! {
        <div
            class="flex-col w-full"
            accessibility_role={AccessibilityRole::Group}
            aria-label={section.title()}
            test_id={format!("launcher.section.{id}")}
        >
            {...rows}
        </div>
    }
}

fn heading(section: Section, hint: Option<String>, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let caption = TextStyle::Caption.font_size(theme);
    view! {
        <div
            class="w-full flex-row items-end"
            h={tokens::scaled(Layout::CONTROL_SM_H, scale)}
            px={tokens::scaled(Sp::SM, scale)}
            pb={tokens::scaled(Sp::XXS, scale)}
            gap={tokens::scaled(Sp::SM, scale)}
            accessibility_role={AccessibilityRole::Heading}
            aria-label={section.title()}
        >
            <text size={caption} class="font-semibold" color={tc.text_muted}>{section.title()}</text>
            <div class="flex-1" />
            {?hint.map(|hint| view! {
                <text size={caption} class="truncate" color={tc.placeholder}>{hint}</text>
            })}
        </div>
    }
}

fn repo_row(
    row: &LauncherRow,
    index: usize,
    selected: bool,
    count: Option<Option<usize>>,
    continues: bool,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let mono = TextStyle::MonoSmall.font_size(theme);
    let paint = tokens::row_paint(theme, tc.elevated_surface, selected, true);
    let row_h = theme.metrics.ui_row_height.round();
    let gap = tokens::scaled(Sp::SM, scale);
    let changed = count.flatten().filter(|count| *count > 0);
    let icon = if row.worktree {
        lucide::GIT_BRANCH
    } else {
        lucide::FOLDER_GIT
    };
    let icon_color = if row.worktree && changed.is_some() {
        tc.status_warning
    } else {
        paint.icon
    };
    let guide = row.nested.then(|| {
        let hairline = tokens::scaled(1.0, scale).max(1.0);
        let inset = tokens::scaled(Sp::XXS, scale);
        let mid = Ico::SM / 2.0;
        let above = (row_h - Ico::SM) / 2.0 - inset;
        let down = if continues { row_h } else { row_h / 2.0 };
        view! {
            <div class="shrink-0" w={Ico::SM} h={row_h}>
                <div class="absolute" left={mid} top={-above} w={hairline} h={above + down} bg={tc.border} />
                <div class="absolute" left={mid} top={row_h / 2.0} w={mid + gap - inset} h={hairline} bg={tc.border} />
            </div>
        }
    });
    let id = format!("launcher-row:{index}:{}", row.label);
    let click: quark_ui::Action = Action::from(CommandsAction::LauncherSelect(index)).into();
    view! {
        <div
            class="w-full flex-row items-center shrink-0"
            h={row_h}
            px={tokens::scaled(Sp::SM, scale)}
            gap={gap}
            rounded={theme.metrics.control_radius}
            bg={paint.bg}
            @when {!selected} { hover_bg={tc.ghost_element_hover} }
            cursor={CursorHint::Pointer}
            id={id.clone()}
            key={id}
            accessibility_role={AccessibilityRole::ListBoxOption}
            aria-label={row.label.clone()}
            aria-selected={selected}
            on:click={click}
        >
            {?guide}
            <icon svg={icon} size={Ico::SM} color={icon_color} />
            <div class="flex-1 flex-row items-center overflow-hidden" min_w={0.0} gap={gap}>
                <div class="flex-row items-center shrink-0">
                    {list::highlighted(&row.label, &row.highlights, paint.text, theme)}
                </div>
                {?row.detail.clone().map(|detail| view! {
                    <div class="flex-row overflow-hidden" min_w={0.0}>
                        <text size={mono} class="font-mono truncate" color={tc.placeholder}>{detail}</text>
                    </div>
                })}
            </div>
            <div class="flex-row items-center overflow-hidden shrink-0" w={tokens::scaled(Layout::BRANCH_MAX_W, scale)}>
                {?row.branch.as_ref().map(|branch| view! {
                    <div
                        class="flex-row items-center overflow-hidden"
                        px={tokens::scaled(Sp::XS + Sp::XXS, scale)}
                        rounded={tokens::scaled(Rad::SM, scale)}
                        bg={tc.element_background}
                    >
                        <text size={mono} class="font-mono truncate" color={tc.text}>{branch.clone()}</text>
                    </div>
                })}
            </div>
            <div class="flex-row items-center justify-end shrink-0" w={tokens::scaled(Layout::CHANGES_W, scale)}>
                {?changed.map(|count| view! {
                    <text size={small} color={tc.status_warning}>{format!("{count} changed")}</text>
                })}
            </div>
            <div class="flex-row items-center justify-end shrink-0" w={tokens::scaled(Layout::CONTROL_SM_H, scale)}>
                if selected {
                    {kbd_icon(icons::CORNER_DOWN_LEFT, theme)}
                }
            </div>
        </div>
    }
}

fn launcher_row(
    row: &LauncherRow,
    index: usize,
    selected: bool,
    signed_in: bool,
    saved: &[crate::core::keymap::KeymapOverride],
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let mono = TextStyle::MonoSmall.font_size(theme);
    let paint = tokens::row_paint(theme, tc.elevated_surface, selected, true);
    let (label, detail) = match row.choice {
        LauncherChoice::PullRequests => {
            let (label, detail) = pull_request_row(signed_in);
            (label.to_owned(), Some(detail.to_owned()))
        }
        _ => (row.label.clone(), row.detail.clone()),
    };
    let icon = match row.choice {
        LauncherChoice::Repository(_) => lucide::FOLDER_GIT,
        LauncherChoice::PullRequest { .. } => lucide::GIT_PULL_REQUEST,
        LauncherChoice::Path(_) => lucide::ARROW_RIGHT,
        LauncherChoice::PullRequests => lucide::GITHUB_MARK,
        LauncherChoice::OpenFolder => lucide::FOLDER_OPEN,
        LauncherChoice::Clone(_) => icons::DOWNLOAD,
    };
    let mono_detail = matches!(
        row.choice,
        LauncherChoice::Repository(_)
            | LauncherChoice::Path(_)
            | LauncherChoice::PullRequest { .. }
    );
    let binding = matches!(row.choice, LauncherChoice::OpenFolder)
        .then(|| keymap::display_binding(saved, ShortcutCommand::OpenRepository))
        .flatten();
    let id = format!("launcher-row:{index}:{label}");
    let click: quark_ui::Action = Action::from(CommandsAction::LauncherSelect(index)).into();
    view! {
        <div
            class="w-full flex-row items-center shrink-0"
            h={theme.metrics.ui_row_height.round()}
            px={tokens::scaled(Sp::SM, scale)}
            gap={tokens::scaled(Sp::SM, scale)}
            rounded={theme.metrics.control_radius}
            bg={paint.bg}
            @when {!selected} { hover_bg={tc.ghost_element_hover} }
            cursor={CursorHint::Pointer}
            id={id.clone()}
            key={id}
            accessibility_role={AccessibilityRole::ListBoxOption}
            aria-label={label.clone()}
            aria-selected={selected}
            on:click={click}
        >
            <icon svg={icon} size={Ico::SM} color={paint.icon} />
            {list::highlighted(&label, &row.highlights, paint.text, theme)}
            {?row.branch.as_ref().map(|branch| view! {
                <div
                    class="flex-row items-center overflow-hidden shrink-0"
                    max_w={tokens::scaled(Layout::BRANCH_MAX_W, scale)}
                    px={tokens::scaled(Sp::XS + Sp::XXS, scale)}
                    rounded={tokens::scaled(Rad::SM, scale)}
                    bg={tc.element_background}
                >
                    <text size={mono} class="font-mono truncate" color={tc.text}>{branch.clone()}</text>
                </div>
            })}
            <div class="flex-1" min_w={0.0} />
            {?detail.map(|detail| if mono_detail {
                view! {
                    <div class="flex-row overflow-hidden" min_w={0.0} max_w={tokens::scaled(Layout::LAUNCHER_PATH_MAX_W, scale)}>
                        <text size={mono} class="font-mono truncate" color={tc.placeholder}>{detail}</text>
                    </div>
                }
            } else {
                view! { <text size={small} class="truncate" color={paint.muted}>{detail}</text> }
            })}
            {?binding.map(|binding| kbd(binding, theme))}
            if selected {
                {kbd_icon(icons::CORNER_DOWN_LEFT, theme)}
            }
        </div>
    }
}
