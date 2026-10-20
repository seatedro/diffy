use quark::view;
use quark_app::ViewContext;
use quark_components::{Button, ButtonSize, ButtonStyle, switch};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::theme::Theme;

use super::DeliveryController;
use super::state::UpdateState;
use crate::app::tokens::{self, Sp, TextStyle};
use crate::contracts::commands::{CommandsAction, SettingsSnapshot};
use crate::contracts::delivery::DeliveryAction;

fn row(title: &str, detail: &str, control: AnyElement, last: bool, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    view! {
        <div
            class="flex-row items-center w-full"
            gap={tokens::scaled(Sp::LG, scale)}
            px={tokens::scaled(Sp::MD, scale)}
            py={tokens::scaled(Sp::SM, scale)}
            @when {!last} { border_b={tc.border_variant} }
        >
            <div class="flex-1 flex-col" min_w={0.0} gap={tokens::scaled(Sp::XXS, scale)}>
                <text size={TextStyle::Body.font_size(theme)} color={tc.text}>{title}</text>
                <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{detail}</text>
            </div>
            <div class="shrink-0">{control}</div>
        </div>
    }
}

fn version_detail(update: &UpdateState) -> String {
    let current = format!("Diffy {}", crate::APP_VERSION);
    match update {
        UpdateState::Idle => current,
        UpdateState::Checking => format!("{current} · checking for updates"),
        UpdateState::Downloading(update) => {
            format!("{current} · downloading {}", update.version)
        }
        UpdateState::ReadyToRestart(staged) | UpdateState::Restarting(staged) => {
            format!("{current} · {} is ready", staged.update.version)
        }
        UpdateState::Failed(message) => format!("{current} · {message}"),
    }
}

pub fn updates_settings(
    delivery: &DeliveryController,
    settings: &SettingsSnapshot,
    width: f32,
    cx: &mut ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let update = delivery.update();
    let busy = matches!(
        update,
        UpdateState::Checking
            | UpdateState::Downloading(_)
            | UpdateState::ReadyToRestart(_)
            | UpdateState::Restarting(_)
    );
    let staged = match update {
        UpdateState::ReadyToRestart(staged) => Some(staged.update.version.clone()),
        _ => None,
    };
    let automatic = view! {
        <div>{switch(settings.settings.auto_update).label("Automatic updates").on_toggle(CommandsAction::ToggleAutoUpdate)}</div>
    };
    let check = view! {
        <Button
            on:click={DeliveryAction::CheckForUpdates}
            label="Check for updates"
            variant={ButtonStyle::Subtle}
            size={ButtonSize::Compact}
            disabled={busy}
        />
    };
    let restart = staged.as_ref().map(|version| {
        row(
            "Update ready",
            &format!("Diffy {version} is downloaded and verified"),
            view! {
                <Button
                    on:click={DeliveryAction::RestartToUpdate}
                    label="Restart to update"
                    variant={ButtonStyle::Filled}
                    size={ButtonSize::Compact}
                />
            },
            true,
            theme,
        )
    });
    view! {
        <div
            class="flex-col"
            w={width}
            gap={tokens::scaled(Sp::MD, scale)}
            accessibility_role={AccessibilityRole::Group}
            aria-label="Updates"
            test_id="settings.updates"
        >
            <div class="flex-col" gap={tokens::scaled(Sp::XXS, scale)}>
                <text size={TextStyle::Body.font_size(theme)} class="font-semibold" color={tc.text_strong}>"Updates"</text>
                <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>"Signed releases from Diffy's update channel."</text>
            </div>
            <div
                class="flex-col w-full overflow-hidden"
                rounded={theme.metrics.control_radius}
                bg={tc.elevated_surface}
                border={tc.border_variant}
            >
                {row("Automatic updates", "Check for new versions in the background", automatic, false, theme)}
                {row("Version", &version_detail(update), check, restart.is_none(), theme)}
                {?restart}
            </div>
        </div>
    }
}
