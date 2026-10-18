use quark::view;
use quark_app::ViewContext;
use quark_components::{Button, ButtonSize, ButtonStyle};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::text_input::text_editor_element;
use quark_ui::theme::Theme;

use super::MutationsController;
use crate::ai::Provider;
use crate::app::text_targets::{
    SETTINGS_ANTHROPIC_KEY, SETTINGS_OPENAI_KEY, SETTINGS_STEERING_PROMPT,
};
use crate::app::tokens::{self, Sp, TextStyle};
use crate::contracts::commands::SettingsSnapshot;
use crate::contracts::mutations::MutationsAction;
use crate::platform::secrets::AiKeyKind;

const PROMPT_MIN_LINES: f32 = 4.0;
const PROMPT_MAX_LINES: f32 = 14.0;

fn heading(label: &str, detail: &str, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    view! {
        <div class="flex-col" gap={tokens::scaled(Sp::XXS, tokens::ui_scale(theme))}>
            <text size={TextStyle::Body.font_size(theme)} class="font-semibold" color={tc.text_strong}>{label}</text>
            <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{detail}</text>
        </div>
    }
}

fn provider_row(current: Option<Provider>, gap: f32) -> AnyElement {
    let options = [
        ("Automatic", None),
        ("OpenAI", Some(Provider::OpenAi)),
        ("Anthropic", Some(Provider::Anthropic)),
    ];
    view! {
        <div class="flex-row items-center" gap={gap} role="radiogroup" aria-label="AI provider">
            for (label, provider) in options key={label} {
                <div>
                    {Button::new(MutationsAction::SetAiProvider(provider))
                        .label(label)
                        .active(current == provider)
                        .size(ButtonSize::Compact)}
                </div>
            }
        </div>
    }
}

struct KeyBlock {
    kind: AiKeyKind,
    provider: &'static str,
    focus: FocusId,
    stored: bool,
    editing: bool,
}

fn key_block(
    block: KeyBlock,
    mutations: &MutationsController,
    width: f32,
    cx: &ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let tc = &theme.colors;
    let gap = tokens::scaled(Sp::SM, tokens::ui_scale(theme));
    let small = TextStyle::BodySmall.font_size(theme);
    let label = format!("{} API key", block.provider);
    let kind = block.kind;
    if block.stored && !block.editing {
        return view! {
            <div class="flex-col" gap={gap}>
                <text size={TextStyle::Body.font_size(theme)} color={tc.text}>{label}</text>
                <div class="flex-row items-center" gap={gap}>
                    <text size={small} color={tc.text_muted}>"Saved in the system keychain"</text>
                    <div class="flex-1" />
                    {Button::new(MutationsAction::SetAiKeyEditing { kind, editing: true })
                        .label(format!("Replace {} key", block.provider))
                        .size(ButtonSize::Compact)}
                    {Button::new(MutationsAction::ClearAiKey { kind })
                        .label(format!("Remove {} key", block.provider))
                        .style(ButtonStyle::Danger)
                        .size(ButtonSize::Compact)}
                </div>
            </div>
        };
    }
    let field = mutations.key_field(kind);
    let focused = cx.is_focused(block.focus);
    let input = text_input(label.clone(), field.text())
        .field(field)
        .focus_target(block.focus)
        .focused(focused)
        .masked(true)
        .placeholder("Paste an API key");
    view! {
        <div class="flex-col" gap={gap}>
            <text size={TextStyle::Body.font_size(theme)} color={tc.text}>{label}</text>
            <div class="flex-row items-center" gap={gap}>
                <div w={(width * 0.6).max(0.0)}>{input}</div>
                {Button::new(MutationsAction::SaveAiKeyField(kind))
                    .label(format!("Save {} key", block.provider))
                    .style(ButtonStyle::Filled)
                    .size(ButtonSize::Compact)}
                if block.stored {
                    <div>
                        {Button::new(MutationsAction::SetAiKeyEditing { kind, editing: false })
                            .label("Cancel")
                            .size(ButtonSize::Compact)}
                    </div>
                }
            </div>
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
    let scale = tokens::ui_scale(theme);
    let gap = tokens::scaled(Sp::SM, scale);
    let section = tokens::scaled(Sp::XL, scale);
    let body = TextStyle::Body.font_size(theme);
    let line = body * 1.35;

    let now = cx.frame.elapsed().as_millis() as u64;
    let editor = mutations.steering_editor_mut();
    editor.set_font_size(body);
    editor.set_clock(now);
    let prompt_h = editor.flush_fit(
        &mut cx.frame.text().system,
        width,
        line * PROMPT_MIN_LINES,
        line * PROMPT_MAX_LINES,
    );

    let ai = mutations.ai();
    let blocks = [
        KeyBlock {
            kind: AiKeyKind::OpenAi,
            provider: "OpenAI",
            focus: SETTINGS_OPENAI_KEY,
            stored: ai.openai.is_some(),
            editing: ai.openai_editing,
        },
        KeyBlock {
            kind: AiKeyKind::Anthropic,
            provider: "Anthropic",
            focus: SETTINGS_ANTHROPIC_KEY,
            stored: ai.anthropic.is_some(),
            editing: ai.anthropic_editing,
        },
    ];
    let provider = provider_row(ai.provider, gap);
    let keys: Vec<AnyElement> = blocks
        .into_iter()
        .map(|block| key_block(block, mutations, width, cx))
        .collect();
    let custom = match &mutations.ai().steering {
        Some(prompt) => !prompt.is_empty(),
        None => !settings.settings.ai_steering_prompt.trim().is_empty(),
    };
    let prompt_note = if custom {
        "A custom prompt guides generated commit messages."
    } else {
        "The default prompt guides generated commit messages."
    };
    let focused = cx.is_focused(SETTINGS_STEERING_PROMPT);
    let prompt = text_editor_element(
        SETTINGS_STEERING_PROMPT,
        ScrollActionBuilder::new(|lines| MutationsAction::ScrollSteeringPrompt(lines).into()),
    )
    .editor_snapshot(mutations.steering_editor())
    .label("Commit steering prompt")
    .focused(focused)
    .font_size(body)
    .text_color(tc.text)
    .w(width)
    .h(prompt_h);

    view! {
        <div
            class="flex-col"
            w={width}
            gap={section}
            accessibility_role={AccessibilityRole::Group}
            aria-label="Clankers"
            test_id="settings.clankers"
        >
            <div class="flex-col" gap={gap}>
                {heading("Provider", "Which service writes commit messages. Automatic prefers Anthropic when both keys are saved.", theme)}
                {provider}
            </div>
            <div class="flex-col" gap={gap}>
                {heading("API keys", "Keys stay in the system keychain and are only sent to their provider.", theme)}
                {...keys}
            </div>
            <div class="flex-col" gap={gap}>
                {heading("Commit steering prompt", prompt_note, theme)}
                <div
                    class="rounded-md"
                    bg={tc.editor_surface}
                    border={if focused { tc.focus_border } else { tc.border }}
                >
                    {prompt}
                </div>
                <div class="flex-row items-center" gap={gap}>
                    {Button::new(MutationsAction::SaveSteeringPrompt)
                        .label("Save prompt")
                        .style(ButtonStyle::Filled)
                        .size(ButtonSize::Compact)}
                    {Button::new(MutationsAction::ResetSteeringPrompt)
                        .label("Reset to default")
                        .size(ButtonSize::Compact)}
                </div>
            </div>
        </div>
    }
}
