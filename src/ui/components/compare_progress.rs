//! Compare-in-progress panel — shown in the main viewport while a compare
//! is running. Reports the active ref pair, current phase, elapsed time,
//! and a shimmer / determinate progress bar, with a cancel affordance.
//!
//! Kept deliberately lightweight: no per-frame allocation beyond the
//! elapsed-time string. The shimmer bar is GPU-side (halogen's
//! `BackgroundEffect::Shimmer`), so it doesn't cost CPU time to animate.

use halogen::view;

use crate::actions::Action;
use crate::ui::components::{Button, ButtonStyle};
use crate::ui::design::{Alpha, Ico, Rad, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::{AppState, ComparePhase, CompareProgress};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

pub fn compare_progress_panel(
    progress: &CompareProgress,
    state: &AppState,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let phase_label = progress.phase.label();
    let elapsed = elapsed_text(progress.started_at_ms, state.clock_ms);
    let count_text = file_count_line(progress);

    // Ref chips mirror the title-bar segmented compare control so the
    // progress panel reads as an extension of that surface.
    let chips = view! { scale,
        <div class="flex-row items-center" gap={Sp::MD}>
            {ref_chip(&progress.left_label, tc, scale)}
            <icon svg={lucide::ARROW_LEFT_RIGHT}
                  size={Ico::SM}
                  color={tc.text_muted} />
            {ref_chip(&progress.right_label, tc, scale)}
        </div>
    };

    // Progress bar: determinate rail once we know the file count and the
    // phase has moved past enumeration; shimmer otherwise. The determinate
    // path currently only toggles at `RenderingFirstFile` (one discrete
    // milestone), so its bar is visual reassurance more than granular.
    let bar = progress_rail(progress, tc, scale);

    view! { scale,
        <div class="flex-1 items-center justify-center" p={Sp::XL}>
            <div class="w-full flex-col" max_w={Sz::CARD_MD} gap={Sp::LG}
                 p={Sp::XL}
                 rounded={Rad::XL}
                 bg={tc.elevated_surface}
                 border_b={tc.border}
                 shadow_preset={crate::ui::design::Shadow::PANEL}>
                <div class="flex-col items-center" gap={Sp::MD}>
                    {chips}
                </div>
                <div class="flex-row items-center" gap={Sp::SM}>
                    <icon svg={lucide::LOADER} size={Ico::SM} color={tc.text_muted} />
                    <div class="flex-1" min_w={0.0}>
                        <text class="text-sm font-medium truncate" color={tc.text_strong}>
                            {phase_label.to_owned()}
                        </text>
                    </div>
                    <text class="text-xs font-mono" color={tc.text_muted}>{elapsed}</text>
                </div>
                {bar}
                if let Some(line) = count_text {
                    <text class="text-xs" color={tc.text_muted.with_alpha(Alpha::SOFT)}>
                        {line}
                    </text>
                }
                <div class="flex-row justify-center" pt={Sp::XS}>
                    {Button::new(Action::CancelCompare)
                        .label("Cancel")
                        .style(ButtonStyle::Subtle)}
                </div>
            </div>
        </div>
    }
}

fn ref_chip(label: &str, tc: &crate::ui::theme::ThemeColors, scale: f32) -> AnyElement {
    view! { scale,
        <div class="flex-row items-center"
             gap={Sp::XS}
             px={Sp::MD} py={Sp::XS}
             rounded={Rad::MD}
             bg={tc.element_background}
             border={tc.border_variant}>
            <icon svg={lucide::GIT_BRANCH} size={Ico::SM} color={tc.text_muted} />
            <text class="text-sm font-medium font-mono" color={tc.text_strong}>
                {label.to_owned()}
            </text>
        </div>
    }
}

fn progress_rail(
    progress: &CompareProgress,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
) -> AnyElement {
    let h = Sz::PROGRESS_H;
    let show_determinate = matches!(
        progress.phase,
        ComparePhase::PopulatingList | ComparePhase::RenderingFirstFile
    );

    if show_determinate && progress.file_count_total.is_some() {
        // Split the determinate phases 50/50 — PopulatingList fills half,
        // RenderingFirstFile fills the rest. Meaningful visual progress
        // without pretending we have finer granularity than we do.
        let fill = match progress.phase {
            ComparePhase::PopulatingList => 0.5,
            ComparePhase::RenderingFirstFile => 0.9,
            _ => 0.0,
        };
        crate::ui::components::progress_bar(fill)
            .height(h)
            .into_any()
    } else {
        // GPU shimmer — animated base-to-highlight gradient looping at
        // `speed` cycles/sec. Widths are clamped inside the shader; nothing
        // to do per-frame on the CPU.
        let base = tc.element_background;
        let highlight = tc.ghost_element_hover;
        view! { scale,
            <div w_full h={h}
                 rounded={h / 2.0}
                 overflow_hidden
                 bg={Color::TRANSPARENT}
                 bg_effect={crate::ui::element::shimmer(base, highlight, 1.2)} />
        }
    }
}

fn elapsed_text(started_at_ms: u64, now_ms: u64) -> String {
    let delta_ms = now_ms.saturating_sub(started_at_ms);
    let total_secs = delta_ms / 1000;
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    format!("{minutes}:{seconds:02}")
}

fn file_count_line(progress: &CompareProgress) -> Option<String> {
    progress.file_count_total.map(|total| {
        if total == 1 {
            "1 file changed".to_owned()
        } else {
            format!("{total} files changed")
        }
    })
}
