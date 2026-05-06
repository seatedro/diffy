use halogen::view;

use crate::actions::Action;
use crate::core::compare::CompareMode;
use crate::core::vcs::model::RefKind;
use crate::ui::components;
use crate::ui::design::{Ico, Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::AppState;
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

pub fn compare_menu(state: &AppState, theme: &Theme, width: f32, height: f32) -> AnyElement {
    let tc = &theme.colors;
    let m = &theme.metrics;
    let scale = m.ui_scale();
    let menu_w = (Sz::CONTEXT_MENU_MIN_W * 1.4 * scale).round();
    let menu_x = ((width - menu_w) / 2.0).round();
    let menu_y = m.title_bar_height + (Sp::XS * scale).round();
    let profile = state.repository.location.with(&state.store, |location| {
        crate::ui::vcs::profile(location.as_ref())
    });
    let modes = profile.compare_modes();

    let (head_branch, trunk) = state.repository.refs.with(&state.store, |refs| {
        let head = refs
            .iter()
            .find(|reference| reference.active && reference.kind == RefKind::Branch)
            .map(|reference| reference.name.clone());
        let trunk = refs
            .iter()
            .find(|reference| {
                reference.kind == RefKind::Branch
                    && matches!(reference.name.as_str(), "main" | "master" | "develop")
            })
            .map(|reference| reference.name.clone());
        (head, trunk)
    });

    let show_branch_preset = profile.shows_branch_preset()
        && matches!((&head_branch, &trunk), (Some(h), Some(t)) if h != t);
    let head_commit = state
        .repository
        .changes
        .with(&state.store, |changes| changes.first().cloned());
    let current_change = state.repository.changes.with(&state.store, |changes| {
        changes
            .iter()
            .find(|change| change.flags.working_copy || change.flags.current)
            .cloned()
    });
    let compare_mode = state.compare.mode.get(&state.store);
    let mode_rows = modes
        .iter()
        .enumerate()
        .map(|(index, mode)| {
            mode_row(
                mode.mode,
                mode.label,
                mode.description,
                compare_mode,
                Some(index + 1),
                theme,
            )
        })
        .collect::<Vec<_>>();
    let branch_shortcut = show_branch_preset.then_some(modes.len() + 1);
    let mut next_shortcut = modes.len() + 1 + if show_branch_preset { 1 } else { 0 };
    let current_change_preset = current_change.as_ref().and_then(|change| {
        profile.current_change_preset_label(change).map(|label| {
            let shortcut = next_shortcut;
            next_shortcut += 1;
            preset_row(
                &label,
                &crate::ui::vcs::change_summary_label(change),
                crate::actions::CompareAction::ApplyComparePreset("@::commit".to_owned()).into(),
                Some(shortcut),
                theme,
            )
        })
    });
    let head_commit_preset = if profile.shows_head_commit_preset() {
        head_commit.as_ref().map(|commit| {
            let shortcut = next_shortcut;
            preset_row(
                &format!("HEAD ({})", commit.short_revision),
                &crate::ui::vcs::change_summary_label(commit),
                crate::actions::CompareAction::ApplyComparePreset(format!(
                    "{}::commit",
                    commit.revision.id
                ))
                .into(),
                Some(shortcut),
                theme,
            )
        })
    } else {
        None
    };
    let show_presets =
        show_branch_preset || current_change_preset.is_some() || head_commit_preset.is_some();

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
                {...mode_rows}
                if show_presets {
                    <div class="w-full" py={Sp::XS} px={Sp::SM}>
                        <div class="w-full" h={Sz::SEPARATOR_W} bg={tc.border_variant} />
                    </div>
                }
                if show_branch_preset {
                    {preset_row(
                        &format!("{} vs {}", head_branch.as_deref().unwrap(), trunk.as_deref().unwrap()),
                        "Changes since fork",
                        crate::actions::CompareAction::ApplyComparePreset(
                            format!("{}:{}:merge", trunk.as_deref().unwrap(), head_branch.as_deref().unwrap())
                        ).into(),
                        branch_shortcut,
                        theme,
                    )}
                }
                {?current_change_preset}
                {?head_commit_preset}
            </div>
        </div>
    }
}

fn mode_row(
    mode: CompareMode,
    label: &str,
    desc: &str,
    active: CompareMode,
    shortcut: Option<usize>,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let selected = mode == active;
    let check_size = (Ico::XS * scale).round();
    let shortcut_badge = shortcut.map(|shortcut| components::kbd(shortcut.to_string(), theme));

    view! { scale,
        <div class="flex-row items-center"
             px={Sp::MD} py={Sp::XS + Sp::XXS}
             rounded={Rad::MD}
             hover_bg={tc.sidebar_row_hover}
             on_click={crate::actions::CompareAction::SetCompareMode(mode).into()}
             cursor={CursorHint::Pointer}>
            <div class="flex-col flex-1 overflow-hidden" min_w={0.0}>
                <text class="text-sm truncate" color={if selected { tc.text_strong } else { tc.text }}>{label}</text>
                <text class="text-xs truncate" color={tc.text_muted}>{desc}</text>
            </div>
            if selected {
                <div class="shrink-0" pl={Sp::SM}>
                    <icon svg={lucide::CHECK} size={check_size} color={tc.accent} />
                </div>
            }
            if let Some(shortcut_badge) = shortcut_badge {
                <div class="shrink-0" pl={Sp::SM}>
                    {shortcut_badge}
                </div>
            }
        </div>
    }
}

fn preset_row(
    label: &str,
    desc: &str,
    action: Action,
    shortcut: Option<usize>,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let shortcut_badge = shortcut.map(|shortcut| components::kbd(shortcut.to_string(), theme));

    view! { scale,
        <div class="w-full flex-row items-center"
             px={Sp::MD} py={Sp::XS + Sp::XXS}
             rounded={Rad::MD}
             hover_bg={tc.sidebar_row_hover}
             on_click={action}
             cursor={CursorHint::Pointer}>
            <div class="flex-col flex-1 overflow-hidden" min_w={0.0}>
                <text class="text-sm truncate" color={tc.text}>{label}</text>
                <text class="text-xs truncate" color={tc.text_muted}>{desc}</text>
            </div>
            if let Some(shortcut_badge) = shortcut_badge {
                <div class="shrink-0" pl={Sp::SM}>
                    {shortcut_badge}
                </div>
            }
        </div>
    }
}
