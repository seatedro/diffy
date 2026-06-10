use halogen::{SemanticRole, view};

use crate::actions::Action;
use crate::ai::stream::{ANTHROPIC_MODEL, OPENAI_MODEL};
use crate::core::compare::backends::DifftasticBackend;
use crate::core::compare::{LayoutMode, RendererKind};
use crate::editor::input_element::text_editor_element;
use crate::fonts::FontRole;
use crate::input::{
    ShortcutCommand, ShortcutEntry, active_bindings, binding_conflict, format_binding,
    override_for, shortcut_groups,
};
use crate::platform::secrets::AiKeyKind;
use crate::ui::components::{
    Button, ButtonSize, ButtonStyle, SegmentedControl, SegmentedItem, toggle,
};
use crate::ui::design::{Ico, Rad, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{AppState, FocusTarget, SettingsSection, UpdateState};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme, ThemeMode};

pub fn settings_page(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let active = state.ui.settings_section.get(&state.store);

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
    let nav_w = (Sz::SETTINGS_NAV_W * scale).round();

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
                {Button::new(crate::actions::SettingsAction::CloseSettings.into())
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
    let accessibility_label = section.label();

    view! { scale,
        <div class="flex-row items-center"
             id={format!("settings-section:{accessibility_label}")}
             key={accessibility_label.to_owned()}
             test_id={"settings-section"}
             semantic_role={SemanticRole::Tab}
             gap={Sp::SM}
             px={Sp::SM} py={Sp::XS + Sp::XXS}
             rounded={Rad::MD}
             bg={if selected { tc.element_selected } else { Color::TRANSPARENT }}
             hover_bg={if !selected { tc.ghost_element_hover }}
             on_click={crate::actions::SettingsAction::SetSettingsSection(section).into()}
             accessibility_role={accesskit::Role::Tab}
             accessibility_id={format!("settings-section:{accessibility_label}")}
             accessibility_label={accessibility_label}
             accessibility_selected={selected}
             cursor={CursorHint::Pointer}>
            <icon svg={section.icon()} size={Ico::SM * scale} color={icon_color} />
            <text class="text-sm font-medium" color={label_color}>{section.label()}</text>
        </div>
    }
}

fn section_content(state: &AppState, theme: &Theme, section: SettingsSection) -> AnyElement {
    if section == SettingsSection::Keymaps {
        return keymaps_layout(state, theme);
    }

    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let max_w = (Sz::SETTINGS_CONTENT_MAX_W * scale).round();

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
        SettingsSection::Keymaps => unreachable!(),
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
            <div class="flex-1 flex-col" min_h={0.0} max_w={max_w} gap={Sp::LG}>
                <div class="flex-col" gap={Sp::XXS}>
                    <text class="text-lg font-semibold" color={tc.text_strong}>{title}</text>
                    <text class="text-sm" color={tc.text_muted}>{description}</text>
                </div>
                {body}
            </div>
        </div>
    }
}

fn keymaps_layout(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let inner_max_w = (Sz::SETTINGS_KEYMAPS_MAX_W * scale).round();
    let capture = state.ui.keymap_capture.get(&state.store);
    let cx = &*state.store;

    let groups: Vec<AnyElement> = shortcut_groups()
        .iter()
        .map(|group| keymap_group(state, theme, group.title, group.entries, capture))
        .collect();

    view! { scale,
        <div class="flex-1 flex-col items-stretch overflow-hidden" min_h={0.0}>
            <div class="w-full flex-col"
                 pt={Sp::XL} pb={Sp::LG}
                 px={Sp::XXL}>
                <div class="flex-col" max_w={inner_max_w} gap={Sp::XXS}>
                    <text class="text-lg font-semibold" color={tc.text_strong}>{"Keymaps"}</text>
                    <text class="text-sm" color={tc.text_muted}>
                        {"Review and rebind keyboard shortcuts."}
                    </text>
                </div>
            </div>
            <div class="flex-1 flex-col w-full" min_h={0.0}
                 clip
                 scroll_y={@state.ui.keymaps_scroll_top_px}
                 scroll_total={@state.ui.keymaps_content_height_px}
                 on_scroll={ScrollActionBuilder::SettingsKeymaps}>
                <div class="flex-col"
                     px={Sp::XXL}
                     pb={Sp::XXXL}
                     max_w={inner_max_w}
                     gap={Sp::LG}>
                    {...groups}
                </div>
            </div>
        </div>
    }
    .into_any()
}

/// Content height of the keymaps scroll body, in scaled (logical) pixels.
/// Used by `shell::build_ui_frame` to clamp wheel scroll and to drive the
/// scrollbar thumb. Mirror any layout change in `keymap_group` / `keymap_row`
/// here, otherwise the scrollbar position drifts from real content.
pub fn keymaps_content_height(theme: &Theme) -> f32 {
    let m = &theme.metrics;
    let scale = m.ui_scale();

    // Row height is the max of:
    //   * compact button: icon (14) + 2*Sp::XXS padding = 18 base
    //   * description text-sm with ~1.4 line-height: ui_small_font_size * 1.4
    //   * key chip: text-xs (~ui_small - 1) line-box + 2px border
    // plus row vertical padding (2*Sp::XXS).
    let row_button_h = (Ico::BUTTON_COMPACT + 2.0 * Sp::XXS) * scale;
    let row_text_h = m.ui_small_font_size * 1.4;
    let row_chip_h = (m.ui_small_font_size - scale) * 1.4 + 2.0;
    let row_inner_h = row_button_h.max(row_text_h).max(row_chip_h);
    let row_h = row_inner_h + 2.0 * Sp::XXS * scale;

    // Header strip: text-xs line-box + 2*(Sp::XS + Sp::XXS) padding.
    let header_h = m.ui_small_font_size * 1.4 + 2.0 * (Sp::XS + Sp::XXS) * scale;
    let line_h = 1.0;
    let group_gap = Sp::LG * scale;
    let bottom_pad = Sp::XXXL * scale;

    let groups = shortcut_groups();
    let n_groups = groups.len() as f32;
    let mut total = bottom_pad + (n_groups - 1.0).max(0.0) * group_gap;

    for g in groups {
        let pair_count = g.entries.len().div_ceil(2) as f32;
        let body_h = pair_count * row_h + (pair_count - 1.0).max(0.0) * line_h;
        total += header_h + line_h + body_h;
    }

    // Small per-pair safety margin: line-height multipliers and chip padding
    // are estimates, and accumulated error from ~30 pairs pushes the last row
    // past the computed max_scroll. A few pixels per pair keeps the bottom
    // row fully on-screen without changing the rendered layout.
    let pair_total: f32 = groups
        .iter()
        .map(|g| g.entries.len().div_ceil(2))
        .sum::<usize>() as f32;
    total + pair_total * 2.0 * scale
}

fn keymap_group(
    state: &AppState,
    theme: &Theme,
    title: &str,
    entries: &'static [ShortcutEntry],
    capture: Option<ShortcutCommand>,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let pair_count = entries.len().div_ceil(2);
    let pairs: Vec<AnyElement> = entries
        .chunks(2)
        .enumerate()
        .map(|(idx, chunk)| {
            let left = keymap_row(state, theme, &chunk[0], capture);
            let right: AnyElement = if let Some(entry) = chunk.get(1) {
                keymap_row(state, theme, entry, capture)
            } else {
                view! { scale, <div class="flex-1" /> }.into_any()
            };
            let separator: Option<AnyElement> = if idx + 1 < pair_count {
                Some(
                    view! { scale,
                        <div class="w-full" h={1.0} bg={tc.border_variant}></div>
                    }
                    .into_any(),
                )
            } else {
                None
            };
            view! { scale,
                <div class="flex-col">
                    <div class="flex-row items-stretch">
                        <div class="flex-1" min_w={0.0}>{left}</div>
                        <div class="shrink-0" w={1.0} bg={tc.border_variant}></div>
                        <div class="flex-1" min_w={0.0}>{right}</div>
                    </div>
                    {?separator}
                </div>
            }
            .into_any()
        })
        .collect();

    view! { scale,
        <div class="flex-col"
             bg={tc.surface}
             border={tc.border_variant}
             rounded={Rad::MD}>
            <div class="flex-row items-center w-full"
                 px={Sp::MD} py={Sp::XS + Sp::XXS}
                 bg={tc.elevated_surface}>
                <text class="text-xs font-semibold mono" color={tc.accent}>{title.to_uppercase()}</text>
                <spacer />
                <text class="text-xs mono" color={tc.text_muted}>
                    {format!("{} {}", entries.len(), if entries.len() == 1 { "binding" } else { "bindings" })}
                </text>
            </div>
            <div class="w-full" h={1.0} bg={tc.border_variant}></div>
            <div class="flex-col">
                {...pairs}
            </div>
        </div>
    }
    .into_any()
}

fn key_chip(label: &str, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    view! { scale,
        <div class="shrink-0 items-center justify-center"
             px={Sp::XS + Sp::XXS} py={0.0}
             bg={tc.element_background}
             border={tc.border_variant}
             rounded={Rad::SM}>
            <text class="text-xs mono text-center" color={tc.text}>{label}</text>
        </div>
    }
    .into_any()
}

fn binding_chips(binding: &str, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let formatted = format_binding(binding);
    let parts: Vec<String> = formatted.split('+').map(|p| p.trim().to_owned()).collect();

    view! { scale,
        <div class="flex-row items-center" gap={Sp::XXS}>
            for (i, part) in parts.iter().enumerate() {
                <fragment>
                    if i > 0 {
                        <text class="text-xs mono" color={tc.text_muted}>{"+"}</text>
                    }
                    {key_chip(part, theme)}
                </fragment>
            }
        </div>
    }
    .into_any()
}

fn keymap_row(
    state: &AppState,
    theme: &Theme,
    entry: &ShortcutEntry,
    capture: Option<ShortcutCommand>,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let command = entry.command;
    let overrides = &state.settings.keymap_overrides;
    let active = active_bindings(overrides, command);
    let customized = override_for(overrides, command).is_some();
    let recording = capture == Some(command);
    let conflict = active
        .first()
        .and_then(|binding| binding_conflict(overrides, entry, binding))
        .map(|entry| format!("Conflicts with {}", entry.description));

    let description_color = if customized { tc.text_strong } else { tc.text };

    let binding_view: AnyElement = if recording {
        view! { scale,
            <text class="text-xs mono" color={tc.text_accent}>{"\u{2190} press keys"}</text>
        }
        .into_any()
    } else {
        let chips: Vec<AnyElement> = active
            .iter()
            .enumerate()
            .map(|(i, binding)| {
                view! { scale,
                    <div class="flex-row items-center" gap={Sp::XXS}>
                        if i > 0 {
                            <text class="text-xs mono" color={tc.text_muted}>{"/"}</text>
                        }
                        {binding_chips(binding, theme)}
                    </div>
                }
                .into_any()
            })
            .collect();
        view! { scale,
            <div class="flex-row items-center" gap={Sp::XS}>
                {...chips}
            </div>
        }
        .into_any()
    };

    let marker_text: &'static str = if conflict.is_some() {
        "!"
    } else if customized {
        "\u{2022}"
    } else {
        " "
    };
    let marker_color = if conflict.is_some() {
        tc.status_warning
    } else if customized {
        tc.accent
    } else {
        Color::TRANSPARENT
    };
    let marker_tooltip = conflict.clone().unwrap_or_else(|| {
        if customized {
            "Customized".to_owned()
        } else {
            String::new()
        }
    });

    let marker: AnyElement = view! { scale,
        <div class="shrink-0 items-center justify-center"
             w={Sp::MD} h={Sp::MD}
             tooltip={marker_tooltip}>
            <text class="text-xs mono font-semibold" color={marker_color}>{marker_text}</text>
        </div>
    }
    .into_any();

    let reset = Button::new(crate::actions::SettingsAction::ResetKeymapBinding(command).into())
        .icon(lucide::CORNER_UP_LEFT)
        .style(ButtonStyle::Ghost)
        .size(ButtonSize::Compact)
        .disabled(!customized)
        .tooltip("Reset binding")
        .into_any();

    let edit = Button::new(crate::actions::SettingsAction::BeginKeymapRebind(command).into())
        .icon(lucide::PENCIL)
        .style(if recording {
            ButtonStyle::Subtle
        } else {
            ButtonStyle::Ghost
        })
        .size(ButtonSize::Compact)
        .tooltip("Record shortcut")
        .into_any();

    let row_bg = if recording {
        tc.element_selected
    } else {
        Color::TRANSPARENT
    };
    let hover_bg = if recording {
        tc.element_selected
    } else {
        tc.element_background
    };

    view! { scale,
        <div class="flex-row items-center w-full"
             gap={Sp::XS}
             px={Sp::SM} py={Sp::XXS}
             bg={row_bg}
             hover_bg={hover_bg}>
            {marker}
            <div class="flex-1 overflow-hidden" min_w={0.0}>
                <text class="text-sm truncate" color={description_color}>{entry.description}</text>
            </div>
            {binding_view}
            {reset}
            {edit}
        </div>
    }
    .into_any()
}

fn appearance_section(state: &AppState, theme: &Theme) -> AnyElement {
    let theme_mode = state.settings.theme_mode;
    let theme_name = state.settings.theme_name.clone();
    let scale_pct = state.settings.ui_scale_pct;
    let ui_font_family = state.settings.fonts.ui_family.clone();
    let mono_font_family = state.settings.fonts.mono_family.clone();

    let mode_control = SegmentedControl::new(vec![
        SegmentedItem::new(
            "Dark",
            crate::actions::SettingsAction::SetThemeMode(ThemeMode::Dark).into(),
            theme_mode == ThemeMode::Dark,
        ),
        SegmentedItem::new(
            "Light",
            crate::actions::SettingsAction::SetThemeMode(ThemeMode::Light).into(),
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
                    crate::actions::SettingsAction::SetUiScalePct(*pct).into(),
                    *pct == scale_pct,
                )
            })
            .collect(),
    )
    .into_any();

    let theme_browse = Button::new(crate::actions::SettingsAction::OpenThemePicker.into())
        .icon(lucide::SUN)
        .label(if theme_name.is_empty() {
            "Browse themes\u{2026}".to_owned()
        } else {
            theme_name.clone()
        })
        .style(ButtonStyle::Subtle)
        .size(ButtonSize::Default)
        .into_any();

    let ui_font_control = font_family_control(FontRole::Ui, &ui_font_family, theme);
    let mono_font_control = font_family_control(FontRole::Mono, &mono_font_family, theme);

    section_card(
        theme,
        vec![
            setting_row(theme, "Mode", "Dark or light palette.", mode_control),
            setting_row(theme, "Theme", "Active color theme.", theme_browse),
            setting_row(
                theme,
                "UI font",
                "Typeface for app chrome.",
                ui_font_control,
            ),
            setting_row(
                theme,
                "Code font",
                "Typeface for diffs and code.",
                mono_font_control,
            ),
            setting_row(theme, "UI Scale", "Density of UI elements.", scale_control),
        ],
    )
}

fn font_family_control(role: FontRole, selected: &str, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let selected = crate::fonts::normalize_font_selection(role, selected);
    let label = crate::fonts::font_selection_label(&selected);
    let action: Action = match role {
        FontRole::Ui => crate::actions::SettingsAction::OpenUiFontPicker.into(),
        FontRole::Mono => crate::actions::SettingsAction::OpenMonoFontPicker.into(),
    };
    let icon = match role {
        FontRole::Ui => lucide::FILE,
        FontRole::Mono => lucide::TERMINAL,
    };
    let tooltip = match role {
        FontRole::Ui => "Choose UI font",
        FontRole::Mono => "Choose code font",
    };
    let control_w = (240.0 * scale).round();

    view! { scale,
        <div class="flex-row items-center shrink-0"
             w={control_w}
             gap={Sp::SM}
             px={Sp::MD} py={Sp::XS}
             bg={tc.element_background}
             hover_bg={tc.element_hover}
             rounded={Rad::XL}
             cursor={CursorHint::Pointer}
             tooltip={tooltip}
             on_click={action}>
            <icon svg={icon} size={Ico::BUTTON_DEFAULT} color={tc.text_muted} />
            <div class="flex-1" min_w={0.0}>
                {text(label).text_sm().medium().color(tc.text).truncate()}
            </div>
            <icon svg={lucide::CHEVRON_DOWN} size={Ico::XS} color={tc.text_muted} />
        </div>
    }
}

fn editor_section(state: &AppState, theme: &Theme) -> AnyElement {
    let wrap_enabled = state.editor.wrap_enabled.get(&state.store);
    let wrap_column = state.editor.wrap_column.get(&state.store);
    let layout = state.compare.layout.get(&state.store);
    let renderer = state.compare.renderer.get(&state.store);

    let wrap_toggle: AnyElement = toggle(wrap_enabled)
        .on_toggle(crate::actions::SettingsAction::ToggleWrap.into())
        .into_any();

    let wrap_column_options: [(u32, &'static str); 4] =
        [(0, "Auto"), (80, "80"), (100, "100"), (120, "120")];
    let wrap_column_control = SegmentedControl::new(
        wrap_column_options
            .iter()
            .map(|(value, label)| {
                SegmentedItem::new(
                    *label,
                    crate::actions::SettingsAction::SetWrapColumn(*value).into(),
                    *value == wrap_column,
                )
                .into()
            })
            .collect(),
    )
    .into_any();

    let layout_control = SegmentedControl::new(vec![
        SegmentedItem::new(
            "Unified",
            crate::actions::CompareAction::SetLayoutMode(LayoutMode::Unified).into(),
            layout == LayoutMode::Unified,
        ),
        SegmentedItem::new(
            "Split",
            crate::actions::CompareAction::SetLayoutMode(LayoutMode::Split).into(),
            layout == LayoutMode::Split,
        ),
    ])
    .into_any();

    let mut renderer_items = vec![SegmentedItem::new(
        "Git",
        crate::actions::CompareAction::SetRenderer(RendererKind::Builtin).into(),
        renderer == RendererKind::Builtin,
    )];
    if DifftasticBackend::is_available() {
        renderer_items.push(SegmentedItem::new(
            "Difftastic",
            crate::actions::CompareAction::SetRenderer(RendererKind::Difftastic).into(),
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
                SegmentedItem::new(
                    n.to_string(),
                    crate::actions::SettingsAction::SetWheelScrollLines(*n).into(),
                    *n == lines,
                )
                .into()
            })
            .collect(),
    )
    .into_any();

    let continuous_toggle = toggle(state.settings.continuous_scroll)
        .on_toggle(crate::actions::SettingsAction::ToggleContinuousScroll.into())
        .into_any();

    section_card(
        theme,
        vec![
            setting_row(
                theme,
                "Mouse wheel speed",
                "Lines scrolled per wheel notch.",
                control,
            ),
            setting_row(
                theme,
                "Continuous scroll",
                "Scroll past one file to flow into the next, like GitHub.",
                continuous_toggle,
            ),
        ],
    )
}

fn clankers_section(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let openai_focused = state
        .ui
        .focus
        .get(&state.store)
        .is_some_and(|t| t == FocusTarget::SettingsOpenAiKey);
    let anthropic_focused = state
        .ui
        .focus
        .get(&state.store)
        .is_some_and(|t| t == FocusTarget::SettingsAnthropicKey);
    let prompt_focused = state
        .ui
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

    let prompt_box_h = (160.0 * scale).round();

    let prompt_editor = text_editor_element()
        .placeholder("Custom steering for commit messages (optional)\u{2026}")
        .editor_snapshot(&state.steering_prompt_editor)
        .focused(prompt_focused)
        .focus_target(FocusTarget::SettingsSteeringPrompt)
        .font_size(theme.metrics.ui_small_font_size)
        .text_color(tc.text)
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
            .on_click(crate::actions::AppAction::SetFocus(Some(target)).into());
    } else {
        input = input.on_click(
            crate::actions::AiAction::SetAiKeyEditing {
                kind,
                editing: true,
            }
            .into(),
        );
    }

    let trailing: Option<AnyElement> = if !key_set {
        None
    } else if editing {
        Some(
            Button::new(
                crate::actions::AiAction::SetAiKeyEditing {
                    kind,
                    editing: false,
                }
                .into(),
            )
            .icon(lucide::CHECK)
            .tooltip("Save")
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Compact)
            .into_any(),
        )
    } else {
        Some(
            Button::new(
                crate::actions::AiAction::SetAiKeyEditing {
                    kind,
                    editing: true,
                }
                .into(),
            )
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
    let update_state = state.ui.update.get(&state.store);
    let auto_update_toggle = toggle(state.settings.auto_update)
        .on_toggle(crate::actions::SettingsAction::ToggleAutoUpdate.into())
        .into_any();

    let update_control = match &update_state {
        UpdateState::Checking => Button::new(Action::Noop)
            .icon(lucide::REFRESH)
            .label("Checking")
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Default)
            .into_any(),
        UpdateState::Available(update) => {
            Button::new(crate::actions::UpdateAction::InstallUpdate.into())
                .icon(lucide::ARROW_DOWN)
                .label(format!("Install {}", update.version))
                .style(ButtonStyle::Filled)
                .size(ButtonSize::Default)
                .into_any()
        }
        UpdateState::Downloading(_) => Button::new(Action::Noop)
            .icon(lucide::ARROW_DOWN)
            .label("Downloading")
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Default)
            .into_any(),
        UpdateState::ReadyToRestart(update) => {
            Button::new(crate::actions::UpdateAction::RestartToUpdate.into())
                .icon(lucide::REFRESH)
                .label(format!("Restart to update {}", update.update.version))
                .style(ButtonStyle::Filled)
                .size(ButtonSize::Default)
                .into_any()
        }
        UpdateState::Restarting(_) => Button::new(Action::Noop)
            .icon(lucide::REFRESH)
            .label("Restarting")
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Default)
            .into_any(),
        _ => Button::new(crate::actions::UpdateAction::CheckForUpdates.into())
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
        UpdateState::Downloading(update) => {
            format!("Downloading and verifying version {}.", update.version)
        }
        UpdateState::ReadyToRestart(update) => {
            format!(
                "Version {} is ready. Restart to update.",
                update.update.version
            )
        }
        UpdateState::Restarting(update) => {
            format!("Restarting to install version {}.", update.update.version)
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
            setting_row(
                theme,
                "Automatic updates",
                "Check quietly every hour.",
                auto_update_toggle,
            ),
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
