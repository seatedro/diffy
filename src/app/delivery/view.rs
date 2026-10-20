use diffy_syntax::{GrammarState, GrammarStatus};
use quark::view;
use quark_app::ViewContext;
use quark_components::{Button, ButtonSize, ButtonStyle, popover_divider, popover_panel};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::Theme;

use super::DeliveryController;
use super::state::UpdateState;
use crate::app::shell::Slot;
use crate::app::tokens::{self, Ico, Layout, Sp, Sz, TextStyle};
use crate::contracts::delivery::DeliveryAction;

const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
pub const LANGUAGES_TOGGLE: FocusId = FocusId::from_key("diffy.delivery.languages");

pub fn view(delivery: &mut DeliveryController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let theme = cx.theme;
    let scale = tokens::ui_scale(theme);
    let failed = delivery.failed_grammars().next().is_some();
    let restart = matches!(delivery.update(), UpdateState::ReadyToRestart(_));
    let open = delivery.languages_open();
    let tc = &theme.colors;
    let small = TextStyle::BodySmall.font_size(theme);
    let language = slot
        .shared
        .selected_file
        .and_then(|selected| {
            let manifest = slot.shared.manifest.as_ref()?;
            let file = manifest
                .files
                .iter()
                .find(|file| file.key == selected.key)?;
            Some(
                delivery
                    .language_for_path(&file.paths.display_path())
                    .map_or_else(|| "Plain text".to_owned(), display_name),
            )
        })
        .unwrap_or_else(|| "Languages".to_owned());
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
            <div
                class="shrink-0"
                focus_ring={LANGUAGES_TOGGLE}
                on:key:enter={DeliveryAction::ToggleLanguages}
                on:key:space={DeliveryAction::ToggleLanguages}
                on:key:escape={DeliveryAction::CloseLanguages}
                test_id="delivery.languages.toggle"
            >
                <div
                    class="flex-row items-center"
                    h={tokens::scaled(Layout::CONTROL_SM_H, scale)}
                    px={tokens::scaled(Sp::XS, scale)}
                    gap={tokens::scaled(Sp::XS, scale)}
                    rounded={theme.metrics.control_radius}
                    hover_bg={tc.ghost_element_hover}
                    @when {open} { bg={tc.ghost_element_selected} }
                    cursor={CursorHint::Pointer}
                    role="button"
                    aria-label="Syntax languages"
                    tooltip="Syntax languages"
                    on:click={DeliveryAction::ToggleLanguages}
                >
                    if failed {
                        <icon svg={lucide::ALERT_CIRCLE} size={Ico::XS} color={tc.status_warning} />
                    }
                    <text size={small} color={tc.text_muted}>{language}</text>
                </div>
            </div>
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
    let (width, height) = cx.frame.size();
    let rows: Vec<AnyElement> = delivery
        .grammars()
        .iter()
        .map(|status| language_row(status, theme))
        .collect();
    let has_rows = !rows.is_empty();
    let available = match delivery.catalog().languages {
        0 => "Grammars download the first time a file needs them.".to_owned(),
        count => {
            format!("{count} languages available. Each downloads the first time a file needs it.")
        }
    };
    Some(
        view! {
            <div class="absolute" left={0.0} top={0.0} w={width} h={height} z_index={199}>
                <div
                    class="absolute"
                    left={0.0}
                    top={0.0}
                    w={width}
                    h={height}
                    block_mouse
                    on:click={DeliveryAction::CloseLanguages}
                    role="button"
                    aria-label="Close syntax languages"
                    test_id="delivery.languages.scrim"
                />
                <div
                    class="absolute"
                    right={tokens::scaled(Sp::MD, scale)}
                    bottom={theme.metrics.status_bar_height + tokens::scaled(Sp::XS, scale)}
                    z_index={200}
                    block_mouse
                    on:click={NoopAction}
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
                    <div class="flex-col w-full" gap={tokens::scaled(Sp::XXS, scale)}>
                        <text
                            size={TextStyle::Body.font_size(theme)}
                            class="font-semibold"
                            color={theme.colors.text_strong}
                        >
                            "Syntax languages"
                        </text>
                        <text size={TextStyle::Caption.font_size(theme)} color={theme.colors.text_muted}>
                            {available}
                        </text>
                    </div>
                    {popover_divider(theme)}
                    if has_rows {
                        <div
                            class="flex-col w-full overflow-y-scroll scrollbar-auto-hide"
                            max_h={tokens::scaled(Layout::LANGUAGE_LIST_MAX_H, scale)}
                            gap={tokens::scaled(Sp::XS, scale)}
                            track_scroll={delivery.languages_scroll()}
                            test_id="delivery.languages.list"
                        >
                            {...rows}
                        </div>
                        {popover_divider(theme)}
                    }
                    {update_row(delivery.update(), theme)}
                </popover_panel>
                </div>
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

fn language_row(status: &GrammarStatus, theme: &Theme) -> AnyElement {
    let language = status.language.as_str();
    let (label, color) = match &status.state {
        GrammarState::Ready => ("Ready", theme.colors.text),
        GrammarState::Installed => ("Installed", theme.colors.text_muted),
        GrammarState::Downloading => ("Downloading", theme.colors.text_muted),
        GrammarState::Failed(_) => ("Failed", theme.colors.status_error),
    };
    let reason = match &status.state {
        GrammarState::Failed(reason) => Some(reason.clone()),
        _ => None,
    };
    let failed = reason.is_some();
    view! {
        <div
            class="flex-col w-full"
            min_w={0.0}
            accessibility_role={AccessibilityRole::Row}
            aria-label={display_name(language)}
            @when {failed} { tooltip={reason.clone().unwrap_or_default()} }
            test_id={format!("delivery.language.{language}")}
        >
            <div class="flex-row items-center w-full" gap={tokens::scaled(Sp::SM, tokens::ui_scale(theme))}>
                <div class="flex-1" min_w={0.0}>
                    <text size={TextStyle::BodySmall.font_size(theme)} color={theme.colors.text}>
                        {display_name(language)}
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
            if let Some(reason) = reason.clone() {
                <div class="w-full overflow-hidden" min_w={0.0}>
                    <text
                        size={TextStyle::Caption.font_size(theme)}
                        class="truncate"
                        color={theme.colors.text_muted}
                    >
                        {reason}
                    </text>
                </div>
            }
        </div>
    }
    .into_any()
}

fn display_name(language: &str) -> String {
    let known = match language {
        "c_sharp" => "C#",
        "cpp" => "C++",
        "css" => "CSS",
        "html" => "HTML",
        "javascript" => "JavaScript",
        "json" => "JSON",
        "objc" => "Objective-C",
        "php" => "PHP",
        "sql" => "SQL",
        "toml" => "TOML",
        "tsx" => "TSX",
        "typescript" => "TypeScript",
        "xml" => "XML",
        "yaml" => "YAML",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_owned();
    }
    let mut chars = language.chars();
    chars.next().map_or_else(String::new, |first| {
        first
            .to_uppercase()
            .chain(chars.map(|c| if c == '_' { ' ' } else { c }))
            .collect()
    })
}
