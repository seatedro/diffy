use quark::view;
use quark_app::ViewContext;
use quark_components::{
    Modal, PALETTE_INPUT, PaletteEvent, SegmentedControl, SegmentedItem, progress_bar, switch,
};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::CommandsController;
use super::keymap::{self, KEYMAP};
use super::list::{self, OptionRow};
use super::picker::PICKER_VISIBLE_ROWS;
use super::state::{CONFIRM_ACCEPT, CONFIRM_CANCEL, LayerKind, SettingsPage};
use crate::app::shell::Slot;
use crate::app::text_targets::{PICKER, SETTINGS};
use crate::app::tokens::{self, Ico, Rad, Sp, Sz, TextStyle};
use crate::contracts::commands::{CommandsAction, SettingsSection, ToastKind};
use crate::contracts::shell::Action;
use crate::core::theme_types::ThemeMode;
use crate::fonts::font_selection_label;
use crate::platform::persistence::Settings;

fn palette_event(event: PaletteEvent) -> quark_ui::Action {
    Action::from(CommandsAction::Palette(event)).into()
}

pub fn clankers_slot(commands: &CommandsController, slot: &Slot, theme: &Theme) -> Option<f32> {
    let page = commands.settings_page()?;
    (page.section == SettingsSection::Clankers).then(|| settings_content_width(theme, slot.width))
}

fn settings_nav_width(theme: &Theme, width: f32) -> f32 {
    tokens::scaled(Sz::SETTINGS_NAV_W, tokens::ui_scale(theme)).min(width * 0.4)
}

fn settings_content_width(theme: &Theme, width: f32) -> f32 {
    let scale = tokens::ui_scale(theme);
    let padding = tokens::scaled(Sp::XL, scale) * 2.0;
    (width - settings_nav_width(theme, width) - padding)
        .min(tokens::scaled(Sz::SETTINGS_CONTENT_MAX_W, scale))
        .max(0.0)
}

pub fn overlays(
    commands: &mut CommandsController,
    slot: &Slot,
    clankers: Option<AnyElement>,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let theme = cx.theme;
    let row_stride =
        theme.metrics.ui_row_height.round() + tokens::scaled(Sp::XS, tokens::ui_scale(theme));
    commands.observe_view(cx.focus(), row_stride);
    let size = (slot.width, slot.height);
    let mut children: Vec<AnyElement> = Vec::new();
    if commands.settings_page().is_some() {
        children.push(settings_page(commands, size, clankers, cx));
    }
    let layers: Vec<LayerKind> = [
        LayerKind::Picker,
        LayerKind::Palette,
        LayerKind::Confirmation,
        LayerKind::ContextMenu,
    ]
    .into_iter()
    .filter(|kind| commands.has_layer(*kind))
    .collect();
    for kind in layers {
        let element = match kind {
            LayerKind::Palette => {
                let focused = cx.is_focused(PALETTE_INPUT);
                commands
                    .palette()
                    .render(size, cx.theme, focused, palette_event)
            }
            LayerKind::Picker => picker_panel(commands, size, cx),
            LayerKind::Confirmation => confirmation(commands, size, cx.theme),
            LayerKind::ContextMenu => context_menu(commands, size, cx.theme),
        };
        children.extend(element);
    }
    if children.is_empty() {
        return None;
    }
    Some(view! {
        <div class="absolute top-0 left-0" w={slot.width} h={slot.height}>
            {...children}
        </div>
    })
}

fn picker_panel(
    commands: &mut CommandsController,
    size: (f32, f32),
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let focused = cx.is_focused(PICKER);
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let row_stride = list::row_stride(theme);
    let picker = commands.picker_mut()?;
    picker.reveal_pending(row_stride);
    let picker = &*picker;
    let title = picker.kind.title();
    let panel_w = (Sz::MODAL_MD * scale)
        .min(size.0 - tokens::scaled(Sz::MODAL_MARGIN, scale))
        .max(1.0);
    let input = text_input(title, "")
        .field(&picker.query)
        .placeholder(picker.kind.placeholder())
        .focus_target(PICKER)
        .focused(focused)
        .search(true)
        .bare()
        .w_full()
        .h(tokens::scaled(Sz::INPUT, scale));
    let list = if picker.rows.is_empty() {
        view! {
            <div class="flex-row items-center" h={theme.metrics.ui_row_height} px={tokens::scaled(Sp::MD, scale)}>
                <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>"No matches"</text>
            </div>
        }
    } else {
        list::virtual_list(
            picker.rows.len(),
            picker.scroll_px,
            PICKER_VISIBLE_ROWS,
            theme,
            title,
            |i| {
                let row = &picker.rows[i];
                list::option_row(
                    OptionRow {
                        id: format!("picker-row:{i}:{}", row.label),
                        label: &row.label,
                        highlights: &row.highlights,
                        detail: row.detail.as_deref(),
                        rhs: None,
                        icon: None,
                        selected: i == picker.selected,
                        current: row.current,
                    },
                    theme,
                    Action::from(CommandsAction::SelectOverlayEntry(i)).into(),
                )
            },
            ScrollActionBuilder::new(|lines| {
                Action::from(CommandsAction::ScrollOverlay(lines)).into()
            }),
        )
    };
    Some(view! {
        <div
            class="absolute top-0 left-0 flex-col items-center"
            w={size.0}
            h={size.1}
            z_index={300}
            pt={tokens::scaled(Sz::MODAL_TOP_OFFSET, scale)}
            bg={tc.overlay_scrim}
            test_id="picker-backdrop"
            on:click={CommandsAction::CloseOverlay}
            block_mouse
        >
            <div
                class="flex-col overflow-hidden"
                w={panel_w}
                bg={tc.elevated_surface}
                border={tc.border}
                rounded={tokens::scaled(Rad::XL, scale)}
                on:click={NoopAction}
                test_id="picker"
                role="dialog"
                focus_scope="diffy.picker"
                trap_focus={true}
                accessibility_role={AccessibilityRole::Dialog}
                aria-label={title}
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
                <div p={tokens::scaled(Sp::XS, scale)}>{list}</div>
            </div>
        </div>
    })
}

fn confirmation(
    commands: &CommandsController,
    size: (f32, f32),
    theme: &Theme,
) -> Option<AnyElement> {
    let request = commands.confirmation()?;
    let scale = tokens::ui_scale(theme);
    let icon = if request.destructive {
        lucide::ALERT_CIRCLE
    } else {
        lucide::INFO
    };
    let cancel = action_button(
        "Cancel",
        CommandsAction::CancelConfirmation.into(),
        CONFIRM_CANCEL,
        ButtonTone::Plain,
        theme,
    );
    let confirm = action_button(
        &request.confirm_label,
        CommandsAction::Confirm.into(),
        CONFIRM_ACCEPT,
        if request.destructive {
            ButtonTone::Danger
        } else {
            ButtonTone::Primary
        },
        theme,
    );
    let modal = Modal::new(
        request.title.clone(),
        request.message.clone(),
        icon,
        tokens::scaled(Sz::MODAL_SM, scale),
        size.0,
        size.1,
        CommandsAction::CancelConfirmation,
    )
    .footer_child(view! { <div class="flex-1" /> })
    .footer_child(cancel)
    .footer_child(confirm);
    Some(view! { <div class="absolute top-0 left-0" z_index={350}>{modal}</div> })
}

fn context_menu(
    commands: &mut CommandsController,
    size: (f32, f32),
    theme: &Theme,
) -> Option<AnyElement> {
    let menu = commands.context_menu_mut().render(size, theme)?;
    Some(view! {
        <div
            class="absolute top-0 left-0"
            w={size.0}
            h={size.1}
            z_index={450}
            test_id="context-menu-backdrop"
            on:click={CommandsAction::CloseContextMenu}
        >
            {menu}
        </div>
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonTone {
    Plain,
    Primary,
    Danger,
}

pub fn action_button(
    label: &str,
    action: Action,
    focus: FocusId,
    tone: ButtonTone,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let (bg, hover, fg) = match tone {
        ButtonTone::Plain => (tc.element_background, tc.element_hover, tc.text),
        ButtonTone::Primary => (tc.accent, tc.accent_strong, tc.on_accent),
        ButtonTone::Danger => (tc.status_error, tc.status_error, tc.on_accent),
    };
    view! {
        <div
            class="flex-row items-center justify-center shrink-0"
            px={tokens::scaled(Sp::MD, scale)}
            h={theme.metrics.ui_row_height}
            rounded={theme.metrics.control_radius}
            bg={bg}
            hover_bg={hover}
            cursor={CursorHint::Pointer}
            focus_ring={focus}
            role="button"
            aria-label={label}
            on:click={action}
        >
            <text size={TextStyle::Body.font_size(theme)} class="font-medium" color={fg}>{label}</text>
        </div>
    }
}

fn settings_page(
    commands: &CommandsController,
    size: (f32, f32),
    clankers: Option<AnyElement>,
    cx: &mut ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let Some(page) = commands.settings_page() else {
        return view! { <div /> };
    };
    let settings = &commands.settings().settings;
    let top = theme.metrics.title_bar_height.min(size.1);
    let nav_w = settings_nav_width(theme, size.0);
    let nav: Vec<AnyElement> = SettingsSection::ALL
        .iter()
        .map(|section| nav_item(*section, page.section == *section, theme))
        .collect();
    let content = match page.section {
        SettingsSection::Appearance => appearance(settings, theme),
        SettingsSection::Editor => editor(settings, theme),
        SettingsSection::Behavior => behavior(settings, theme),
        SettingsSection::Keymaps => keymaps(settings, page, theme),
        SettingsSection::Clankers => clankers.unwrap_or_else(|| {
            note(
                "AI providers",
                "Provider keys and the commit steering prompt appear here.",
                theme,
            )
        }),
        SettingsSection::About => about(theme),
    };
    view! {
        <div
            class="absolute left-0 flex-row"
            top={top}
            w={size.0}
            h={(size.1 - top).max(0.0)}
            z_index={200}
            bg={tc.app_bg}
            key_context={keymap::SETTINGS_CONTEXT}
            focus_ring={SETTINGS}
            accessibility_role={AccessibilityRole::Region}
            aria-label="Settings"
            test_id="settings"
        >
            <div
                class="flex-col shrink-0"
                w={nav_w}
                h_full
                gap={tokens::scaled(Sp::XXS, scale)}
                p={tokens::scaled(Sp::MD, scale)}
                bg={tc.sidebar_background}
                border_r={tc.border_variant}
                accessibility_role={AccessibilityRole::TabList}
                aria-label="Settings sections"
            >
                <div class="flex-row items-center" pb={tokens::scaled(Sp::SM, scale)} gap={tokens::scaled(Sp::SM, scale)}>
                    <text size={TextStyle::Heading.font_size(theme)} class="font-semibold" color={tc.text_strong}>"Settings"</text><div class="flex-1" />
                    <div
                        class="flex-row items-center justify-center"
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
                {...nav}
            </div>
            <div
                class="flex-1 flex-col overflow-hidden"
                min_w={0.0}
                p={tokens::scaled(Sp::XL, scale)}
                gap={tokens::scaled(Sp::LG, scale)}
            >
                <text size={TextStyle::HeadingLg.font_size(theme)} class="font-semibold" color={tc.text_strong}>
                    {page.section.label()}
                </text>
                <div
                    class="flex-col w-full flex-1 overflow-y-scroll"
                    min_h={0.0}
                    track_scroll={&page.scroll}
                    accessibility_role={AccessibilityRole::Group}
                    aria-label={format!("{} settings", page.section.label())}
                    test_id="settings.content"
                >
                    {content}
                </div>
            </div>
        </div>
    }
}

fn nav_item(section: SettingsSection, selected: bool, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    view! {
        <div
            class="flex-row items-center w-full"
            h={theme.metrics.ui_row_height}
            px={tokens::scaled(Sp::SM, scale)}
            rounded={theme.metrics.control_radius}
            bg={if selected { tc.sidebar_row_selected } else { Color::TRANSPARENT }}
            hover_bg={tc.sidebar_row_hover}
            cursor={CursorHint::Pointer}
            id={format!("settings.section.{}", section.label())}
            accessibility_role={AccessibilityRole::Tab}
            aria-label={section.label()}
            aria-selected={selected}
            on:click={CommandsAction::SetSettingsSection(section)}
        >
            <text size={TextStyle::Body.font_size(theme)} color={if selected { tc.text_strong } else { tc.text }}>
                {section.label()}
            </text>
        </div>
    }
}

fn row(label: &str, detail: Option<&str>, control: AnyElement, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    view! {
        <div
            class="flex-row items-center w-full"
            gap={tokens::scaled(Sp::LG, scale)}
            py={tokens::scaled(Sp::SM, scale)}
            border_b={tc.border_variant}
            max_w={tokens::scaled(Sz::SETTINGS_CONTENT_MAX_W, scale)}
        >
            <div class="flex-1 flex-col" min_w={0.0} gap={tokens::scaled(Sp::XXS, scale)}>
                <text size={TextStyle::Body.font_size(theme)} color={tc.text}>{label}</text>
                {?detail.map(|d| view! { <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{d}</text> })}
            </div>
            {control}
        </div>
    }
}

fn toggle(label: &str, on: bool, action: CommandsAction) -> AnyElement {
    view! { <div class="shrink-0">{switch(on).label(label).on_toggle(action)}</div> }
}

fn value_button(label: &str, value: &str, action: CommandsAction, theme: &Theme) -> AnyElement {
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
            hover_bg={tc.element_hover}
            cursor={CursorHint::Pointer}
            id={format!("settings.{label}")}
            role="button"
            aria-label={label}
            on:click={action}
        >
            <text size={TextStyle::Body.font_size(theme)} color={tc.text}>{value}</text>
            <icon svg={lucide::CHEVRON_DOWN} size={Ico::XS} color={tc.icon} />
        </div>
    }
}

fn stepper(
    label: &str,
    value: String,
    minus: CommandsAction,
    plus: CommandsAction,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let step = |icon: &'static str, name: String, action: CommandsAction| {
        view! {
            <div
                class="flex-row items-center justify-center"
                w={theme.metrics.ui_row_height}
                h={theme.metrics.ui_row_height}
                rounded={theme.metrics.control_radius}
                bg={tc.element_background}
                hover_bg={tc.element_hover}
                cursor={CursorHint::Pointer}
                id={format!("settings.{name}")}
                role="button"
                aria-label={name}
                on:click={action}
            >
                <icon svg={icon} size={Ico::XS} color={tc.icon} />
            </div>
        }
    };
    view! {
        <div class="flex-row items-center shrink-0" gap={tokens::scaled(Sp::SM, scale)}>
            {step(lucide::MINUS, format!("Decrease {label}"), minus)}
            <text size={TextStyle::Body.font_size(theme)} color={tc.text}>{value}</text>
            {step(lucide::PLUS, format!("Increase {label}"), plus)}
        </div>
    }
}

fn appearance(settings: &Settings, theme: &Theme) -> AnyElement {
    let scale = tokens::ui_scale(theme);
    let mode = SegmentedControl::new(vec![
        SegmentedItem::new(
            "Dark",
            CommandsAction::SetThemeMode(ThemeMode::Dark),
            settings.theme_mode == ThemeMode::Dark,
        ),
        SegmentedItem::new(
            "Light",
            CommandsAction::SetThemeMode(ThemeMode::Light),
            settings.theme_mode == ThemeMode::Light,
        ),
    ]);
    let pct = settings.ui_scale_pct;
    view! {
        <div class="flex-col w-full" gap={tokens::scaled(Sp::XS, scale)}>
            {row("Theme", None, value_button("Theme", &settings.theme_name, CommandsAction::OpenThemePicker, theme), theme)}
            {row("Mode", None, view! { <div class="shrink-0">{mode}</div> }, theme)}
            {row(
                "Interface scale",
                None,
                stepper(
                    "interface scale",
                    format!("{pct}%"),
                    CommandsAction::DecreaseUiScale,
                    CommandsAction::IncreaseUiScale,
                    theme,
                ),
                theme,
            )}
            {row("Interface font", None, value_button("Interface font", &font_selection_label(&settings.fonts.ui_family), CommandsAction::OpenUiFontPicker, theme), theme)}
            {row("Code font", None, value_button("Code font", &font_selection_label(&settings.fonts.mono_family), CommandsAction::OpenMonoFontPicker, theme), theme)}
        </div>
    }
}

fn editor(settings: &Settings, theme: &Theme) -> AnyElement {
    let scale = tokens::ui_scale(theme);
    let viewport = &settings.viewport;
    let column = if viewport.wrap_column == 0 {
        "Window".to_owned()
    } else {
        viewport.wrap_column.to_string()
    };
    let lines = settings.wheel_scroll_lines;
    view! {
        <div class="flex-col w-full" gap={tokens::scaled(Sp::XS, scale)}>
            {row("Wrap lines", None, toggle("Wrap lines", viewport.wrap_enabled, CommandsAction::ToggleWrap), theme)}
            {row(
                "Wrap column",
                Some("Window wraps at the viewport edge"),
                stepper(
                    "wrap column",
                    column,
                    CommandsAction::SetWrapColumn(viewport.wrap_column.saturating_sub(WRAP_STEP)),
                    CommandsAction::SetWrapColumn(viewport.wrap_column.saturating_add(WRAP_STEP).min(MAX_WRAP_COLUMN)),
                    theme,
                ),
                theme,
            )}
            {row("Continuous scroll", Some("Show every file in one scrolling view"), toggle("Continuous scroll", settings.continuous_scroll, CommandsAction::ToggleContinuousScroll), theme)}
            {row(
                "Scroll speed",
                Some("Lines per mouse wheel step"),
                stepper(
                    "scroll speed",
                    lines.to_string(),
                    CommandsAction::SetWheelScrollLines(lines.saturating_sub(1)),
                    CommandsAction::SetWheelScrollLines(lines.saturating_add(1)),
                    theme,
                ),
                theme,
            )}
        </div>
    }
}

const WRAP_STEP: u32 = 10;
const MAX_WRAP_COLUMN: u32 = 400;

fn behavior(settings: &Settings, theme: &Theme) -> AnyElement {
    let scale = tokens::ui_scale(theme);
    view! {
        <div class="flex-col w-full" gap={tokens::scaled(Sp::XS, scale)}>
            {row("Automatic updates", Some("Check for new versions in the background"), toggle("Automatic updates", settings.auto_update, CommandsAction::ToggleAutoUpdate), theme)}
        </div>
    }
}

fn keymaps(settings: &Settings, page: &SettingsPage, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let saved = &settings.keymap_overrides;
    let mut rows: Vec<AnyElement> = Vec::new();
    for group in KEYMAP.groups() {
        rows.push(view! {
            <div class="w-full" pt={tokens::scaled(Sp::MD, scale)}>
                <text size={TextStyle::BodySmall.font_size(theme)} class="font-medium" color={tc.text_muted}>{group.title}</text>
            </div>
        });
        for entry in group.entries {
            let capturing = page.capture == Some(entry.command);
            let binding = if capturing {
                "Press keys…".to_owned()
            } else {
                keymap::display_binding(saved, entry.command)
                    .unwrap_or_else(|| "Unbound".to_owned())
            };
            let overridden = saved.iter().any(|o| o.command == entry.command);
            let action = if capturing {
                CommandsAction::CancelKeymapRebind
            } else {
                CommandsAction::BeginKeymapRebind(entry.command)
            };
            let reset = overridden.then(|| {
                view! {
                    <div
                        class="flex-row items-center"
                        px={tokens::scaled(Sp::SM, scale)}
                        h={theme.metrics.ui_row_height}
                        rounded={theme.metrics.control_radius}
                        hover_bg={tc.ghost_element_hover}
                        cursor={CursorHint::Pointer}
                        id={format!("keymap.reset.{:?}", entry.command)}
                        role="button"
                        aria-label={format!("Reset {}", entry.description)}
                        on:click={CommandsAction::ResetKeymapBinding(entry.command)}
                    >
                        <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>"Reset"</text>
                    </div>
                }
            });
            rows.push(view! {
                <div
                    class="flex-row items-center w-full"
                    gap={tokens::scaled(Sp::SM, scale)}
                    h={theme.metrics.ui_row_height}
                    max_w={tokens::scaled(Sz::SETTINGS_CONTENT_MAX_W, scale)}
                >
                    <text size={TextStyle::Body.font_size(theme)} class="truncate" color={tc.text}>{entry.description}</text><div class="flex-1" />
                    {?reset}
                    <div
                        class="flex-row items-center shrink-0"
                        px={tokens::scaled(Sp::SM, scale)}
                        h={theme.metrics.ui_row_height}
                        rounded={theme.metrics.control_radius}
                        bg={if capturing { tc.element_selected } else { tc.element_background }}
                        hover_bg={tc.element_hover}
                        cursor={CursorHint::Pointer}
                        id={format!("keymap.{:?}", entry.command)}
                        role="button"
                        aria-label={if capturing { format!("Press keys for {}", entry.description) } else { format!("Rebind {}", entry.description) }}
                        on:click={action}
                    >
                        <text size={TextStyle::MonoSmall.font_size(theme)} color={tc.text}>{binding}</text>
                    </div>
                </div>
            });
        }
    }
    let notice = page.notice.clone().map(|notice| {
        view! {
            <div accessibility_role={AccessibilityRole::Alert} aria-label={notice.clone()}>
                <text size={TextStyle::BodySmall.font_size(theme)} color={tc.status_error}>{notice}</text>
            </div>
        }
    });
    view! {
        <div class="flex-col w-full shrink-0">
            {?notice}
            {...rows}
        </div>
    }
}

fn note(title: &str, detail: &str, theme: &Theme) -> AnyElement {
    crate::app::shell::empty_state(title, detail, theme)
}

fn about(theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let name = crate::platform::startup::app_display_name();
    view! {
        <div class="flex-col" gap={tokens::scaled(Sp::XS, scale)}>
            <text size={TextStyle::Body.font_size(theme)} class="font-medium" color={tc.text}>{name}</text>
            <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{format!("Version {}", crate::APP_VERSION)}</text>
        </div>
    }
}

pub fn toasts(
    commands: &mut CommandsController,
    slot: &Slot,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let now = cx.frame.elapsed().as_millis() as u64;
    if let Some(next) = commands.toasts_mut().tick(now) {
        cx.frame
            .request_frame_in(std::time::Duration::from_millis(next.max(1)));
    }
    let items = commands.toasts().items();
    if items.is_empty() {
        return None;
    }
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let width = tokens::scaled(Sz::TOAST_MAX_W, scale)
        .min(slot.width - tokens::scaled(Sp::LG, scale) * 2.0)
        .max(1.0);
    let margin = tokens::scaled(Sp::LG, scale);
    let rows: Vec<AnyElement> = items
        .iter()
        .map(|toast| {
            let stripe = match toast.kind {
                ToastKind::Info => tc.status_info,
                ToastKind::Success => tc.line_add_text,
                ToastKind::Error => tc.status_error,
            };
            let role = match toast.kind {
                ToastKind::Error => AccessibilityRole::Alert,
                ToastKind::Info | ToastKind::Success => AccessibilityRole::Status,
            };
            let progress = toast.fraction().map(|f| view! { <div class="w-full">{progress_bar(f)}</div> });
            view! {
                <div
                    class="flex-row w-full overflow-hidden"
                    bg={tc.elevated_surface}
                    border={tc.border}
                    rounded={tokens::scaled(Rad::XL, scale)}
                    key={toast.id.to_string()}
                    accessibility_role={role}
                    aria-label={toast.message.clone()}
                    test_id="toast"
                >
                    <div class="shrink-0" w={tokens::scaled(Sz::TOAST_STRIPE_W, scale)} bg={stripe} />
                    <div class="flex-1 flex-col" min_w={0.0} gap={tokens::scaled(Sp::XXS, scale)} p={tokens::scaled(Sp::MD, scale)}>
                        <text size={TextStyle::Body.font_size(theme)} class="font-medium" color={tc.text}>{toast.message.clone()}</text>
                        {?toast.description.clone().map(|d| view! { <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{d}</text> })}
                        {?progress}
                    </div>
                    <div
                        class="flex-row items-center justify-center shrink-0"
                        w={theme.metrics.ui_row_height}
                        hover_bg={tc.ghost_element_hover}
                        cursor={CursorHint::Pointer}
                        id={format!("toast.dismiss.{}", toast.id)}
                        role="button"
                        aria-label="Dismiss"
                        on:click={CommandsAction::DismissToast(toast.id)}
                    >
                        <icon svg={lucide::X} size={Ico::XS} color={tc.icon} />
                    </div>
                </div>
            }
        })
        .collect();
    Some(view! {
        <div
            class="absolute flex-col"
            right={margin}
            bottom={theme.metrics.status_bar_height + margin}
            w={width}
            gap={tokens::scaled(Sp::SM, scale)}
            z_index={500}
            accessibility_role={AccessibilityRole::Log}
            aria-label="Notifications"
        >
            {...rows}
        </div>
    })
}
