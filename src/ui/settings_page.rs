use halogen::view;

use crate::actions::Action;
use crate::core::compare::{LayoutMode, RendererKind};
use crate::ui::components::{
    Button, ButtonSize, ButtonStyle, SegmentedControl, SegmentedItem, toggle,
};
use crate::ui::design::{Ico, Rad, Sp};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{AppState, SettingsSection};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme, ThemeMode};

const NAV_WIDTH: f32 = 220.0;
const CONTENT_MAX_WIDTH: f32 = 720.0;

pub fn settings_page(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let active = state.settings_section.get(&state.store);

    let nav = nav_panel(state, theme, active);
    let content = section_content(state, theme, active);

    view! {
        <div class="flex-1 flex-row" min_h={0.0} bg={tc.editor_surface}>
            {nav}
            <div class="flex-1 flex-col overflow-hidden" min_w={0.0}>
                {content}
            </div>
        </div>
    }
}

fn nav_panel(_state: &AppState, theme: &Theme, active: SettingsSection) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let nav_w = (NAV_WIDTH * scale).round();

    let entries: Vec<AnyElement> = SettingsSection::ALL
        .iter()
        .copied()
        .map(|section| nav_row(theme, section, section == active))
        .collect();

    view! { scale,
        <div class="flex-col shrink-0"
             w={nav_w}
             bg={tc.surface}
             border_r={tc.border_variant}>
            <div class="flex-row items-center"
                 px={Sp::MD} pt={Sp::MD} pb={Sp::SM}
                 gap={Sp::SM}>
                <text class="text-xs font-semibold" color={tc.text_muted}>{"SETTINGS"}</text>
                <spacer />
                {Button::new(Action::CloseSettings)
                    .icon(lucide::X)
                    .style(ButtonStyle::Ghost)
                    .size(ButtonSize::Compact)
                    .tooltip("Close settings  (Esc)")}
            </div>
            <div class="flex-col" px={Sp::XS} gap={Sp::XS}>
                {...entries}
            </div>
        </div>
    }
}

fn nav_row(theme: &Theme, section: SettingsSection, selected: bool) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let icon_color = if selected {
        tc.text_strong
    } else {
        tc.text_muted
    };
    let label_color = if selected { tc.text_strong } else { tc.text };

    view! { scale,
        <div class="flex-row items-center"
             gap={Sp::SM}
             px={Sp::SM} py={Sp::XS + Sp::XXS}
             rounded={Rad::MD}
             bg={if selected { tc.element_selected } else { Color::TRANSPARENT }}
             hover_bg={if !selected { tc.ghost_element_hover }}
             on_click={Action::SetSettingsSection(section)}
             cursor={CursorHint::Pointer}>
            <icon svg={section.icon()} size={Ico::SM * scale} color={icon_color} />
            <text class="text-sm font-medium" color={label_color}>{section.label()}</text>
        </div>
    }
}

fn section_content(state: &AppState, theme: &Theme, section: SettingsSection) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let max_w = (CONTENT_MAX_WIDTH * scale).round();

    let (title, description, body) = match section {
        SettingsSection::Appearance => (
            "Appearance",
            "Theme and density.",
            appearance_section(state, theme),
        ),
        SettingsSection::Editor => (
            "Editor",
            "How diffs are laid out.",
            editor_section(state, theme),
        ),
        SettingsSection::Behavior => (
            "Behavior",
            "Input and interaction.",
            behavior_section(state, theme),
        ),
        SettingsSection::About => ("About", "Diffy build information.", about_section(theme)),
    };

    view! { scale,
        <div class="flex-1 flex-col items-stretch overflow-hidden"
             min_h={0.0}
             px={Sp::XXL}
             pt={Sp::XL}
             pb={Sp::XXL}>
            <div class="flex-col" max_w={max_w} gap={Sp::LG}>
                <div class="flex-col" gap={Sp::XXS}>
                    <text class="text-lg font-semibold" color={tc.text_strong}>{title}</text>
                    <text class="text-sm" color={tc.text_muted}>{description}</text>
                </div>
                {body}
            </div>
        </div>
    }
}

fn appearance_section(state: &AppState, theme: &Theme) -> AnyElement {
    let theme_mode = state.settings.theme_mode;
    let theme_name = state.settings.theme_name.clone();
    let scale_pct = state.settings.ui_scale_pct;

    let mode_control = SegmentedControl::new(vec![
        SegmentedItem::new(
            "Dark",
            Action::SetThemeMode(ThemeMode::Dark),
            theme_mode == ThemeMode::Dark,
        ),
        SegmentedItem::new(
            "Light",
            Action::SetThemeMode(ThemeMode::Light),
            theme_mode == ThemeMode::Light,
        ),
    ])
    .into_any();

    let scale_options: [u16; 7] = [80, 90, 100, 110, 125, 150, 180];
    let scale_control = SegmentedControl::new(
        scale_options
            .iter()
            .map(|pct| {
                SegmentedItem::new(
                    format!("{pct}%"),
                    Action::SetUiScalePct(*pct),
                    *pct == scale_pct,
                )
            })
            .collect(),
    )
    .into_any();

    let theme_browse = Button::new(Action::OpenThemePicker)
        .icon(lucide::SUN)
        .label(if theme_name.is_empty() {
            "Browse themes\u{2026}".to_owned()
        } else {
            theme_name.clone()
        })
        .style(ButtonStyle::Subtle)
        .size(ButtonSize::Default)
        .into_any();

    section_card(
        theme,
        vec![
            setting_row(theme, "Mode", "Dark or light palette.", mode_control),
            setting_row(theme, "Theme", "Active color theme.", theme_browse),
            setting_row(theme, "UI Scale", "Density of UI elements.", scale_control),
        ],
    )
}

fn editor_section(state: &AppState, theme: &Theme) -> AnyElement {
    let wrap_enabled = state.editor.wrap_enabled.get(&state.store);
    let wrap_column = state.editor.wrap_column.get(&state.store);
    let layout = state.compare.layout.get(&state.store);
    let renderer = state.compare.renderer.get(&state.store);

    let wrap_toggle: AnyElement = toggle(wrap_enabled)
        .on_toggle(Action::ToggleWrap)
        .into_any();

    let wrap_column_options: [(u32, &'static str); 4] =
        [(0, "Auto"), (80, "80"), (100, "100"), (120, "120")];
    let wrap_column_control = SegmentedControl::new(
        wrap_column_options
            .iter()
            .map(|(value, label)| {
                SegmentedItem::new(*label, Action::SetWrapColumn(*value), *value == wrap_column)
            })
            .collect(),
    )
    .into_any();

    let layout_control = SegmentedControl::new(vec![
        SegmentedItem::new(
            "Unified",
            Action::SetLayoutMode(LayoutMode::Unified),
            layout == LayoutMode::Unified,
        ),
        SegmentedItem::new(
            "Split",
            Action::SetLayoutMode(LayoutMode::Split),
            layout == LayoutMode::Split,
        ),
    ])
    .into_any();

    let renderer_control = SegmentedControl::new(vec![
        SegmentedItem::new(
            "Git",
            Action::SetRenderer(RendererKind::Builtin),
            renderer == RendererKind::Builtin,
        ),
        SegmentedItem::new(
            "Difftastic",
            Action::SetRenderer(RendererKind::Difftastic),
            renderer == RendererKind::Difftastic,
        ),
    ])
    .into_any();

    section_card(
        theme,
        vec![
            setting_row(
                theme,
                "Diff algorithm",
                "Git's line-based diff or Difftastic's syntax-aware diff.",
                renderer_control,
            ),
            setting_row(
                theme,
                "Layout",
                "Unified or side-by-side diff.",
                layout_control,
            ),
            setting_row(
                theme,
                "Wrap lines",
                "Wrap long lines instead of horizontal scroll.",
                wrap_toggle,
            ),
            setting_row(
                theme,
                "Wrap column",
                "Column at which to wrap. Auto follows the viewport.",
                wrap_column_control,
            ),
        ],
    )
}

fn behavior_section(state: &AppState, theme: &Theme) -> AnyElement {
    let lines = state.settings.wheel_scroll_lines;
    let options: [u8; 5] = [1, 2, 3, 5, 7];
    let control = SegmentedControl::new(
        options
            .iter()
            .map(|n| {
                SegmentedItem::new(n.to_string(), Action::SetWheelScrollLines(*n), *n == lines)
            })
            .collect(),
    )
    .into_any();

    section_card(
        theme,
        vec![setting_row(
            theme,
            "Mouse wheel speed",
            "Lines scrolled per wheel notch.",
            control,
        )],
    )
}

fn about_section(theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let version = env!("CARGO_PKG_VERSION");

    section_card(
        theme,
        vec![
            view! { scale,
                <div class="flex-col" gap={Sp::XS}>
                    <text class="text-lg font-semibold" color={tc.text_strong}>{"Diffy"}</text>
                    <text class="text-sm" color={tc.text_muted}>
                        {format!("Version {version}")}
                    </text>
                    <text class="text-sm" color={tc.text_muted}>
                        {"Native GPU-accelerated Git diff viewer."}
                    </text>
                </div>
            }
            .into_any(),
        ],
    )
}

fn section_card(theme: &Theme, rows: Vec<AnyElement>) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    view! { scale,
        <div class="flex-col"
             bg={tc.surface}
             border={tc.border_variant}
             rounded={Rad::XL}
             p={Sp::LG}
             gap={Sp::MD}>
            {...rows}
        </div>
    }
}

fn setting_row(theme: &Theme, label: &str, description: &str, control: AnyElement) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    view! { scale,
        <div class="flex-row items-center" gap={Sp::LG} py={Sp::XS}>
            <div class="flex-col flex-1" min_w={0.0} gap={Sp::XXS}>
                <text class="text-sm font-medium" color={tc.text_strong}>{label}</text>
                <text class="text-xs" color={tc.text_muted}>{description}</text>
            </div>
            <div class="shrink-0">
                {control}
            </div>
        </div>
    }
}
