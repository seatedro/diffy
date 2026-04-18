use halogen::view;

use crate::actions::Action;
use crate::ui::components::button::{Button, ButtonSize, ButtonStyle};
use crate::ui::design::{Ico, Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::{AppState, AsyncStatus};
use crate::ui::style::Styled;

pub fn auth_modal(
    state: &AppState,
    theme: &crate::ui::theme::Theme,
    width: f32,
    height: f32,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let token_present = state.github.auth.token_present.get(&state.store);
    let device_flow = state.github.auth.device_flow.get(&state.store);
    let status = state.github.auth.status.get(&state.store);
    let last_error = state.last_error.get(&state.store);

    let phase = if token_present {
        AuthPhase::Success
    } else if let Some(flow) = device_flow.as_ref() {
        AuthPhase::Awaiting {
            user_code: flow.user_code.clone(),
            verification_uri: flow.verification_uri.clone(),
        }
    } else if status == AsyncStatus::Failed {
        AuthPhase::Failed(last_error.clone().unwrap_or_else(|| {
            "Couldn't start the GitHub sign-in flow.".to_owned()
        }))
    } else if status == AsyncStatus::Loading {
        AuthPhase::Starting
    } else {
        AuthPhase::Idle
    };

    let panel_width = Sz::MODAL_LG;
    let panel = view! { scale,
        <div class="flex-col overflow-hidden"
             w={panel_width} p={Sp::XXL} gap={Sp::XL}
             bg={tc.elevated_surface} rounded={Rad::XXXL}
             border_b={tc.border} shadow_preset={Shadow::MODAL}
             on_click={Action::Noop}>

            // Close affordance
            <div class="flex-row w-full justify-end" h={20.0}>
                <Button action={Action::CloseOverlay}
                        style={ButtonStyle::Ghost}
                        size={ButtonSize::Compact}>
                    <Icon>{lucide::X}</Icon>
                </Button>
            </div>

            // Header — centered brand mark + title + subtitle
            <div class="flex-col items-center" gap={Sp::MD}>
                <icon svg={lucide::GITHUB_MARK} size={44.0} color={tc.text_strong} />
                <text class="text-center" size={20.0} medium color={tc.text_strong}>{"Sign in to GitHub"}</text>
                <text class="text-sm text-center" color={tc.text_muted}>
                    {"Connect your account to load pull requests and review diffs."}
                </text>
            </div>

            {render_phase(phase, tc, scale)}
        </div>
    };

    view! { scale,
        <div class="absolute flex-col items-center justify-center"
             top={0.0} left={0.0}
             w={width} h={height}
             z_index={100}
             bg={tc.overlay_scrim}
             on_click={Action::CloseOverlay}
             hit_identity={HitIdentity::OverlayBackdrop}>
            {panel}
        </div>
    }
}

enum AuthPhase {
    Idle,
    Starting,
    Awaiting {
        user_code: String,
        verification_uri: String,
    },
    Failed(String),
    Success,
}

fn render_phase(
    phase: AuthPhase,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
) -> AnyElement {
    match phase {
        AuthPhase::Idle => render_idle(tc, scale),
        AuthPhase::Starting => render_starting(tc, scale),
        AuthPhase::Awaiting {
            user_code,
            verification_uri,
        } => render_awaiting(user_code, verification_uri, tc, scale),
        AuthPhase::Failed(message) => render_failed(message, tc, scale),
        AuthPhase::Success => render_success(tc, scale),
    }
}

fn render_idle(tc: &crate::ui::theme::ThemeColors, scale: f32) -> AnyElement {
    view! { scale,
        <div class="flex-col items-center w-full" gap={Sp::LG}>
            <div class="flex-row w-full justify-center">
                <Button action={Action::StartGitHubDeviceFlow}
                        tooltip={"Start GitHub device flow"}
                        style={ButtonStyle::Filled}>
                    <Icon>{lucide::KEY}</Icon>
                    <Label>{"Continue with GitHub"}</Label>
                </Button>
            </div>
            <text class="text-xs text-center" color={tc.text_muted}>
                {"We'll ask GitHub to generate a short, one-time device code."}
            </text>
        </div>
    }
}

fn render_starting(tc: &crate::ui::theme::ThemeColors, scale: f32) -> AnyElement {
    view! { scale,
        <div class="flex-col items-center w-full" gap={Sp::MD} py={Sp::LG}>
            <icon svg={lucide::LOADER} size={Ico::XXL} color={tc.text_accent} />
            <text class="text-sm text-center" color={tc.text_muted}>
                {"Contacting GitHub\u{2026}"}
            </text>
        </div>
    }
}

fn render_awaiting(
    user_code: String,
    verification_uri: String,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
) -> AnyElement {
    // Bigger, generous padding for the hero code box; slight letter-spacing
    // comes naturally from the font at this size.
    let code_for_copy = user_code.clone();
    view! { scale,
        <div class="flex-col w-full" gap={Sp::LG}>
            <div class="flex-col items-center" gap={Sp::XS}>
                <text class="text-sm text-center" color={tc.text_muted}>
                    {"Enter this code at"}
                </text>
                <text class="text-sm text-center" color={tc.text_accent}>
                    {&verification_uri}
                </text>
            </div>

            // Hero device-code card
            <div class="flex-row items-center justify-center w-full"
                 gap={Sp::MD} py={Sp::XL} px={Sp::LG}
                 rounded={Rad::XXL}
                 bg={tc.surface}
                 border_b={tc.border_variant}>
                <text class="mono bold text-center" size={28.0} color={tc.text_strong}>
                    {&user_code}
                </text>
                <Button action={Action::CopyText(code_for_copy)}
                        tooltip={"Copy code"}
                        style={ButtonStyle::Subtle}
                        size={ButtonSize::Compact}>
                    <Icon>{lucide::COPY}</Icon>
                </Button>
            </div>

            // Primary CTA — open the browser (we already auto-open once, but
            // give users an explicit way to reopen).
            <div class="flex-row w-full justify-center">
                <Button action={Action::OpenDeviceFlowBrowser}
                        tooltip={"Open github.com in your browser"}
                        style={ButtonStyle::Filled}>
                    <Icon>{lucide::EXTERNAL_LINK}</Icon>
                    <Label>{"Open GitHub in browser"}</Label>
                </Button>
            </div>

            <div class="flex-row items-center justify-center" gap={Sp::XS}>
                <icon svg={lucide::LOADER} size={Ico::XS} color={tc.text_muted} />
                <text class="text-xs text-center" color={tc.text_muted}>
                    {"Waiting for you to authorize\u{2026}"}
                </text>
            </div>
        </div>
    }
}

fn render_failed(
    message: String,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
) -> AnyElement {
    view! { scale,
        <div class="flex-col items-center w-full" gap={Sp::MD}>
            <div class="flex-row items-center w-full" gap={Sp::SM}
                 p={Sp::MD} rounded={Rad::XL}
                 bg={tc.surface}
                 border_b={tc.status_error}>
                <icon svg={lucide::ALERT_CIRCLE} size={Ico::SM} color={tc.status_error} />
                <text class="text-sm" color={tc.status_error}>{&message}</text>
            </div>
            <div class="flex-row w-full justify-center">
                <Button action={Action::StartGitHubDeviceFlow}
                        tooltip={"Retry device flow"}
                        style={ButtonStyle::Filled}>
                    <Icon>{lucide::REFRESH}</Icon>
                    <Label>{"Try again"}</Label>
                </Button>
            </div>
        </div>
    }
}

fn render_success(tc: &crate::ui::theme::ThemeColors, scale: f32) -> AnyElement {
    view! { scale,
        <div class="flex-col items-center w-full" gap={Sp::MD} py={Sp::LG}>
            <icon svg={lucide::CHECK} size={Ico::XXL} color={tc.accent} />
            <text class="text-sm text-center" color={tc.text}>
                {"Signed in. Finishing up\u{2026}"}
            </text>
        </div>
    }
}
