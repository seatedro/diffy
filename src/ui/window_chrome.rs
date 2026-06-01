use halogen::view;

use crate::actions::{Action, ResizeEdge, WindowAction};
use crate::ui::components::avatar::AvatarImage;
use crate::ui::components::{Button, ButtonSize, ButtonStyle, avatar};
use crate::ui::design::{Ico, Rad, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{AppState, AsyncStatus, OverlaySurface, UpdateState, WorkspaceSource};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme, ThemeColors};

const ON_MACOS: bool = cfg!(target_os = "macos");

pub(crate) fn window_chrome(
    state: &AppState,
    theme: &Theme,
    sidebar_visible: f32,
    is_maximized: bool,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let bar_h = theme.metrics.title_bar_height;
    let drag_action: Action = WindowAction::BeginDrag.into();

    let is_text_compare = state.workspace.source.get(&state.store) == WorkspaceSource::TextCompare;
    let has_repo = !is_text_compare && state.compare.repo_path.with(&state.store, |p| p.is_some());
    let is_ready = state.is_workspace_ready();
    let ref_picker_open = state.overlays_top() == Some(OverlaySurface::RefPicker);
    let repo_label = state.compare.repo_path.with(&state.store, |p| {
        p.as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("diffy")
            .to_owned()
    });
    let supports_pr_preview = state
        .repository
        .capabilities
        .with(&state.store, |capabilities| {
            capabilities.is_some_and(|capabilities| capabilities.github_pull_requests)
        });
    let right_ref = state.compare.right_ref.get(&state.store);
    let pr_preview_active = supports_pr_preview
        && state.workspace.source.get(&state.store) == WorkspaceSource::Compare
        && state.compare.mode.get(&state.store) == crate::core::compare::CompareMode::ThreeDot
        && state.repository.location.with(&state.store, |location| {
            crate::ui::vcs::profile(location.as_ref()).is_working_copy_ref(&right_ref)
        });
    // When a PR is actively open for review, the "PR preview" affordance is replaced by
    // a compact direct link to that pull request.
    let pr_open = state.pull_request_review_enabled();
    let pr_button = if pr_open {
        state.active_pull_request_key().map(|(_, _, number)| {
            (
                format!("#{number}"),
                format!("Open PR #{number} on github.com"),
            )
        })
    } else {
        None
    };
    let sidebar_icon = if sidebar_visible > 0.5 {
        lucide::PANEL_LEFT_CLOSE
    } else {
        lucide::PANEL_LEFT_OPEN
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

    let cluster = crate::ui::title_bar::compare_cluster_view(state, theme);

    let bar_border_b = if ref_picker_open {
        Color::TRANSPARENT
    } else {
        tc.border_variant
    };

    // Traffic-light inset is a *logical-point* value; halogen's `w` is raw
    // physical pixels, so multiply by dpi (recovered as combined_scale /
    // ui_scale_factor) to clear the buttons on Retina too.
    let dpi = scale / state.ui_scale_factor().max(0.01);
    let traffic_inset = Sz::CHROME_TRAFFIC_INSET * dpi;

    view! { scale,
        <div class="flex-row items-center" min_w={0.0}
             h={bar_h} w_full
             px={Sp::MD}
             bg={tc.title_bar_background}
             border_b={bar_border_b}
             on_click={drag_action}
             cursor={CursorHint::Default}>

            // left — leading inset / app mark, sidebar toggle, repo chip
            <div class="flex-1 flex-row items-center" min_w={0.0} gap={Sp::XS}>
                if ON_MACOS {
                    <div w={traffic_inset} />
                } else {
                    <div class="flex-row items-center" px={Sp::SM}>
                        <icon svg={lucide::GIT_COMPARE} size={Ico::SM} color={tc.accent} />
                    </div>
                }
                if is_ready && !is_text_compare {
                    {chrome_icon_button(
                        sidebar_icon,
                        crate::actions::FileListAction::ToggleSidebar.into(),
                        "Toggle sidebar (\u{2318}B)",
                        tc,
                        scale,
                    )}
                }
                if has_repo {
                    {repo_chip(&repo_label, tc, scale)}
                }
            </div>

            // center — compare cluster
            <div class="flex-shrink-0 flex-row items-center" gap={Sp::XS}>
                if let Some(c) = cluster {
                    {c}
                }
            </div>

            // right — working tree, update, account, window controls
            <div class="flex-1 flex-row items-center justify-end" min_w={0.0} gap={Sp::XS}>
                if is_ready && !is_text_compare {
                    if let Some((pr_label, pr_tooltip)) = pr_button {
                        <Button action={crate::actions::GitHubAction::OpenPullRequestInBrowser.into()}
                                size={ButtonSize::Compact}
                                tooltip={pr_tooltip}>
                            <Icon>{lucide::EXTERNAL_LINK}</Icon>
                            <Label>{pr_label}</Label>
                        </Button>
                    } else if supports_pr_preview {
                        <Button action={crate::actions::CompareAction::PreviewPullRequest.into()}
                                active={pr_preview_active}
                                size={ButtonSize::Compact}
                                tooltip={"Preview PR with working tree edits"}>
                            <Icon>{lucide::GIT_PULL_REQUEST}</Icon>
                            <Label>{"PR preview"}</Label>
                        </Button>
                    }
                    <Button action={crate::actions::WorkspaceAction::ShowWorkingTree.into()}
                            active={state.workspace.source.get(&state.store) == WorkspaceSource::Status}
                            size={ButtonSize::Compact}
                            tooltip={"Show working tree changes"}>
                        <Icon>{lucide::FOLDER_GIT}</Icon>
                        <Label>{"Working tree"}</Label>
                    </Button>
                }
                if let Some(chip) = update_chip(state) {
                    {chip}
                }
                {account_chip(auth_user.as_ref(), auth_avatar, auth_loading, tc, scale)}
                if !ON_MACOS {
                    {window_controls(is_maximized, tc, scale)}
                }
            </div>
        </div>
    }
}

fn chrome_icon_button(
    icon: &'static str,
    action: Action,
    tooltip: &'static str,
    tc: &ThemeColors,
    scale: f32,
) -> AnyElement {
    view! { scale,
        <div class="flex-row items-center justify-center"
             px={Sp::SM} py={Sp::XXS}
             rounded={Rad::SM}
             hover_bg={tc.ghost_element_hover}
             on_click={action}
             cursor={CursorHint::Pointer}
             tooltip={tooltip}>
            <icon svg={icon} size={Ico::SM} color={tc.text_muted} />
        </div>
    }
}

fn repo_chip(label: &str, tc: &ThemeColors, scale: f32) -> AnyElement {
    view! { scale,
        <div class="flex-row items-center"
             gap={Sp::XS} px={Sp::SM} py={Sp::XXS}
             rounded={Rad::SM}
             hover_bg={tc.ghost_element_hover}
             on_click={crate::actions::OverlayAction::OpenRepoPicker.into()}
             cursor={CursorHint::Pointer}
             tooltip={"Switch repository"}
             min_w={0.0}>
            <icon svg={lucide::FOLDER} size={Ico::XS} color={tc.text_muted} />
            <text class="text-sm font-medium truncate" color={tc.text_strong}>{label}</text>
            <icon svg={lucide::CHEVRON_DOWN} size={Ico::XS} color={tc.text_muted} />
        </div>
    }
}

fn update_chip(state: &AppState) -> Option<AnyElement> {
    match state.ui.update.get(&state.store) {
        UpdateState::Available(update) => Some(
            Button::new(crate::actions::UpdateAction::InstallUpdate.into())
                .icon(lucide::ARROW_DOWN)
                .label("Update")
                .tooltip(format!("Install Diffy {}", update.version))
                .style(ButtonStyle::Filled)
                .size(ButtonSize::Compact)
                .into_any(),
        ),
        UpdateState::Downloading(update) => Some(
            Button::new(Action::Noop)
                .icon(lucide::ARROW_DOWN)
                .label("Updating")
                .tooltip(format!("Downloading Diffy {}", update.version))
                .style(ButtonStyle::Subtle)
                .size(ButtonSize::Compact)
                .into_any(),
        ),
        UpdateState::ReadyToRestart(update) => Some(
            Button::new(crate::actions::UpdateAction::RestartToUpdate.into())
                .icon(lucide::REFRESH)
                .label("Restart")
                .tooltip(format!(
                    "Restart Diffy to update to {}",
                    update.update.version
                ))
                .style(ButtonStyle::Filled)
                .size(ButtonSize::Compact)
                .into_any(),
        ),
        UpdateState::Restarting(update) => Some(
            Button::new(Action::Noop)
                .icon(lucide::REFRESH)
                .label("Restarting")
                .tooltip(format!(
                    "Restarting to install Diffy {}",
                    update.update.version
                ))
                .style(ButtonStyle::Subtle)
                .size(ButtonSize::Compact)
                .into_any(),
        ),
        _ => None,
    }
}

fn account_chip(
    user: Option<&crate::core::forge::github::GitHubUser>,
    avatar_image: Option<AvatarImage>,
    loading: bool,
    tc: &ThemeColors,
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
                     gap={Sp::XS} px={Sp::SM} py={Sp::XXS}
                     rounded={Rad::SM}
                     hover_bg={tc.ghost_element_hover}
                     on_click={crate::actions::GitHubAction::OpenAccountMenu.into()}
                     cursor={CursorHint::Pointer}
                     tooltip={tooltip}>
                    {avatar(avatar_name).size(Ico::LG).image(avatar_image)}
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
                     gap={Sp::XS} px={Sp::SM} py={Sp::XXS}
                     rounded={Rad::SM}
                     hover_bg={tc.ghost_element_hover}
                     on_click={crate::actions::GitHubAction::StartGitHubDeviceFlow.into()}
                     cursor={CursorHint::Pointer}
                     tooltip={tooltip}>
                    <icon svg={lucide::KEY} size={Ico::XS} color={tc.text_muted} />
                    <text class="text-sm font-medium" color={tc.text_strong}>{label}</text>
                </div>
            }
        }
    }
}

fn window_controls(is_maximized: bool, tc: &ThemeColors, scale: f32) -> AnyElement {
    let max_icon = if is_maximized {
        lucide::RESTORE
    } else {
        lucide::MAXIMIZE
    };
    let max_tip = if is_maximized { "Restore" } else { "Maximize" };
    view! { scale,
        <div class="flex-row items-center" h_full>
            {control_button(lucide::MINUS, WindowAction::Minimize.into(), "Minimize", tc.ghost_element_hover, tc, scale)}
            {control_button(max_icon, WindowAction::ToggleMaximize.into(), max_tip, tc.ghost_element_hover, tc, scale)}
            {control_button(lucide::X, WindowAction::Close.into(), "Close", Color::rgba(220, 70, 70, 220), tc, scale)}
        </div>
    }
}

/// Renders 4 edge + 4 corner resize hit zones around the window. Returns
/// `None` on macOS — there the OS owns resize. Caller stacks the result on
/// top of the root so reverse-iteration hit-testing finds an edge before the
/// chrome's drag region or any UI underneath.
pub(crate) fn resize_edges(width: f32, height: f32) -> Option<AnyElement> {
    if ON_MACOS {
        return None;
    }
    const EDGE: f32 = 6.0;
    const CORNER: f32 = 10.0;
    let mid_w = (width - 2.0 * CORNER).max(0.0);
    let mid_h = (height - 2.0 * CORNER).max(0.0);
    let _scale = 1.0_f32;
    let n = WindowAction::BeginResize(ResizeEdge::North).into();
    let s = WindowAction::BeginResize(ResizeEdge::South).into();
    let e = WindowAction::BeginResize(ResizeEdge::East).into();
    let w = WindowAction::BeginResize(ResizeEdge::West).into();
    let ne = WindowAction::BeginResize(ResizeEdge::NorthEast).into();
    let nw = WindowAction::BeginResize(ResizeEdge::NorthWest).into();
    let se = WindowAction::BeginResize(ResizeEdge::SouthEast).into();
    let sw = WindowAction::BeginResize(ResizeEdge::SouthWest).into();
    Some(view! { _scale,
        <div class="absolute" left={0.0} top={0.0} w={width} h={height}>
            // edges
            <div class="absolute" left={CORNER} top={0.0} w={mid_w} h={EDGE}
                 on_click={n} cursor={CursorHint::Default} />
            <div class="absolute" left={CORNER} top={height - EDGE} w={mid_w} h={EDGE}
                 on_click={s} cursor={CursorHint::Default} />
            <div class="absolute" left={0.0} top={CORNER} w={EDGE} h={mid_h}
                 on_click={w} cursor={CursorHint::Default} />
            <div class="absolute" left={width - EDGE} top={CORNER} w={EDGE} h={mid_h}
                 on_click={e} cursor={CursorHint::Default} />
            // corners (rendered last → win the hit test)
            <div class="absolute" left={0.0} top={0.0} w={CORNER} h={CORNER}
                 on_click={nw} cursor={CursorHint::Default} />
            <div class="absolute" left={width - CORNER} top={0.0} w={CORNER} h={CORNER}
                 on_click={ne} cursor={CursorHint::Default} />
            <div class="absolute" left={0.0} top={height - CORNER} w={CORNER} h={CORNER}
                 on_click={sw} cursor={CursorHint::Default} />
            <div class="absolute" left={width - CORNER} top={height - CORNER} w={CORNER} h={CORNER}
                 on_click={se} cursor={CursorHint::Default} />
        </div>
    })
}

fn control_button(
    icon: &'static str,
    action: Action,
    tooltip: &'static str,
    hover: Color,
    tc: &ThemeColors,
    _scale: f32,
) -> AnyElement {
    view! { _scale,
        <div class="flex-row items-center justify-center"
             w={Sz::CHROME_CONTROL_W} h_full
             hover_bg={hover}
             on_click={action}
             cursor={CursorHint::Pointer}
             tooltip={tooltip}>
            <icon svg={icon} size={Ico::XS} color={tc.text_muted} />
        </div>
    }
}
