use quark::view;
use quark_app::ViewContext;
use quark_components::{
    Button, ButtonSize, ButtonStyle, SegmentedControl, SegmentedItem, kbd, slider, switch,
};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme, ThemeMode as QuarkMode};

use super::CommandsController;
use super::keymap::{self, KEYMAP};
use super::state::SettingsPage;
use crate::app::icons;
use crate::app::shell::Slot;
use crate::app::text_targets::{SETTINGS, SETTINGS_SEARCH};
use crate::app::tokens::{self, Ico, Layout, Rad, Sp, Sz, TextStyle};
use crate::contracts::commands::{CommandsAction, SettingsSection};
use crate::contracts::shell::Action;
use crate::core::compare::{LayoutMode, RendererKind};
use crate::core::theme_types::ThemeMode;
use crate::fonts::font_selection_label;
use crate::platform::persistence::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Theme,
    Mode,
    Size,
    UiFont,
    CodeFont,
    Engine,
    Layout,
    Wrap,
    WrapColumn,
    Continuous,
    ScrollSpeed,
    AutoUpdate,
    RepoRoots,
    RepoDepth,
    RepoHidden,
    RepoScan,
}

struct Item {
    section: SettingsSection,
    control: Control,
    title: &'static str,
    description: &'static str,
}

const ITEMS: &[Item] = &[
    Item {
        section: SettingsSection::Appearance,
        control: Control::Theme,
        title: "Theme",
        description: "Colors for the whole window, searchable",
    },
    Item {
        section: SettingsSection::Appearance,
        control: Control::Mode,
        title: "Appearance",
        description: "Follow macOS or pick one",
    },
    Item {
        section: SettingsSection::Appearance,
        control: Control::Size,
        title: "Interface size",
        description: "Scales everything except the code font",
    },
    Item {
        section: SettingsSection::Appearance,
        control: Control::UiFont,
        title: "Interface font",
        description: "Labels, menus and the sidebar",
    },
    Item {
        section: SettingsSection::Appearance,
        control: Control::CodeFont,
        title: "Code font",
        description: "Diffs and editors",
    },
    Item {
        section: SettingsSection::Editor,
        control: Control::Engine,
        title: "Engine",
        description: "Git's line diff or Difftastic's syntax-aware diff",
    },
    Item {
        section: SettingsSection::Editor,
        control: Control::Layout,
        title: "Layout",
        description: "Default layout for new diffs",
    },
    Item {
        section: SettingsSection::Editor,
        control: Control::Wrap,
        title: "Wrap lines",
        description: "Wrap long lines instead of scrolling sideways",
    },
    Item {
        section: SettingsSection::Editor,
        control: Control::WrapColumn,
        title: "Wrap column",
        description: "Auto wraps at the edge of the diff",
    },
    Item {
        section: SettingsSection::Editor,
        control: Control::Continuous,
        title: "Continuous scroll",
        description: "Show every file in one scrolling view",
    },
    Item {
        section: SettingsSection::Editor,
        control: Control::ScrollSpeed,
        title: "Scroll speed",
        description: "Lines per mouse wheel step",
    },
    Item {
        section: SettingsSection::Behavior,
        control: Control::AutoUpdate,
        title: "Automatic updates",
        description: "Check for new versions in the background",
    },
    Item {
        section: SettingsSection::Behavior,
        control: Control::RepoRoots,
        title: "Find repositories in",
        description: "Searched in the background and when the repository picker opens",
    },
    Item {
        section: SettingsSection::Behavior,
        control: Control::RepoDepth,
        title: "Search depth",
        description: "Folder levels below each location",
    },
    Item {
        section: SettingsSection::Behavior,
        control: Control::RepoHidden,
        title: "Include hidden folders",
        description: "Dotfiles, plugin checkouts and caches",
    },
    Item {
        section: SettingsSection::Behavior,
        control: Control::RepoScan,
        title: "Last repository scan",
        description: "Skips Library, node_modules and target folders",
    },
];

const REPO_DEPTHS: [u8; 3] = [3, 5, 7];

const SIZE_STOPS: [u16; 5] = [80, 100, 125, 150, 180];
const WRAP_PRESETS: [(u32, &str); 4] = [(0, "Auto"), (80, "80"), (100, "100"), (120, "120")];
const MAX_SCROLL_LINES: u8 = 20;

fn section_icon(section: SettingsSection) -> &'static str {
    match section {
        SettingsSection::Appearance => icons::PALETTE,
        SettingsSection::Editor => lucide::FILE_DIFF,
        SettingsSection::Behavior => lucide::SETTINGS,
        SettingsSection::Keymaps => icons::KEYBOARD,
        SettingsSection::Clankers => lucide::SPARKLES,
        SettingsSection::Accounts => icons::USER,
        SettingsSection::Updates => icons::DOWNLOAD,
    }
}

pub fn sheet_size(theme: &Theme, size: (f32, f32)) -> (f32, f32) {
    let scale = tokens::ui_scale(theme);
    let margin = tokens::scaled(Sz::MODAL_MARGIN, scale);
    (
        tokens::scaled(Layout::SETTINGS_SHEET_W, scale)
            .min(size.0 - margin)
            .max(1.0),
        tokens::scaled(Layout::SETTINGS_SHEET_H, scale)
            .min(size.1 - theme.metrics.title_bar_height - margin)
            .max(1.0),
    )
}

pub fn content_width(theme: &Theme, size: (f32, f32), section: SettingsSection) -> f32 {
    let scale = tokens::ui_scale(theme);
    let (w, _) = sheet_size(theme, size);
    let preview = if section == SettingsSection::Appearance && w >= preview_min_width(theme) {
        tokens::scaled(Layout::SETTINGS_PREVIEW_W, scale)
    } else {
        0.0
    };
    (w - tokens::scaled(Layout::SETTINGS_SHEET_NAV_W, scale)
        - preview
        - tokens::scaled(Sp::XL, scale) * 2.0)
        .max(0.0)
}

fn preview_min_width(theme: &Theme) -> f32 {
    tokens::scaled(Layout::SETTINGS_SHEET_W, tokens::ui_scale(theme)) * 0.85
}

pub fn settings_slot(
    commands: &CommandsController,
    slot: &Slot,
    theme: &Theme,
) -> Option<(SettingsSection, f32)> {
    let page = commands.settings_page()?;
    if !page.search.text().trim().is_empty() {
        return None;
    }
    matches!(
        page.section,
        SettingsSection::Clankers | SettingsSection::Accounts | SettingsSection::Updates
    )
    .then(|| {
        (
            page.section,
            content_width(theme, (slot.width, slot.height), page.section),
        )
    })
}

pub fn sheet(
    commands: &CommandsController,
    size: (f32, f32),
    external: Option<AnyElement>,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let page = commands.settings_page()?;
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let settings = &commands.settings().settings;
    let search_focused = cx.is_focused(SETTINGS_SEARCH);
    let (w, h) = sheet_size(theme, size);
    let top = theme.metrics.title_bar_height
        + ((size.1 - theme.metrics.title_bar_height - h) / 2.0).max(0.0);
    let left = ((size.0 - w) / 2.0).max(0.0);
    let nav_w = tokens::scaled(Layout::SETTINGS_SHEET_NAV_W, scale);
    let query = page.search.text().trim().to_lowercase();
    let search = text_input("Search settings", "")
        .field(&page.search)
        .placeholder("Search settings")
        .focus_target(SETTINGS_SEARCH)
        .focused(search_focused)
        .search(true)
        .bare()
        .w_full()
        .h((theme.metrics.ui_small_font_size * 1.5).round());
    let nav: Vec<AnyElement> = SettingsSection::ALL
        .iter()
        .map(|section| {
            nav_item(
                *section,
                query.is_empty() && page.section == *section,
                theme,
            )
        })
        .collect();
    let (title, description, body) = if query.is_empty() {
        (
            page.section.label().to_owned(),
            page.section.description().to_owned(),
            section_body(page, settings, commands, external, theme),
        )
    } else {
        (
            "Search".to_owned(),
            format!(
                "Settings matching \u{201C}{}\u{201D}",
                page.search.text().trim()
            ),
            search_body(&query, page, settings, commands, theme),
        )
    };
    let preview = (query.is_empty()
        && page.section == SettingsSection::Appearance
        && w >= preview_min_width(theme))
    .then(|| preview(settings, theme));
    Some(view! {
        <div
            class="absolute top-0 left-0"
            w={size.0}
            h={size.1}
            z_index={200}
            test_id="settings-backdrop"
        >
            <div
                class="absolute left-0 w-full"
                top={theme.metrics.title_bar_height}
                h={(size.1 - theme.metrics.title_bar_height).max(0.0)}
                bg={tc.overlay_scrim}
                on:click={CommandsAction::CloseSettings}
                block_mouse
            />
            <div
                class="absolute flex-row overflow-hidden"
                left={left}
                top={top}
                w={w}
                h={h}
                bg={tc.elevated_surface}
                border={tc.border}
                rounded={tokens::scaled(Rad::XL, scale)}
                on:click={NoopAction}
                key_context={keymap::SETTINGS_CONTEXT}
                focus_ring={SETTINGS}
                focus_scope="diffy.settings"
                accessibility_role={AccessibilityRole::Dialog}
                aria-label="Settings"
                test_id="settings"
            >
                <div
                    class="flex-col shrink-0 h-full"
                    w={nav_w}
                    gap={tokens::scaled(Sp::XXS, scale)}
                    p={tokens::scaled(Sp::SM, scale)}
                    bg={tc.sidebar_background}
                    border_r={tc.border_variant}
                    accessibility_role={AccessibilityRole::TabList}
                    aria-label="Settings sections"
                >
                    <div class="flex-row items-center w-full" pb={tokens::scaled(Sp::SM, scale)} gap={tokens::scaled(Sp::XS, scale)}>
                        <div
                            class="flex-1 flex-row items-center"
                            gap={tokens::scaled(Sp::XS, scale)}
                            px={tokens::scaled(Sp::SM, scale)}
                            h={theme.metrics.ui_row_height}
                            rounded={theme.metrics.control_radius}
                            bg={tc.element_background}
                            border={if search_focused { tc.focus_border } else { tc.border_variant }}
                        >
                            <icon svg={lucide::SEARCH} size={Ico::XS} color={tc.text_muted} />
                            <div class="flex-1" min_w={0.0}>{search}</div>
                        </div>
                    </div>
                    {...nav}
                </div>
                <div
                    class="flex-1 flex-col h-full"
                    min_w={0.0}
                    p={tokens::scaled(Sp::XL, scale)}
                    gap={tokens::scaled(Sp::MD, scale)}
                >
                    <div class="flex-row items-start w-full" gap={tokens::scaled(Sp::MD, scale)}>
                        <div class="flex-1 flex-col" min_w={0.0} gap={tokens::scaled(Sp::XXS, scale)}>
                            <text size={TextStyle::HeadingLg.font_size(theme)} class="font-semibold" color={tc.text_strong}>{title}</text>
                            <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{description}</text>
                        </div>
                        <div
                            class="flex-row items-center justify-center shrink-0"
                            w={theme.metrics.ui_row_height}
                            h={theme.metrics.ui_row_height}
                            rounded={theme.metrics.control_radius}
                            hover_bg={tc.ghost_element_hover}
                            cursor={CursorHint::Pointer}
                            id="settings.close"
                            role="button"
                            aria-label="Close settings"
                            on:click={CommandsAction::CloseSettings}
                        >
                            <icon svg={lucide::X} size={Ico::SM} color={tc.icon} />
                        </div>
                    </div>
                    <div
                        class="flex-col w-full flex-1 overflow-y-scroll"
                        min_h={0.0}
                        track_scroll={&page.scroll}
                        accessibility_role={AccessibilityRole::Group}
                        aria-label={format!("{} settings", page.section.label())}
                        test_id="settings.content"
                    >
                        {body}
                    </div>
                </div>
                {?preview}
            </div>
        </div>
    })
}

fn nav_item(section: SettingsSection, selected: bool, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    view! {
        <div
            class="flex-row items-center w-full"
            h={theme.metrics.ui_row_height}
            px={tokens::scaled(Sp::SM, scale)}
            gap={tokens::scaled(Sp::SM, scale)}
            rounded={theme.metrics.control_radius}
            bg={if selected { tc.sidebar_row_selected } else { Color::TRANSPARENT }}
            hover_bg={if selected { tc.sidebar_row_selected } else { tc.sidebar_row_hover }}
            cursor={CursorHint::Pointer}
            id={format!("settings.section.{}", section.label())}
            accessibility_role={AccessibilityRole::Tab}
            aria-label={section.label()}
            aria-selected={selected}
            on:click={CommandsAction::SetSettingsSection(section)}
        >
            <icon svg={section_icon(section)} size={Ico::SM} color={if selected { tc.text_strong } else { tc.text_muted }} />
            <text size={TextStyle::Body.font_size(theme)} color={if selected { tc.text_strong } else { tc.text_muted }}>
                {section.label()}
            </text>
        </div>
    }
}

fn card(rows: Vec<AnyElement>, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    view! {
        <div
            class="flex-col w-full shrink-0 overflow-hidden"
            bg={tc.editor_surface}
            border={tc.border_variant}
            rounded={tokens::scaled(Rad::XL, scale)}
        >
            {...rows}
        </div>
    }
}

fn setting_row(item: &Item, control: AnyElement, last: bool, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let stacked = item.control == Control::Size;
    view! {
        <div
            class="w-full"
            @when {stacked} { class="flex-col" }
            @when {!stacked} { class="flex-row items-center" }
            gap={tokens::scaled(if stacked { Sp::SM } else { Sp::LG }, scale)}
            px={tokens::scaled(Sp::MD, scale)}
            py={tokens::scaled(Sp::MD, scale)}
            @when {!last} { border_b={tc.border_variant} }
            accessibility_role={AccessibilityRole::Group}
            aria-label={item.title}
        >
            <div class="flex-1 flex-col" min_w={0.0} gap={tokens::scaled(Sp::XXS, scale)}>
                <text size={TextStyle::Body.font_size(theme)} class="font-medium" color={tc.text}>{item.title}</text>
                <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{item.description}</text>
            </div>
            {control}
        </div>
    }
}

fn rows_for<'a>(
    items: impl Iterator<Item = &'a Item>,
    settings: &Settings,
    commands: &CommandsController,
    theme: &Theme,
) -> Vec<AnyElement> {
    let items: Vec<&Item> = items.collect();
    let count = items.len();
    items
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            setting_row(
                item,
                control(item.control, settings, commands, theme),
                i + 1 == count,
                theme,
            )
        })
        .collect()
}

fn section_body(
    page: &SettingsPage,
    settings: &Settings,
    commands: &CommandsController,
    external: Option<AnyElement>,
    theme: &Theme,
) -> AnyElement {
    let scale = tokens::ui_scale(theme);
    match page.section {
        SettingsSection::Keymaps => keyboard(settings, page, "", theme),
        SettingsSection::Clankers | SettingsSection::Accounts | SettingsSection::Updates => {
            external.unwrap_or_else(|| {
                crate::app::shell::empty_state(
                    page.section.label(),
                    page.section.description(),
                    theme,
                )
            })
        }
        section => {
            let rows = rows_for(
                ITEMS.iter().filter(|i| i.section == section),
                settings,
                commands,
                theme,
            );
            view! { <div class="flex-col w-full" gap={tokens::scaled(Sp::MD, scale)}>{card(rows, theme)}</div> }
        }
    }
}

fn search_body(
    query: &str,
    page: &SettingsPage,
    settings: &Settings,
    commands: &CommandsController,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let mut blocks: Vec<AnyElement> = Vec::new();
    for section in SettingsSection::ALL {
        let matching: Vec<&Item> = ITEMS
            .iter()
            .filter(|i| i.section == section && matches(query, i.title, i.description))
            .collect();
        if matching.is_empty() {
            continue;
        }
        blocks.push(view! {
            <text size={TextStyle::Caption.font_size(theme)} class="font-semibold" color={tc.text_muted}>{section.label().to_uppercase()}</text>
        });
        blocks.push(card(
            rows_for(matching.into_iter(), settings, commands, theme),
            theme,
        ));
    }
    let keys = KEYMAP
        .entries()
        .any(|entry| matches(query, entry.description, ""));
    if keys {
        blocks.push(view! {
            <text size={TextStyle::Caption.font_size(theme)} class="font-semibold" color={tc.text_muted}>"KEYBOARD"</text>
        });
        blocks.push(keyboard(settings, page, query, theme));
    }
    if blocks.is_empty() {
        return crate::app::shell::empty_state(
            "No matching settings",
            "Try a different word",
            theme,
        );
    }
    view! { <div class="flex-col w-full" gap={tokens::scaled(Sp::SM, scale)}>{...blocks}</div> }
}

fn matches(query: &str, title: &str, description: &str) -> bool {
    title.to_lowercase().contains(query) || description.to_lowercase().contains(query)
}

fn control(
    control: Control,
    settings: &Settings,
    commands: &CommandsController,
    theme: &Theme,
) -> AnyElement {
    let scale = tokens::ui_scale(theme);
    match control {
        Control::Theme => theme_button(settings, theme),
        Control::Mode => choice(
            "settings.mode",
            &[
                (
                    "System",
                    CommandsAction::SetThemeMode(ThemeMode::System),
                    settings.theme_mode == ThemeMode::System,
                ),
                (
                    "Dark",
                    CommandsAction::SetThemeMode(ThemeMode::Dark),
                    settings.theme_mode == ThemeMode::Dark,
                ),
                (
                    "Light",
                    CommandsAction::SetThemeMode(ThemeMode::Light),
                    settings.theme_mode == ThemeMode::Light,
                ),
            ],
        ),
        Control::Size => size_slider(settings, theme),
        Control::UiFont => select_button(
            "Interface font",
            &font_selection_label(&settings.fonts.ui_family),
            CommandsAction::OpenUiFontPicker,
            theme,
        ),
        Control::CodeFont => select_button(
            "Code font",
            &font_selection_label(&settings.fonts.mono_family),
            CommandsAction::OpenMonoFontPicker,
            theme,
        ),
        Control::Engine => {
            let renderer = settings
                .last_compare
                .as_ref()
                .map(|c| c.renderer)
                .unwrap_or_default();
            choice(
                "settings.engine",
                &[
                    (
                        "Git",
                        CommandsAction::SetEngine(RendererKind::Builtin),
                        renderer == RendererKind::Builtin,
                    ),
                    (
                        "Difftastic",
                        CommandsAction::SetEngine(RendererKind::Difftastic),
                        renderer == RendererKind::Difftastic,
                    ),
                ],
            )
        }
        Control::Layout => {
            let layout = settings.viewport.layout;
            choice(
                "settings.layout",
                &[
                    (
                        "Unified",
                        CommandsAction::SetLayout(LayoutMode::Unified),
                        layout == LayoutMode::Unified,
                    ),
                    (
                        "Split",
                        CommandsAction::SetLayout(LayoutMode::Split),
                        layout == LayoutMode::Split,
                    ),
                ],
            )
        }
        Control::Wrap => toggle(
            "Wrap lines",
            settings.viewport.wrap_enabled,
            CommandsAction::ToggleWrap,
        ),
        Control::WrapColumn => {
            let column = settings.viewport.wrap_column;
            let items: Vec<(&str, CommandsAction, bool)> = WRAP_PRESETS
                .iter()
                .map(|(value, label)| {
                    (
                        *label,
                        CommandsAction::SetWrapColumn(*value),
                        column == *value,
                    )
                })
                .collect();
            choice("settings.wrap-column", &items)
        }
        Control::Continuous => toggle(
            "Continuous scroll",
            settings.continuous_scroll,
            CommandsAction::ToggleContinuousScroll,
        ),
        Control::ScrollSpeed => {
            let lines = settings.wheel_scroll_lines;
            let slider = slider(
                "settings.scroll-speed",
                "Scroll speed",
                f32::from(lines),
                1.0,
                f32::from(MAX_SCROLL_LINES),
                |v| Action::from(CommandsAction::SetWheelScrollLines(v.round() as u8)).into(),
            )
            .width(tokens::scaled(Layout::SETTINGS_SLIDER_W, scale));
            view! { <div class="shrink-0">{slider}</div> }
        }
        Control::AutoUpdate => toggle(
            "Automatic updates",
            settings.auto_update,
            CommandsAction::ToggleAutoUpdate,
        ),
        Control::RepoRoots => repo_roots(&settings.repo_discovery.roots, theme),
        Control::RepoDepth => {
            let depth = settings.repo_discovery.depth;
            let labels: Vec<String> = REPO_DEPTHS.iter().map(u8::to_string).collect();
            let items: Vec<(&str, CommandsAction, bool)> = REPO_DEPTHS
                .iter()
                .zip(&labels)
                .map(|(value, label)| {
                    (
                        label.as_str(),
                        CommandsAction::SetRepoScanDepth(*value),
                        depth == *value,
                    )
                })
                .collect();
            choice("settings.repo-depth", &items)
        }
        Control::RepoHidden => toggle(
            "Include hidden folders",
            settings.repo_discovery.hidden,
            CommandsAction::ToggleRepoScanHidden,
        ),
        Control::RepoScan => repo_scan_status(commands, theme),
    }
}

fn repo_roots(roots: &[String], theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let height = tokens::scaled(Layout::CONTROL_H, scale);
    let chips: Vec<AnyElement> = roots
        .iter()
        .map(|root| {
            view! {
                <div
                    class="flex-row items-center shrink-0"
                    h={height}
                    pl={tokens::scaled(Sp::SM, scale)}
                    pr={tokens::scaled(Sp::XXS, scale)}
                    gap={tokens::scaled(Sp::XXS, scale)}
                    rounded={theme.metrics.control_radius}
                    border={tc.border_variant}
                    accessibility_role={AccessibilityRole::Group}
                    aria-label={root.clone()}
                >
                    <text size={TextStyle::MonoSmall.font_size(theme)} class="font-mono" color={tc.text}>{root.clone()}</text>
                    <div
                        class="flex-row items-center justify-center"
                        w={tokens::scaled(Layout::CONTROL_SM_H, scale) - tokens::scaled(Sp::XS, scale)}
                        h={tokens::scaled(Layout::CONTROL_SM_H, scale) - tokens::scaled(Sp::XS, scale)}
                        rounded={theme.metrics.control_radius}
                        hover_bg={tc.ghost_element_hover}
                        cursor={CursorHint::Pointer}
                        role="button"
                        aria-label={format!("Remove {root}")}
                        on:click={CommandsAction::RemoveRepoScanRoot(root.clone())}
                    >
                        <icon svg={lucide::X} size={Ico::XS} color={tc.text_muted} />
                    </div>
                </div>
            }
        })
        .collect();
    view! {
        <div class="flex-row items-center shrink-0" gap={tokens::scaled(Sp::SM - Sp::XXS, scale)}>
            {...chips}
            <div
                class="flex-row items-center shrink-0"
                h={height}
                px={tokens::scaled(Sp::SM, scale)}
                gap={tokens::scaled(Sp::XS, scale)}
                rounded={theme.metrics.control_radius}
                bg={tc.element_background}
                border={tc.border_variant}
                hover_bg={tc.element_hover}
                cursor={CursorHint::Pointer}
                id="settings.repo-add-folder"
                role="button"
                aria-label="Add folder"
                on:click={CommandsAction::ChooseRepoScanFolder}
            >
                <icon svg={lucide::PLUS} size={Ico::XS} color={tc.icon} />
                <text size={TextStyle::BodySmall.font_size(theme)} class="font-medium" color={tc.text}>"Add folder\u{2026}"</text>
            </div>
        </div>
    }
}

fn repo_scan_status(commands: &CommandsController, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let (scan, scanning) = commands.repo_scan_status();
    let summary = match (scan, scanning) {
        (_, true) => "Scanning\u{2026}".to_owned(),
        (Some(scan), false) => {
            let now = crate::core::ref_catalog::now_secs();
            let age = crate::core::ref_catalog::relative_age(now, scan.scanned_at);
            let count = scan.repos.len();
            let noun = if count == 1 {
                "repository"
            } else {
                "repositories"
            };
            let ago = if age == "now" {
                "just now".to_owned()
            } else {
                format!("{age} ago")
            };
            format!(
                "{count} {noun} in {:.1} s, {ago}",
                scan.millis as f32 / 1000.0
            )
        }
        (None, false) => "Not scanned yet".to_owned(),
    };
    view! {
        <div class="flex-row items-center shrink-0" gap={tokens::scaled(Sp::MD, scale)}>
            <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{summary}</text>
            <div
                class="flex-row items-center shrink-0"
                h={tokens::scaled(Layout::CONTROL_H, scale)}
                px={tokens::scaled(Sp::SM, scale)}
                gap={tokens::scaled(Sp::XS, scale)}
                rounded={theme.metrics.control_radius}
                bg={tc.element_background}
                border={tc.border_variant}
                hover_bg={tc.element_hover}
                cursor={CursorHint::Pointer}
                id="settings.repo-rescan"
                role="button"
                aria-label="Rescan"
                on:click={CommandsAction::RescanRepositories}
            >
                <icon svg={lucide::REFRESH} size={Ico::XS} color={tc.icon} />
                <text size={TextStyle::BodySmall.font_size(theme)} class="font-medium" color={tc.text}>"Rescan"</text>
            </div>
        </div>
    }
}

fn choice(id: &str, items: &[(&str, CommandsAction, bool)]) -> AnyElement {
    let segments = items
        .iter()
        .map(|(label, action, on)| SegmentedItem::new(*label, Action::from(action.clone()), *on))
        .collect();
    SegmentedControl::new(segments).id(id).into_any()
}

fn toggle(label: &str, on: bool, action: CommandsAction) -> AnyElement {
    view! { <div class="shrink-0">{switch(on).label(label).on_toggle(action)}</div> }
}

fn select_button(label: &str, value: &str, action: CommandsAction, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    view! {
        <div
            class="flex-row items-center shrink-0"
            gap={tokens::scaled(Sp::XS, scale)}
            px={tokens::scaled(Sp::MD, scale)}
            h={theme.metrics.ui_row_height}
            rounded={theme.metrics.control_radius}
            bg={tc.element_background}
            border={tc.control_border}
            hover_bg={tc.element_hover}
            cursor={CursorHint::Pointer}
            id={format!("settings.{label}")}
            role="button"
            aria-label={format!("{label}: {value}")}
            on:click={action}
        >
            <text size={TextStyle::Body.font_size(theme)} color={tc.text}>{value.to_owned()}</text>
            <icon svg={lucide::CHEVRON_DOWN} size={Ico::XS} color={tc.text_muted} />
        </div>
    }
}

fn theme_button(settings: &Settings, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let swatch = tokens::scaled(Sp::MD, scale);
    let chip = |color: Color| view! { <div class="shrink-0" w={swatch} h={swatch} rounded={theme.metrics.control_radius * 0.5} bg={color} /> };
    view! {
        <div
            class="flex-row items-center shrink-0"
            gap={tokens::scaled(Sp::XS, scale)}
            px={tokens::scaled(Sp::MD, scale)}
            h={theme.metrics.ui_row_height}
            rounded={theme.metrics.control_radius}
            bg={tc.element_background}
            border={tc.control_border}
            hover_bg={tc.element_hover}
            cursor={CursorHint::Pointer}
            id="settings.theme"
            role="button"
            aria-label={format!("Theme: {}", settings.theme_name)}
            on:click={CommandsAction::OpenThemePicker}
        >
            {chip(tc.accent)}
            {chip(tc.line_add_text)}
            {chip(tc.line_del_text)}
            <text size={TextStyle::Body.font_size(theme)} color={tc.text}>{settings.theme_name.clone()}</text>
            <icon svg={lucide::CHEVRON_DOWN} size={Ico::XS} color={tc.text_muted} />
        </div>
    }
}

fn size_slider(settings: &Settings, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let width = tokens::scaled(Layout::SETTINGS_SHEET_W, scale);
    let (min, max) = (SIZE_STOPS[0], SIZE_STOPS[SIZE_STOPS.len() - 1]);
    let control = slider(
        "settings.interface-size",
        "Interface size",
        f32::from(settings.ui_scale_pct),
        f32::from(min),
        f32::from(max),
        |v| Action::from(CommandsAction::SetUiScalePct(snap(v))).into(),
    )
    .step(5.0)
    .width(width);
    let control = div().flex_row().w_full().child(control);
    let stops: Vec<AnyElement> = SIZE_STOPS
        .iter()
        .map(|stop| view! { <text size={TextStyle::Caption.font_size(theme)} class="font-mono" color={tc.text_muted}>{stop.to_string()}</text> })
        .collect();
    view! {
        <div class="flex-col w-full" gap={tokens::scaled(Sp::XXS, scale)}>
            {control}
            <div class="flex-row items-center justify-between w-full" pr={tokens::scaled(Sp::XXL, scale)}>{...stops}</div>
        </div>
    }
}

fn snap(value: f32) -> u16 {
    let value = value.round() as u16;
    SIZE_STOPS
        .iter()
        .copied()
        .find(|stop| stop.abs_diff(value) <= 3)
        .unwrap_or(value)
}

fn keyboard(settings: &Settings, page: &SettingsPage, query: &str, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let saved = &settings.keymap_overrides;
    let mut columns: [Vec<AnyElement>; 2] = Default::default();
    let mut heights = [0usize; 2];
    for group in KEYMAP.groups() {
        let entries: Vec<_> = group
            .entries
            .iter()
            .filter(|entry| query.is_empty() || matches(query, entry.description, group.title))
            .collect();
        if entries.is_empty() {
            continue;
        }
        let count = entries.len();
        let rows: Vec<AnyElement> = entries
            .into_iter()
            .enumerate()
            .map(|(i, entry)| {
                let capturing = page.capture == Some(entry.command);
                let binding = if capturing {
                    "Press keys\u{2026}".to_owned()
                } else {
                    keymap::display_binding(saved, entry.command).unwrap_or_else(|| "Unbound".to_owned())
                };
                let overridden = saved.iter().any(|o| o.command == entry.command);
                let action = if capturing {
                    CommandsAction::CancelKeymapRebind
                } else {
                    CommandsAction::BeginKeymapRebind(entry.command)
                };
                let reset = overridden.then(|| {
                    Button::new(Action::from(CommandsAction::ResetKeymapBinding(entry.command)))
                        .icon(icons::UNDO)
                        .tooltip(format!("Reset {}", entry.description))
                        .style(ButtonStyle::Ghost)
                        .size(ButtonSize::Compact)
                });
                let last = i + 1 == count;
                view! {
                    <div
                        class="flex-row items-center w-full"
                        h={theme.metrics.ui_row_height.round() + tokens::scaled(Sp::XS, scale)}
                        px={tokens::scaled(Sp::MD, scale)}
                        gap={tokens::scaled(Sp::SM, scale)}
                        @when {!last} { border_b={tc.border_variant} }
                    >
                        <div class="flex-1 flex-row" min_w={0.0}><text size={TextStyle::Body.font_size(theme)} class="truncate" color={tc.text}>{entry.description}</text></div>
                        {?reset.map(|b| view! { <div class="shrink-0" aria-label={format!("Reset {}", entry.description)}>{b}</div> })}
                        <div
                            class="flex-row items-center shrink-0"
                            gap={tokens::scaled(Sp::XS, scale)}
                            px={tokens::scaled(Sp::XS, scale)}
                            h={theme.metrics.ui_row_height}
                            rounded={theme.metrics.control_radius}
                            bg={if capturing { tc.ghost_element_selected } else { Color::TRANSPARENT }}
                            hover_bg={tc.ghost_element_hover}
                            cursor={CursorHint::Pointer}
                            id={format!("keymap.{:?}", entry.command)}
                            role="button"
                            aria-label={if capturing { format!("Press keys for {}", entry.description) } else { format!("Rebind {}", entry.description) }}
                            on:click={action}
                        >
                            {kbd(binding, theme)}
                            <icon svg={lucide::PENCIL} size={Ico::XS} color={tc.text_muted} />
                        </div>
                    </div>
                }
            })
            .collect();
        let noun = if count == 1 { "binding" } else { "bindings" };
        let block = view! {
            <div class="flex-col w-full" gap={tokens::scaled(Sp::XS, scale)}>
                <div class="flex-row items-center w-full" gap={tokens::scaled(Sp::SM, scale)}>
                    <text size={TextStyle::Caption.font_size(theme)} class="font-semibold" color={tc.text_muted}>{group.title.to_uppercase()}</text>
                    <div class="flex-1" />
                    <text size={TextStyle::Caption.font_size(theme)} color={tc.text_muted}>{format!("{count} {noun}")}</text>
                </div>
                {card(rows, theme)}
            </div>
        };
        let column = if heights[0] <= heights[1] { 0 } else { 1 };
        heights[column] += count + 2;
        columns[column].push(block);
    }
    let notice = page.notice.clone().map(|notice| {
        view! {
            <div accessibility_role={AccessibilityRole::Alert} aria-label={notice.clone()}>
                <text size={TextStyle::BodySmall.font_size(theme)} color={tc.status_error}>{notice}</text>
            </div>
        }
    });
    let [left, right] = columns;
    view! {
        <div class="flex-col w-full shrink-0" gap={tokens::scaled(Sp::SM, scale)}>
            {?notice}
            <div class="flex-row items-start w-full" gap={tokens::scaled(Sp::MD, scale)}>
                <div class="flex-1 flex-col" min_w={0.0} gap={tokens::scaled(Sp::MD, scale)}>{...left}</div>
                <div class="flex-1 flex-col" min_w={0.0} gap={tokens::scaled(Sp::MD, scale)}>{...right}</div>
            </div>
        </div>
    }
}

fn preview(settings: &Settings, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let mode = match settings.theme_mode {
        ThemeMode::Light => QuarkMode::Light,
        ThemeMode::Dark | ThemeMode::System => theme.mode,
    };
    let shown = crate::app::theme::preview_theme(settings, mode);
    let pc = &shown.colors;
    let mono = TextStyle::Mono.font_size(theme);
    let line = |number: &str, bar: Color, bg: Color, parts: Vec<(String, Color)>| {
        let spans: Vec<AnyElement> = parts
            .into_iter()
            .map(|(text_part, color)| view! { <text size={mono} class="font-mono" color={color}>{text_part}</text> })
            .collect();
        view! {
            <div class="flex-row items-center w-full" h={(mono * 1.7).round()} bg={bg}>
                <div class="flex-row justify-end shrink-0" w={tokens::scaled(Sp::XXL, scale)} pr={tokens::scaled(Sp::SM, scale)}>
                    <text size={mono} class="font-mono" color={pc.gutter_text}>{number.to_owned()}</text>
                </div>
                <div class="shrink-0 h-full" w={tokens::scaled(Sp::XXS, scale)} bg={bar} />
                <div class="flex-row items-center" pl={tokens::scaled(Sp::SM, scale)}>{...spans}</div>
            </div>
        }
    };
    let plain = Color::TRANSPARENT;
    let rows = vec![
        line(
            "12",
            plain,
            plain,
            vec![
                ("fn ".into(), pc.syntax_keyword),
                ("send".into(), pc.syntax_function),
                ("(&self) {".into(), pc.text),
            ],
        ),
        line(
            "13",
            pc.line_del_text,
            pc.line_del,
            vec![
                ("  let _ = tx.".into(), pc.text),
                ("send".into(), pc.syntax_function),
                ("(cmd);".into(), pc.text),
            ],
        ),
        line(
            "13",
            pc.line_add_text,
            pc.line_add,
            vec![
                ("  tx.".into(), pc.text),
                ("send".into(), pc.syntax_function),
                ("(cmd)?;".into(), pc.text),
            ],
        ),
        line("14", plain, plain, vec![("}".into(), pc.text)]),
    ];
    view! {
        <div
            class="flex-col shrink-0 h-full"
            w={tokens::scaled(Layout::SETTINGS_PREVIEW_W, scale)}
            pt={tokens::scaled(Sp::XL, scale)}
            pr={tokens::scaled(Sp::XL, scale)}
            gap={tokens::scaled(Sp::SM, scale)}
            accessibility_role={AccessibilityRole::Figure}
            aria-label="Preview"
            test_id="settings.preview"
        >
            <text size={TextStyle::Caption.font_size(theme)} class="font-semibold" color={tc.text_muted}>"PREVIEW"</text>
            <div
                class="flex-col w-full overflow-hidden"
                bg={pc.editor_surface}
                border={tc.border_variant}
                rounded={tokens::scaled(Rad::XL, scale)}
            >
                <div class="flex-row items-center w-full" h={theme.metrics.ui_row_height} px={tokens::scaled(Sp::MD, scale)} bg={pc.file_header_bg}>
                    <text size={TextStyle::BodySmall.font_size(theme)} class="font-medium" color={pc.text}>"worker.rs"</text>
                </div>
                <div class="flex-col w-full" py={tokens::scaled(Sp::XS, scale)}>{...rows}</div>
            </div>
        </div>
    }
}
