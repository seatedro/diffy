use halogen::view;

use crate::actions::Action;
use crate::ai::stream::{ANTHROPIC_MODEL, OPENAI_MODEL};
use crate::core::compare::backends::DifftasticBackend;
use crate::core::compare::{LayoutMode, RendererKind};
use crate::platform::secrets::AiKeyKind;
use crate::ui::components::{
    Button, ButtonSize, ButtonStyle, SegmentedControl, SegmentedItem, toggle,
};
use crate::ui::design::{Ico, Rad, Sp, Sz};
use crate::ui::editor_element::{CursorSnapshot, text_editor_element};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{AppState, FocusTarget, SettingsSection, UpdateState};
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
        SettingsSection::Clankers => (
            "Clankers",
            "AI assistance. Keys stay in your OS keyring; no telemetry.",
            clankers_section(state, theme),
        ),
        SettingsSection::About => (
            "About",
            "Diffy build information.",
            about_section(state, theme),
        ),
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

    let mut renderer_items = vec![SegmentedItem::new(
        "Git",
        Action::SetRenderer(RendererKind::Builtin),
        renderer == RendererKind::Builtin,
    )];
    if DifftasticBackend::is_available() {
        renderer_items.push(SegmentedItem::new(
            "Difftastic",
            Action::SetRenderer(RendererKind::Difftastic),
            renderer == RendererKind::Difftastic,
        ));
    }
    let renderer_control = SegmentedControl::new(renderer_items).into_any();

    let renderer_description = if DifftasticBackend::is_available() {
        "Git's line-based diff or Difftastic's syntax-aware diff."
    } else {
        "Git's line-based diff."
    };

    section_card(
        theme,
        vec![
            setting_row(
                theme,
                "Diff algorithm",
                renderer_description,
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

fn clankers_section(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let openai_focused = state
        .focus
        .get(&state.store)
        .is_some_and(|t| t == FocusTarget::SettingsOpenAiKey);
    let anthropic_focused = state
        .focus
        .get(&state.store)
        .is_some_and(|t| t == FocusTarget::SettingsAnthropicKey);
    let prompt_focused = state
        .focus
        .get(&state.store)
        .is_some_and(|t| t == FocusTarget::SettingsSteeringPrompt);

    let cursor = state.text_edit.cursor.get(&state.store);
    let anchor = state.text_edit.anchor.get(&state.store);
    let cursor_moved_at_ms = state.text_edit.cursor_moved_at_ms.get(&state.store);
    let input_h = (Sz::INPUT_LABELED * scale).round();

    let openai_row = ai_key_row(
        theme,
        scale,
        AiKeyKind::OpenAi,
        FocusTarget::SettingsOpenAiKey,
        format!("OpenAI API key  \u{2022}  {OPENAI_MODEL}"),
        "sk-\u{2026}",
        &state.ai_openai_key,
        state.ai_openai_editing,
        openai_focused,
        cursor,
        anchor,
        cursor_moved_at_ms,
        input_h,
    );
    let anthropic_row = ai_key_row(
        theme,
        scale,
        AiKeyKind::Anthropic,
        FocusTarget::SettingsAnthropicKey,
        format!("Anthropic API key  \u{2022}  {ANTHROPIC_MODEL}"),
        "sk-ant-\u{2026}",
        &state.ai_anthropic_key,
        state.ai_anthropic_editing,
        anthropic_focused,
        cursor,
        anchor,
        cursor_moved_at_ms,
        input_h,
    );

    let keys_card = view! { scale,
        <div class="flex-col"
             bg={tc.surface}
             border={tc.border_variant}
             rounded={Rad::XL}
             p={Sp::LG}
             gap={Sp::MD}>
            <div class="flex-col" gap={Sp::XXS}>
                <text class="text-sm font-medium" color={tc.text_strong}>{"API keys"}</text>
                <text class="text-xs" color={tc.text_muted}>
                    {"Anthropic is preferred when both are set. Leave blank to disable."}
                </text>
            </div>
            <div class="flex-col w-full" gap={Sp::SM}>
                {openai_row}
                {anthropic_row}
            </div>
        </div>
    }
    .into_any();

    let prompt_cursor = CursorSnapshot {
        x: state.steering_prompt_editor.cursor_pos.x,
        y: state.steering_prompt_editor.cursor_pos.y,
        moved_at_ms: state.steering_prompt_editor.cursor_moved_at_ms,
    };
    let prompt_selection = state.steering_prompt_editor.selection_rects();
    let prompt_box_h = (160.0 * scale).round();

    let prompt_editor = text_editor_element()
        .placeholder("Custom steering for commit messages (optional)\u{2026}")
        .is_empty(state.steering_prompt_editor.is_empty())
        .focused(prompt_focused)
        .focus_target(FocusTarget::SettingsSteeringPrompt)
        .editor_id(1)
        .font_size(theme.metrics.ui_small_font_size)
        .text_color(tc.text)
        .cursor(prompt_cursor)
        .selection(prompt_selection)
        .content_height(state.steering_prompt_editor.content_height())
        .scroll_y(state.steering_prompt_editor.scroll_y)
        .w_full()
        .flex_1();

    let prompt_card = view! { scale,
        <div class="flex-col"
             bg={tc.surface}
             border={tc.border_variant}
             rounded={Rad::XL}
             p={Sp::LG}
             gap={Sp::SM}>
            <div class="flex-col" gap={Sp::XXS}>
                <text class="text-sm font-medium" color={tc.text_strong}>{"Steering prompt"}</text>
                <text class="text-xs" color={tc.text_muted}>
                    {"Overrides the built-in commit-message prompt. Leave empty for the default."}
                </text>
            </div>
            <div class="flex-col w-full"
                 h={prompt_box_h}
                 rounded={Rad::LG}
                 border={tc.border_variant}
                 @when { prompt_focused } { border={tc.accent} }>
                <div class="flex-1 w-full" min_h={0.0} px={Sp::SM} py={Sp::XS}>
                    {prompt_editor}
                </div>
            </div>
        </div>
    }
    .into_any();

    view! { scale,
        <div class="flex-col" gap={Sp::LG}>
            {keys_card}
            {prompt_card}
        </div>
    }
    .into_any()
}

#[allow(clippy::too_many_arguments)]
fn ai_key_row(
    _theme: &Theme,
    scale: f32,
    kind: AiKeyKind,
    target: FocusTarget,
    label: String,
    placeholder: &'static str,
    value: &str,
    editing: bool,
    focused: bool,
    cursor: usize,
    anchor: usize,
    cursor_moved_at_ms: u64,
    input_h: f32,
) -> AnyElement {
    let key_set = !value.is_empty();
    let editable = !key_set || editing;

    let mut input = text_input(label, value.to_owned())
        .placeholder(placeholder)
        .focused(focused && editable)
        .cursor_moved_at(cursor_moved_at_ms)
        .masked(true)
        .flex_1()
        .h(input_h);
    if editable {
        input = input
            .focus_target(target)
            .cursor(if focused { cursor } else { 0 })
            .anchor(if focused { anchor } else { 0 })
            .on_click(Action::SetFocus(Some(target)));
    } else {
        input = input.on_click(Action::SetAiKeyEditing {
            kind,
            editing: true,
        });
    }

    let trailing: Option<AnyElement> = if !key_set {
        None
    } else if editing {
        Some(
            Button::new(Action::SetAiKeyEditing {
                kind,
                editing: false,
            })
            .icon(lucide::CHECK)
            .tooltip("Save")
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Compact)
            .into_any(),
        )
    } else {
        Some(
            Button::new(Action::SetAiKeyEditing {
                kind,
                editing: true,
            })
            .icon(lucide::PENCIL)
            .tooltip("Edit")
            .style(ButtonStyle::Ghost)
            .size(ButtonSize::Compact)
            .into_any(),
        )
    };

    view! { scale,
        <div class="flex-row items-center w-full" gap={Sp::SM}>
            {input.into_any()}
            {?trailing}
        </div>
    }
    .into_any()
}

fn about_section(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let version = crate::APP_VERSION;
    let update_state = state.update.get(&state.store);

    let update_control = match &update_state {
        UpdateState::Checking => Button::new(Action::Noop)
            .icon(lucide::REFRESH)
            .label("Checking")
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Default)
            .into_any(),
        UpdateState::Available(update) => Button::new(Action::InstallUpdate)
            .icon(lucide::ARROW_DOWN)
            .label(format!("Install {}", update.version))
            .style(ButtonStyle::Filled)
            .size(ButtonSize::Default)
            .into_any(),
        UpdateState::Installing(_) => Button::new(Action::Noop)
            .icon(lucide::ARROW_DOWN)
            .label("Installing")
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Default)
            .into_any(),
        _ => Button::new(Action::CheckForUpdates)
            .icon(lucide::REFRESH)
            .label("Check for updates")
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Default)
            .into_any(),
    };

    let update_description = match &update_state {
        UpdateState::Available(update) => {
            format!(
                "Version {} is ready for {}.",
                update.version, update.platform
            )
        }
        UpdateState::Installing(update) => {
            format!("Installing version {}. Diffy will restart.", update.version)
        }
        UpdateState::Failed(message) => format!("Update failed: {message}"),
        _ => "Signed cross-platform updates from Diffy's release channel.".to_owned(),
    };

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
            setting_row(theme, "Updates", &update_description, update_control),
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
