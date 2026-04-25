use halogen::view;

use crate::actions::Action;
use crate::ui::components::avatar::{AvatarImage, avatar};
use crate::ui::design::{Ico, Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::AppState;
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

pub fn account_menu(state: &AppState, theme: &Theme, width: f32, height: f32) -> AnyElement {
    let tc = &theme.colors;
    let m = &theme.metrics;
    let scale = m.ui_scale();

    let menu_w = (260.0 * scale).round();
    let menu_x = (width - menu_w - (Sp::XL * scale).round()).max(0.0);
    let menu_y = m.title_bar_height + (Sp::XS * scale).round();

    let user = state.github.auth.user.get(&state.store);
    let avatar_img = state.github.auth.avatar.with(&state.store, |a| {
        a.as_ref().map(|b| AvatarImage {
            rgba: b.rgba.clone(),
            width: b.width,
            height: b.height,
            cache_key: b.cache_key,
        })
    });

    let (login, display_name) = match user.as_ref() {
        Some(u) => {
            let display = if u.name.is_empty() || u.name == u.login {
                format!("@{}", u.login)
            } else {
                u.name.clone()
            };
            (format!("@{}", u.login), display)
        }
        None => (String::new(), "Not signed in".to_owned()),
    };

    view! { scale,
        <div class="absolute" left={0.0} top={0.0} w={width} h={height}
             z_index={200}
             bg={Color::TRANSPARENT}
             on_click={crate::actions::OverlayAction::CloseOverlay.into()}
             hit_identity={HitIdentity::OverlayBackdrop}>
            <div class="absolute flex-col overflow-hidden"
                 left={menu_x} top={menu_y}
                 w={menu_w}
                 py={Sp::XS}
                 bg={tc.elevated_surface}
                 border={tc.border}
                 rounded={Rad::XL}
                 shadow_preset={Shadow::DROPDOWN}
                 on_click={Action::Noop}>

                // User header
                <div class="flex-row items-center" gap={Sp::SM}
                     px={Sp::MD} py={Sp::SM}>
                    {avatar(login.clone().into_or_default("?".to_owned())).size(32.0).image(avatar_img)}
                    <div class="flex-col flex-1 overflow-hidden" min_w={0.0}>
                        <text class="text-sm font-medium truncate" color={tc.text_strong}>{&display_name}</text>
                        if !login.is_empty() && display_name != login {
                            <text class="text-xs truncate" color={tc.text_muted}>{&login}</text>
                        }
                    </div>
                </div>

                // Divider
                <div class="w-full" py={Sp::XS} px={Sp::SM}>
                    <div class="w-full" h={Sz::SEPARATOR_W} bg={tc.border_variant} />
                </div>

                {menu_row(lucide::SETTINGS, "Settings", crate::actions::SettingsAction::OpenSettings.into(), theme)}
                {menu_row(lucide::KEY, "Sign out", crate::actions::GitHubAction::SignOutGitHub.into(), theme)}
            </div>
        </div>
    }
}

fn menu_row(icon: &'static str, label: &str, action: Action, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let icon_size = (Ico::SM * scale).round();

    view! { scale,
        <div class="w-full flex-row items-center" gap={Sp::SM}
             px={Sp::MD} py={Sp::XS + Sp::XXS}
             rounded={Rad::MD}
             hover_bg={tc.sidebar_row_hover}
             on_click={action}
             cursor={CursorHint::Pointer}>
            <icon svg={icon} size={icon_size} color={tc.text_muted} />
            <text class="text-sm" color={tc.text}>{label}</text>
        </div>
    }
}

trait OrDefault {
    fn into_or_default(self, default: String) -> String;
}

impl OrDefault for String {
    fn into_or_default(self, default: String) -> String {
        if self.is_empty() { default } else { self }
    }
}
