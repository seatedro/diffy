use halogen::view;

use crate::actions::Action;
use crate::core::compare::CompareMode;
use crate::ui::components::avatar::AvatarImage;
use crate::ui::components::{Button, avatar};
use crate::ui::design::{Ico, Rad, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{AppState, AsyncStatus, CompareField, WorkspaceMode, WorkspaceSource};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

pub(crate) fn title_bar(
    state: &AppState,
    theme: &Theme,
    sidebar_visible: f32,
    _window_width: f32,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let has_repo = state.compare.repo_path.with(&state.store, |p| p.is_some());
    let repo_loaded = state.repository.status.get(&state.store) == AsyncStatus::Ready;
    let is_ready = state.is_workspace_ready();

    let repo_label = state.compare.repo_path.with(&state.store, |p| {
        p.as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("diffy")
            .to_owned()
    });

    let sidebar_icon = if sidebar_visible > 0.5 {
        lucide::PANEL_LEFT_CLOSE
    } else {
        lucide::PANEL_LEFT_OPEN
    };

    let left_ref_value = state.compare.left_ref.get(&state.store);
    let right_ref_value = state.compare.right_ref.get(&state.store);
    let left_label = if left_ref_value.is_empty() {
        "base".to_owned()
    } else {
        left_ref_value.clone()
    };
    let right_label = if right_ref_value.is_empty() {
        "head".to_owned()
    } else if right_ref_value == crate::core::vcs::git::service::WORKDIR_REF {
        "working copy".to_owned()
    } else {
        right_ref_value.clone()
    };

    let (mode_label, mode_tooltip) = match state.compare.mode.get(&state.store) {
        CompareMode::SingleCommit => (
            "commit",
            "Single commit \u{2014} diff a commit against its parent",
        ),
        CompareMode::TwoDot => ("diff", "Diff \u{2014} compare two refs directly"),
        CompareMode::ThreeDot => (
            "merge",
            "Merge \u{2014} changes since the right ref diverged from the left",
        ),
    };

    let auth_user = state.github.auth.user.get(&state.store);
    let auth_loading = state.github.auth.status.get(&state.store) == AsyncStatus::Loading;
    let auth_avatar = state.github.auth.avatar.with(&state.store, |a| {
        a.as_ref().map(|b| AvatarImage {
            rgba: b.rgba.clone(),
            width: b.width,
            height: b.height,
            cache_key: b.cache_key,
        })
    });
    view! { scale,
        <div class="flex-row items-center" min_w={0.0}
             h={theme.metrics.title_bar_height} w_full
             px={Sp::XL}
             bg={tc.title_bar_background}
             border_b={tc.border_variant}>

            // left
            <div class="flex-1 flex-row items-center" min_w={0.0} gap={Sp::SM}>
                if is_ready {
                    <Button action={Action::ToggleSidebar}
                            tooltip={"Toggle sidebar (\u{2318}B)"}>
                        <Icon>{sidebar_icon}</Icon>
                    </Button>
                }
                if has_repo {
                    {ref_selector_button(
                        &repo_label,
                        lucide::FOLDER,
                        false,
                        Action::OpenRepoPicker,
                        "Switch repository",
                        tc,
                        scale,
                    )}
                } else {
                    <div class="flex-row items-center" gap={Sp::SM}>
                        <icon svg={lucide::GIT_COMPARE} size={Ico::LG} color={tc.accent} />
                        <div min_w={0.0}>
                            <text class="font-semibold truncate" color={tc.text_strong}>{"diffy"}</text>
                        </div>
                    </div>
                }
            </div>

            // center
            <div class="flex-shrink-0 flex-row items-center" gap={Sp::XS}>
                if has_repo && repo_loaded {
                    <div class="flex-row items-center" gap={Sp::SM}>
                        {ref_selector_button(
                            &left_label,
                            lucide::GIT_BRANCH,
                            left_ref_value.is_empty(),
                            Action::OpenRefPicker(CompareField::Left),
                            "Select base ref",
                            tc,
                            scale,
                        )}
                        <div px={Sp::SM} py={Sp::XS}
                             rounded={Rad::MD}
                             hover_bg={tc.ghost_element_hover}
                             on_click={Action::OpenCompareMenu}
                             cursor={CursorHint::Pointer}
                             tooltip={mode_tooltip}>
                            <text class="text-xs font-medium" color={tc.text_muted}>{mode_label}</text>
                        </div>
                        {ref_selector_button(
                            &right_label,
                            lucide::GIT_BRANCH,
                            right_ref_value.is_empty(),
                            Action::OpenRefPicker(CompareField::Right),
                            "Select head ref",
                            tc,
                            scale,
                        )}
                    </div>
                } else if state.workspace_mode.get(&state.store) == WorkspaceMode::Loading {
                    <text class="text-sm" color={tc.text_muted}>{"Comparing\u{2026}"}</text>
                }
            </div>

            // right
            <div class="flex-1 flex-row items-center justify-end" min_w={0.0} gap={Sp::XS}>
                if is_ready {
                    <Button action={Action::ShowWorkingTree}
                            active={state.workspace.source.get(&state.store) == WorkspaceSource::Status}
                            tooltip={"Show working tree changes"}>
                        <Icon>{lucide::FOLDER_GIT}</Icon>
                        <Label>{"Working tree"}</Label>
                    </Button>
                }
                {account_chip(auth_user.as_ref(), auth_avatar, auth_loading, tc, scale)}
            </div>
        </div>
    }
}

fn account_chip(
    user: Option<&crate::core::vcs::github::GitHubUser>,
    avatar_image: Option<AvatarImage>,
    loading: bool,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
) -> AnyElement {
    match user {
        Some(user) => {
            let display = format!("@{}", user.login);
            let tooltip = if user.name.is_empty() || user.name == user.login {
                format!("Signed in as @{}", user.login)
            } else {
                format!("Signed in as {} (@{})", user.name, user.login)
            };
            let avatar_name = if user.name.is_empty() {
                user.login.clone()
            } else {
                user.name.clone()
            };
            view! { scale,
                <div class="flex-row items-center"
                     gap={Sp::XS} px={Sp::SM} py={Sp::XS}
                     rounded={Rad::MD}
                     hover_bg={tc.ghost_element_hover}
                     on_click={Action::OpenAccountMenu}
                     cursor={CursorHint::Pointer}
                     tooltip={tooltip}>
                    {avatar(avatar_name).size(20.0).image(avatar_image)}
                    <text class="text-sm font-medium" color={tc.text_strong}>{display}</text>
                </div>
            }
        }
        None => {
            let (label, tooltip) = if loading {
                ("Signing in\u{2026}", "GitHub device flow in progress")
            } else {
                ("Sign in", "Sign in to GitHub")
            };
            view! { scale,
                <div class="flex-row items-center"
                     gap={Sp::XS} px={Sp::SM} py={Sp::XS}
                     rounded={Rad::MD}
                     hover_bg={tc.ghost_element_hover}
                     on_click={Action::StartGitHubDeviceFlow}
                     cursor={CursorHint::Pointer}
                     tooltip={tooltip}>
                    <icon svg={lucide::KEY} size={Ico::SM} color={tc.text_muted} />
                    <text class="text-sm font-medium" color={tc.text_strong}>{label}</text>
                </div>
            }
        }
    }
}

fn ref_selector_button(
    label: &str,
    icon: &'static str,
    is_placeholder: bool,
    action: Action,
    tooltip_text: &str,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
) -> AnyElement {
    let text_color = if is_placeholder {
        tc.text_muted
    } else {
        tc.text_strong
    };
    view! { scale,
        <div class="flex-row items-center"
             gap={Sp::XS} px={Sp::SM} py={Sp::XS}
             rounded={Rad::MD}
             hover_bg={tc.ghost_element_hover}
             on_click={action}
             cursor={CursorHint::Pointer}
             tooltip={tooltip_text}
             min_w={Sz::REF_SELECTOR_MIN_W}>
            <icon svg={icon} size={Ico::SM} color={tc.text_muted} />
            <div class="flex-1" min_w={0.0}>
                <text class="text-sm font-medium truncate" color={text_color}>{label}</text>
            </div>
            <icon svg={lucide::CHEVRON_DOWN} size={Ico::XS} color={tc.text_muted} />
        </div>
    }
}
