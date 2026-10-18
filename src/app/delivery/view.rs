use diffy_syntax::{GrammarState, GrammarStatus};
use quark::view;
use quark_app::ViewContext;
use quark_components::{Button, ButtonSize, ButtonStyle, popover_panel};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::Theme;

use super::DeliveryController;
use super::state::UpdateState;
use crate::app::shell::Slot;
use crate::app::tokens::{self, Sp, Sz, TextStyle};
use crate::contracts::delivery::DeliveryAction;
use crate::core::syntax::grammar;

const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

pub fn view(delivery: &mut DeliveryController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let theme = cx.theme;
    let scale = tokens::ui_scale(theme);
    let failed = delivery.failed_grammars().next().is_some();
    let restart = matches!(delivery.update(), UpdateState::ReadyToRestart(_));
    let open = delivery.languages_open();
    view! {
        <div
            h={slot.height}
            class="flex-row items-center shrink-0"
            gap={tokens::scaled(Sp::XS, scale)}
            test_id="status.delivery"
        >
            if restart {
                <Button
                    on:click={DeliveryAction::RestartToUpdate}
                    label="Restart to update"
                    icon={lucide::REFRESH}
                    variant={ButtonStyle::Filled}
                    size={ButtonSize::Compact}
                />
            }
            <Button
                on:click={DeliveryAction::ToggleLanguages}
                label="Syntax languages"
                icon={if failed { lucide::ALERT_CIRCLE } else { lucide::CODE }}
                active={open}
                size={ButtonSize::Compact}
            />
            <text size={TextStyle::Caption.font_size(theme)} color={theme.colors.text_muted}>
                {VERSION}
            </text>
        </div>
    }
}

pub fn overlay(
    delivery: &mut DeliveryController,
    _slot: &Slot,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    if !delivery.languages_open() {
        return None;
    }
    let theme = cx.theme;
    let scale = tokens::ui_scale(theme);
    let rows: Vec<AnyElement> = grammar::languages()
        .iter()
        .map(|language| {
            let status = delivery
                .grammars()
                .iter()
                .find(|status| status.language == language.name);
            language_row(language.name, status, theme)
        })
        .collect();
    Some(
        view! {
            <div
                class="absolute"
                right={tokens::scaled(Sp::MD, scale)}
                bottom={theme.metrics.status_bar_height + tokens::scaled(Sp::XS, scale)}
                z_index={200}
            >
                <popover_panel(theme)
                    class="relative"
                    w={tokens::scaled(Sz::TOAST_MAX_W, scale)}
                    p={tokens::scaled(Sp::SM, scale)}
                    gap={tokens::scaled(Sp::XS, scale)}
                    accessibility_role={AccessibilityRole::Group}
                    aria-label="Syntax languages"
                    test_id="delivery.languages"
                >
                    {update_row(delivery.update(), theme)}
                    {...rows}
                </popover_panel>
            </div>
        }
        .into_any(),
    )
}

fn update_row(update: &UpdateState, theme: &Theme) -> AnyElement {
    let (detail, button) = match update {
        UpdateState::Idle => (String::new(), check_button(false)),
        UpdateState::Checking => ("Checking for updates".to_owned(), check_button(true)),
        UpdateState::Downloading(update) => (
            format!("Downloading Diffy {}", update.version),
            check_button(true),
        ),
        UpdateState::ReadyToRestart(staged) => (
            format!("Diffy {} is ready", staged.update.version),
            view! {
                <Button
                    on:click={DeliveryAction::RestartToUpdate}
                    label="Restart to update"
                    variant={ButtonStyle::Filled}
                    size={ButtonSize::Compact}
                />
            }
            .into_any(),
        ),
        UpdateState::Restarting(staged) => (
            format!("Installing Diffy {}", staged.update.version),
            check_button(true),
        ),
        UpdateState::Failed(message) => (message.clone(), check_button(false)),
    };
    view! {
        <div class="flex-row items-center w-full">
            <div class="flex-1 flex-col" min_w={0.0}>
                <text size={TextStyle::BodySmall.font_size(theme)} color={theme.colors.text}>
                    {VERSION}
                </text>
                if !detail.is_empty() {
                    <text
                        size={TextStyle::Caption.font_size(theme)}
                        class="truncate"
                        color={theme.colors.text_muted}
                    >
                        {detail}
                    </text>
                }
            </div>
            {button}
        </div>
    }
    .into_any()
}

fn check_button(disabled: bool) -> AnyElement {
    view! {
        <Button
            on:click={DeliveryAction::CheckForUpdates}
            label="Check for updates"
            variant={ButtonStyle::Subtle}
            size={ButtonSize::Compact}
            disabled={disabled}
        />
    }
    .into_any()
}

fn language_row(
    language: &'static str,
    status: Option<&GrammarStatus>,
    theme: &Theme,
) -> AnyElement {
    let state = status.map(|status| &status.state);
    let (label, color) = match state {
        Some(GrammarState::Ready | GrammarState::Installed) => ("Ready", theme.colors.text_muted),
        Some(GrammarState::Downloading) => ("Downloading", theme.colors.text_muted),
        Some(GrammarState::Failed(_)) => ("Download failed", theme.colors.status_error),
        None => ("Not loaded", theme.colors.text_muted),
    };
    let reason = match state {
        Some(GrammarState::Failed(reason)) => Some(reason.clone()),
        _ => None,
    };
    let failed = reason.is_some();
    view! {
        <div
            class="flex-row items-center w-full"
            accessibility_role={AccessibilityRole::Row}
            aria-label={language}
            @when {failed} { tooltip={reason.clone().unwrap_or_default()} }
            test_id={format!("delivery.language.{language}")}
        >
            <div class="flex-1" min_w={0.0}>
                <text size={TextStyle::BodySmall.font_size(theme)} color={theme.colors.text}>
                    {language}
                </text>
            </div>
            <text size={TextStyle::Caption.font_size(theme)} color={color}>
                {label}
            </text>
            if failed {
                <Button
                    on:click={DeliveryAction::RetryGrammar { language: language.to_owned() }}
                    label="Retry download"
                    size={ButtonSize::Compact}
                />
            }
        </div>
    }
    .into_any()
}
