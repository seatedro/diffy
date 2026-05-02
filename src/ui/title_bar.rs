use halogen::view;

use crate::actions::Action;
use crate::ui::design::{Ico, Rad, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{AppState, AsyncStatus, CompareField, OverlaySurface, WorkspaceMode};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme, ThemeColors};

pub(crate) fn compare_cluster_view(state: &AppState, theme: &Theme) -> Option<AnyElement> {
    let tc = &theme.colors;
    let has_repo = state.compare.repo_path.with(&state.store, |p| p.is_some());
    let repo_loaded = state.repository.status.get(&state.store) == AsyncStatus::Ready;
    let ref_picker_open = state.overlays_top() == Some(OverlaySurface::RefPicker);

    if has_repo && repo_loaded {
        let profile = state.repository.location.with(&state.store, |location| {
            crate::ui::vcs::profile(location.as_ref())
        });
        let left_ref_value = state.compare.left_ref.get(&state.store);
        let right_ref_value = state.compare.right_ref.get(&state.store);
        let left_label = if left_ref_value.is_empty() {
            "base".to_owned()
        } else {
            left_ref_value.clone()
        };
        let right_label = if right_ref_value.is_empty() {
            "head".to_owned()
        } else {
            profile.compare_ref_display_label(&right_ref_value)
        };

        let mode = profile.compare_mode_ui(state.compare.mode.get(&state.store));
        let (mode_label, mode_tooltip) = (mode.label, mode.tooltip);

        Some(compare_cluster(
            state,
            theme,
            &left_ref_value,
            &right_ref_value,
            &left_label,
            &right_label,
            mode_label,
            mode_tooltip,
            ref_picker_open,
        ))
    } else if state.workspace_mode.get(&state.store) == WorkspaceMode::Loading {
        let label = state.compare_progress.with(&state.store, |p| {
            p.as_ref()
                .map(|p| p.phase.label().to_owned())
                .unwrap_or_else(|| "Comparing\u{2026}".to_owned())
        });
        let _scale = theme.metrics.ui_scale();
        Some(view! { _scale,
            <text class="text-sm" color={tc.text_muted}>{label}</text>
        })
    } else {
        None
    }
}

fn swap_enabled_for_profile(
    profile: crate::ui::vcs::VcsUiProfile,
    left: &str,
    right: &str,
) -> bool {
    let left_trim = left.trim();
    let right_trim = right.trim();
    if left_trim.is_empty() || right_trim.is_empty() {
        return false;
    }
    profile.can_swap_ref(left_trim) && profile.can_swap_ref(right_trim)
}

#[allow(clippy::too_many_arguments)]
fn compare_cluster(
    state: &AppState,
    theme: &Theme,
    left_ref: &str,
    right_ref: &str,
    left_label: &str,
    right_label: &str,
    mode_label: &'static str,
    mode_tooltip: &'static str,
    picker_open: bool,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let profile = state.repository.location.with(&state.store, |location| {
        crate::ui::vcs::profile(location.as_ref())
    });
    let allow_swap = swap_enabled_for_profile(profile, left_ref, right_ref);
    let active_field = state.overlays.ref_picker.active_field.get(&state.store);
    let left_active = picker_open && active_field == CompareField::Left;
    let right_active = picker_open && active_field == CompareField::Right;

    let (left_action, right_action, swap_action) = if picker_open {
        (
            crate::actions::CompareAction::SetActiveRefField(CompareField::Left).into(),
            crate::actions::CompareAction::SetActiveRefField(CompareField::Right).into(),
            crate::actions::CompareAction::SwapDraftRefs.into(),
        )
    } else {
        (
            crate::actions::OverlayAction::OpenRefPicker(CompareField::Left).into(),
            crate::actions::OverlayAction::OpenRefPicker(CompareField::Right).into(),
            crate::actions::CompareAction::SwapRefs.into(),
        )
    };

    let has_pending = if picker_open {
        let original_left = state.overlays.ref_picker.original_left.get(&state.store);
        let original_right = state.overlays.ref_picker.original_right.get(&state.store);
        left_ref != original_left || right_ref != original_right
    } else {
        false
    };

    let cluster_h = ((Sz::SEARCH_INPUT + Sp::XS) * scale).round();
    view! { scale,
        <div class="flex-row shrink-0 items-center overflow-hidden"
             h={cluster_h}
             rounded={Rad::LG}
             border={tc.border}
             bg={tc.elevated_surface}>
            {chip_segment(
                left_label,
                left_ref.is_empty(),
                left_active,
                left_action,
                "Select base ref",
                tc,
                scale,
            )}
            {cluster_divider(tc, scale)}
            <div h_full class="flex-row items-center"
                 px={Sp::LG}
                 hover_bg={tc.ghost_element_hover}
                 on_click={crate::actions::CompareAction::OpenCompareMenu.into()}
                 cursor={CursorHint::Pointer}
                 tooltip={mode_tooltip}>
                <text class="text-xs font-medium" color={tc.text_muted}>{mode_label}</text>
            </div>
            {cluster_divider(tc, scale)}
            {chip_segment(
                right_label,
                right_ref.is_empty(),
                right_active,
                right_action,
                "Select head ref",
                tc,
                scale,
            )}
            if allow_swap {
                {cluster_divider(tc, scale)}
                <div h_full class="flex-row items-center"
                     px={Sp::MD}
                     hover_bg={tc.ghost_element_hover}
                     on_click={swap_action}
                     cursor={CursorHint::Pointer}
                     tooltip={"Swap refs"}>
                    <icon svg={lucide::ARROW_LEFT_RIGHT} size={Ico::SM} color={tc.text_muted} />
                </div>
            }
            if picker_open {
                {cluster_divider(tc, scale)}
                {compare_slot(has_pending, tc, scale)}
            }
        </div>
    }
}

fn cluster_divider(tc: &ThemeColors, _scale: f32) -> AnyElement {
    view! { _scale,
        <div h={Sz::SEPARATOR_H} w={Sz::SEPARATOR_W} bg={tc.border} />
    }
}

fn compare_slot(has_pending: bool, tc: &ThemeColors, scale: f32) -> AnyElement {
    let (bg, hover_bg, text_color, cursor, action, tip) = if has_pending {
        (
            tc.accent,
            tc.accent_strong,
            tc.text_strong,
            CursorHint::Pointer,
            crate::actions::CompareAction::CommitRefPicker.into(),
            "Apply changes and compare",
        )
    } else {
        (
            tc.element_background,
            tc.element_background,
            tc.text_muted,
            CursorHint::Default,
            Action::Noop,
            "No pending changes",
        )
    };
    view! { scale,
        <div h_full class="flex-row items-center"
             px={Sp::MD}
             bg={bg}
             hover_bg={hover_bg}
             on_click={action}
             cursor={cursor}
             tooltip={tip}>
            <text class="text-sm font-semibold" color={text_color}>{"Compare"}</text>
        </div>
    }
}

fn chip_segment(
    label: &str,
    is_placeholder: bool,
    is_active: bool,
    action: Action,
    tooltip_text: &'static str,
    tc: &ThemeColors,
    scale: f32,
) -> AnyElement {
    let text_color = if is_placeholder {
        tc.text_muted
    } else {
        tc.text_strong
    };
    let bg = if is_active {
        tc.ghost_element_active
    } else {
        Color::TRANSPARENT
    };
    view! { scale,
        <div h_full class="flex-row items-center"
             gap={Sp::XS} px={Sp::MD}
             bg={bg}
             hover_bg={tc.ghost_element_hover}
             on_click={action}
             cursor={CursorHint::Pointer}
             tooltip={tooltip_text}
             min_w={Sz::REF_SELECTOR_MIN_W}>
            <icon svg={lucide::GIT_BRANCH} size={Ico::SM} color={tc.text_muted} />
            <div class="flex-1" min_w={0.0}>
                <text class="text-sm font-medium truncate" color={text_color}>{label}</text>
            </div>
            <icon svg={lucide::CHEVRON_DOWN} size={Ico::XS} color={tc.text_muted} />
        </div>
    }
}
