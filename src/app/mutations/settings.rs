use quark::style::BorderStyle;
use quark::view;
use quark_app::ViewContext;
use quark_components::switch;
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::text_input::text_editor_element;
use quark_ui::theme::Theme;

use super::MutationsController;
use super::controls::{self, Chrome, Control, Tone};
use crate::ai::{CliAuth, CliInfo, CliScan, CliTool, DEFAULT_STEERING_PROMPT, Provider};
use crate::app::commands::repo_picker::tilde;
use crate::app::text_targets::{
    SETTINGS_ANTHROPIC_KEY, SETTINGS_OPENAI_KEY, SETTINGS_STEERING_PROMPT,
};
use crate::app::tokens::{self, Sp, TextStyle};
use crate::contracts::commands::{CommandsAction, SettingsSnapshot};
use crate::contracts::mutations::MutationsAction;
use crate::platform::secrets::AiKeyKind;

const PROMPT_MIN_LINES: f32 = 4.0;
const PROMPT_MAX_LINES: f32 = 14.0;
const MASK: &str = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}";

struct Layout {
    gap: f32,
    pad: f32,
    row: f32,
    label: f32,
    radius: f32,
    body: f32,
    small: f32,
}

impl Layout {
    fn new(theme: &Theme) -> Self {
        let scale = tokens::ui_scale(theme);
        Self {
            gap: tokens::scaled(Sp::SM, scale),
            pad: tokens::scaled(Sp::MD, scale),
            row: theme.metrics.ui_row_height,
            label: tokens::scaled(Sp::XXXL * 2.25, scale),
            radius: theme.metrics.control_radius,
            body: TextStyle::Body.font_size(theme),
            small: TextStyle::BodySmall.font_size(theme),
        }
    }
}

fn card(title: &str, detail: &str, body: AnyElement, theme: &Theme, layout: &Layout) -> AnyElement {
    let tc = &theme.colors;
    let title = title.to_owned();
    let detail = detail.to_owned();
    view! {
        <div
            class="flex-col"
            gap={layout.gap}
            p={layout.pad}
            rounded={theme.metrics.panel_radius}
            bg={tc.panel}
            border={tc.border_variant}
        >
            <div class="flex-col">
                <text size={layout.body} class="font-semibold" color={tc.text_strong}>{title}</text>
                <text size={layout.small} color={tc.text_muted}>{detail}</text>
            </div>
            {body}
        </div>
    }
}

fn provider_row(current: Option<Provider>, chrome: &Chrome) -> AnyElement {
    let options = [
        ("automatic", "Automatic", None),
        ("openai", "OpenAI", Some(Provider::OpenAi)),
        ("anthropic", "Anthropic", Some(Provider::Anthropic)),
    ];
    let controls: Vec<Control> = options
        .into_iter()
        .map(|(id, label, provider)| {
            Control::new(
                format!("settings.ai.provider.{id}"),
                label,
                CommandsAction::SetAiProvider(provider),
                if current == provider {
                    Tone::Secondary
                } else {
                    Tone::Ghost
                },
            )
            .icon_if(current == provider, lucide::CHECK)
        })
        .collect();
    controls::row(controls, chrome)
}

fn cli_row(
    tool: CliTool,
    info: Option<&CliInfo>,
    scanning: bool,
    theme: &Theme,
    layout: &Layout,
) -> AnyElement {
    let tc = &theme.colors;
    let name = tool.label();
    let (status, tone) = match info.map(|info| info.auth) {
        None if scanning => ("Checking\u{2026}", tc.text_disabled),
        None => ("Not found", tc.text_disabled),
        Some(CliAuth::SignedIn) => ("Signed in", tc.text_accent),
        Some(CliAuth::SignedOut) => ("Signed out", tc.status_warning),
        Some(CliAuth::Unknown) => ("Installed", tc.text_muted),
    };
    let version = info
        .and_then(|info| info.version.clone())
        .unwrap_or_default();
    let home = dirs::home_dir();
    let location = info
        .map(|info| tilde(&info.program, home.as_deref()))
        .unwrap_or_default();
    let label = if version.is_empty() {
        format!("{name}, {status}")
    } else {
        format!("{name} {version}, {status}")
    };
    view! {
        <div
            class="flex-row items-center"
            h={layout.row}
            px={layout.gap}
            gap={layout.gap}
            rounded={layout.radius}
            bg={tc.editor_surface}
            border={tc.border}
            accessibility_role={AccessibilityRole::Group}
            aria-label={label}
            test_id={format!("settings.ai.cli.{}", tool.binary())}
        >
            <div min_w={layout.label} class="shrink-0">
                <text size={layout.small} class="whitespace-nowrap" color={tc.text_muted}>{name}</text>
            </div>
            <text size={layout.small} class="font-mono" color={tc.text}>{version}</text>
            <div class="flex-1 flex-row" min_w={0.0}>
                <text size={layout.small} class="font-mono truncate" color={tc.text_disabled}>{location}</text>
            </div>
            <text size={layout.small} class="whitespace-nowrap" color={tone}>{status}</text>
        </div>
    }
}

fn cli_body(
    enabled: bool,
    scan: Option<&CliScan>,
    scanning: bool,
    chrome: &Chrome,
    theme: &Theme,
    layout: &Layout,
) -> AnyElement {
    let tc = &theme.colors;
    let rescan = Control::new(
        "settings.ai.cli.rescan",
        if scanning {
            "Checking\u{2026}"
        } else {
            "Check again"
        },
        MutationsAction::ScanAiClis { force: true },
        Tone::Ghost,
    )
    .icon(lucide::REFRESH)
    .disabled(scanning);
    let rows: Vec<AnyElement> = CliTool::ALL
        .into_iter()
        .map(|tool| {
            cli_row(
                tool,
                scan.and_then(|scan| scan.find(tool)),
                scanning,
                theme,
                layout,
            )
        })
        .collect();
    let home = dirs::home_dir();
    let folder = crate::ai::cli::workdir()
        .map(|dir| {
            format!(
                "Runs in {} with tools off and no saved sessions.",
                tilde(&dir, home.as_deref())
            )
        })
        .unwrap_or_default();
    view! {
        <div class="flex-col" gap={layout.gap}>
            <div class="flex-row items-center" gap={layout.gap}>
                <div class="shrink-0" test_id="settings.ai.cli.toggle">
                    {switch(enabled).label("Use local CLI").on_toggle(CommandsAction::ToggleAiCli)}
                </div>
                <div class="flex-1" />
                {controls::row(vec![rescan], chrome)}
            </div>
            {...rows}
            <text size={layout.small} color={tc.text_muted}>{folder}</text>
        </div>
    }
}

struct KeyRow {
    kind: AiKeyKind,
    provider: Provider,
    focus: FocusId,
    stored: Option<String>,
    editing: bool,
}

fn key_row(
    row: KeyRow,
    mutations: &MutationsController,
    chrome: &Chrome,
    layout: &Layout,
    cx: &ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let tc = &theme.colors;
    let name = row.provider.label();
    let model = row.provider.model_id();
    let kind = row.kind;
    let slug = name.to_lowercase();
    if let (Some(last4), false) = (row.stored.as_ref(), row.editing) {
        let masked = format!("{MASK}{last4}");
        let edit = Control::new(
            format!("settings.ai.{slug}.replace"),
            format!("Replace {name} key"),
            MutationsAction::SetAiKeyEditing {
                kind,
                editing: true,
            },
            Tone::Ghost,
        )
        .icon_only(lucide::PENCIL);
        let remove = Control::new(
            format!("settings.ai.{slug}.remove"),
            format!("Remove {name} key"),
            MutationsAction::ClearAiKey { kind },
            Tone::Ghost,
        )
        .icon_only(lucide::X);
        let dot = layout.small * 0.5;
        return view! {
            <div
                class="flex-row items-center"
                h={layout.row}
                px={layout.gap}
                gap={layout.gap}
                rounded={layout.radius}
                bg={tc.editor_surface}
                border={tc.border}
                accessibility_role={AccessibilityRole::Group}
                aria-label={format!("{name} API key, saved, ending in {last4}")}
                test_id={format!("settings.ai.{slug}")}
            >
                <div w={layout.label} class="shrink-0">
                    <text size={layout.small} color={tc.text_muted}>{name}</text>
                </div>
                <text size={layout.small} class="font-mono" color={tc.text_muted}>{masked}</text>
                <text size={layout.small} color={tc.text_disabled}>{model}</text>
                <div class="flex-1" />
                <div w={dot} h={dot} class="rounded-full shrink-0" bg={tc.accent} />
                {controls::row(vec![edit, remove], chrome)}
            </div>
        };
    }
    let field = mutations.key_field(kind);
    let focused = cx.is_focused(row.focus);
    let typed = !field.text().trim().is_empty();
    let input = text_input(format!("{name} API key"), field.text())
        .field(field)
        .focus_target(row.focus)
        .focused(focused)
        .masked(true)
        .bare()
        .placeholder("Paste key\u{2026}")
        .w_full()
        .h(layout.row);
    let mut actions = Vec::new();
    if typed {
        actions.push(
            Control::new(
                format!("settings.ai.{slug}.save"),
                format!("Save {name} key"),
                MutationsAction::SaveAiKeyField(kind),
                Tone::Primary,
            )
            .shortcut("\u{21b5}"),
        );
    }
    if row.stored.is_some() {
        actions.push(Control::new(
            format!("settings.ai.{slug}.cancel"),
            "Cancel",
            MutationsAction::SetAiKeyEditing {
                kind,
                editing: false,
            },
            Tone::Ghost,
        ));
    }
    view! {
        <div
            class="flex-row items-center"
            h={layout.row}
            px={layout.gap}
            gap={layout.gap}
            rounded={layout.radius}
            border={if focused { tc.focus_border } else { tc.border }}
            border_style={BorderStyle::Dashed { dash: layout.gap * 0.5, gap: layout.gap * 0.5, offset: 0.0 }}
            test_id={format!("settings.ai.{slug}")}
            on:key:enter={MutationsAction::SaveAiKeyField(kind)}
        >
            <div w={layout.label} class="shrink-0">
                <text size={layout.small} color={tc.text_muted}>{name}</text>
            </div>
            <div class="flex-1 flex-row items-center" min_w={0.0} h={layout.row}>{input}</div>
            {controls::row(actions, chrome)}
        </div>
    }
}

pub fn clankers_settings(
    mutations: &mut MutationsController,
    settings: &SettingsSnapshot,
    width: f32,
    cx: &mut ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let tc = &theme.colors;
    let layout = Layout::new(theme);
    let chrome = Chrome::from_theme(theme);
    let line = layout.body * 1.35;
    let inner = (width - 4.0 * layout.pad).max(layout.pad);

    let now = cx.frame.elapsed().as_millis() as u64;
    let editor = mutations.steering_editor_mut();
    editor.set_font_size(layout.body);
    editor.set_clock(now);
    let prompt_h = editor.flush_fit(
        &mut cx.frame.text().system,
        inner,
        line * PROMPT_MIN_LINES,
        line * PROMPT_MAX_LINES,
    );

    let ai = mutations.ai();
    let last4 = |secret: &Option<crate::contracts::review::Secret>| {
        secret.as_ref().map(|secret| {
            let key = secret.expose().trim();
            key.chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>()
        })
    };
    let rows = [
        KeyRow {
            kind: AiKeyKind::Anthropic,
            provider: Provider::Anthropic,
            focus: SETTINGS_ANTHROPIC_KEY,
            stored: last4(&ai.anthropic),
            editing: ai.anthropic_editing,
        },
        KeyRow {
            kind: AiKeyKind::OpenAi,
            provider: Provider::OpenAi,
            focus: SETTINGS_OPENAI_KEY,
            stored: last4(&ai.openai),
            editing: ai.openai_editing,
        },
    ];
    let keys: Vec<AnyElement> = rows
        .into_iter()
        .map(|row| key_row(row, mutations, &chrome, &layout, cx))
        .collect();
    let keychain = view! {
        <div
            class="flex-row items-center"
            px={layout.gap}
            rounded={layout.radius}
            bg={tc.element_background}
        >
            <text size={layout.small} color={tc.text_muted}>"Keychain"</text>
        </div>
    };
    let keys_body = view! {
        <div class="flex-col" gap={layout.gap}>
            <div class="flex-row items-center" gap={layout.gap}>
                <icon svg={lucide::SPARKLES} size={chrome.icon} color={tc.text_accent} />
                <text size={layout.small} color={tc.text_muted}>"Saved keys stay in the system keychain."</text>
                <div class="flex-1" />
                {keychain}
            </div>
            {...keys}
        </div>
    };

    let custom = !settings.settings.ai_steering_prompt.trim().is_empty();
    let edited = mutations.steering_editor().text().trim();
    let saved = if edited == DEFAULT_STEERING_PROMPT.trim() {
        String::new()
    } else {
        edited.to_owned()
    };
    let focused = cx.is_focused(SETTINGS_STEERING_PROMPT);
    let prompt = text_editor_element(
        SETTINGS_STEERING_PROMPT,
        ScrollActionBuilder::new(|lines| MutationsAction::ScrollSteeringPrompt(lines).into()),
    )
    .editor_snapshot(mutations.steering_editor())
    .label("Commit steering prompt")
    .focused(focused)
    .font_size(layout.body)
    .text_color(tc.text)
    .w(inner)
    .h(prompt_h);
    let prompt_actions = controls::row(
        vec![
            Control::new(
                "settings.ai.prompt.save",
                "Save prompt",
                CommandsAction::SetAiSteeringPrompt(saved),
                Tone::Secondary,
            ),
            Control::new(
                "settings.ai.prompt.reset",
                "Reset to default",
                CommandsAction::SetAiSteeringPrompt(String::new()),
                Tone::Ghost,
            )
            .disabled(!custom),
        ],
        &chrome,
    );
    let prompt_body = view! {
        <div class="flex-col" gap={layout.gap}>
            <div
                p={layout.gap}
                rounded={layout.radius}
                bg={tc.editor_surface}
                border={if focused { tc.focus_border } else { tc.border }}
            >
                {prompt}
            </div>
            {prompt_actions}
        </div>
    };
    let prompt_detail = if custom {
        "Your prompt guides generated commit messages."
    } else {
        "The default prompt guides generated commit messages."
    };

    let use_cli = settings.settings.ai_use_cli;
    let cli = cli_body(
        use_cli,
        ai.cli.as_deref(),
        ai.scanning,
        &chrome,
        theme,
        &layout,
    );
    let provider_detail = if use_cli {
        "Automatic prefers Claude Code, then Codex, then saved keys."
    } else {
        "Automatic uses Anthropic when both keys are saved."
    };

    view! {
        <div
            class="flex-col"
            w={width}
            gap={layout.pad}
            accessibility_role={AccessibilityRole::Group}
            aria-label="AI"
            test_id="settings.clankers"
        >
            {card(
                "Provider",
                provider_detail,
                provider_row(settings.settings.ai_provider, &chrome),
                theme,
                &layout,
            )}
            {card(
                "Local CLI",
                "Use your Claude Code or Codex sign-in instead of an API key.",
                cli,
                theme,
                &layout,
            )}
            {card("AI keys", "Paste a key to enable commit message generation.", keys_body, theme, &layout)}
            {card("Commit steering prompt", prompt_detail, prompt_body, theme, &layout)}
        </div>
    }
}
