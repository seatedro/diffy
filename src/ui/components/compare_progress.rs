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
use crate::ui::state::{AppState, ComparePhase, CompareProgress, LoadingSubject};
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
    // Only show the standalone "N files changed" line once we've finished
    // counting — otherwise the phase label already contains "N of M".
    let count_text = match progress.phase {
        ComparePhase::LoadingFiles { .. } => None,
        _ => file_count_line(progress),
    };

    // Subject chips — ref pair for compare, folder chip for repo open.
    // Mirror the title-bar look so the panel reads as an extension of
    // whatever surface initiated the op.
    let chips = match &progress.subject {
        LoadingSubject::Compare {
            left_label,
            right_label,
        } => view! { scale,
            <div class="flex-row items-center" gap={Sp::MD}>
                {ref_chip(left_label, tc, scale)}
                <icon svg={lucide::ARROW_LEFT_RIGHT}
                      size={Ico::SM}
                      color={tc.text_muted} />
                {ref_chip(right_label, tc, scale)}
            </div>
        },
        LoadingSubject::RepoOpen { name } => view! { scale,
            <div class="flex-row items-center" gap={Sp::MD}>
                {repo_chip(name, tc, scale)}
            </div>
        },
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
                            {phase_label}
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

fn repo_chip(name: &str, tc: &crate::ui::theme::ThemeColors, scale: f32) -> AnyElement {
    view! { scale,
        <div class="flex-row items-center"
             gap={Sp::XS}
             px={Sp::MD} py={Sp::XS}
             rounded={Rad::MD}
             bg={tc.element_background}
             border={tc.border_variant}>
            <icon svg={lucide::FOLDER} size={Ico::SM} color={tc.text_muted} />
            <text class="text-sm font-medium" color={tc.text_strong}>
                {name.to_owned()}
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

    // Determinate fill only when we have something real to report. Repo
    // opens have three coarse phases with no denominator, so they always
    // shimmer — showing the bar at 92% the moment the panel reveals would
    // read as "almost done" even though work is still happening.
    let determinate_fill = match &progress.subject {
        LoadingSubject::RepoOpen { .. } => None,
        LoadingSubject::Compare { .. } => match progress.phase {
            ComparePhase::LoadingFiles { .. } => progress.file_count_total.and_then(|total| {
                (total > 0).then(|| (progress.files_loaded as f32 / total as f32).clamp(0.0, 1.0))
            }),
            // After backend work is done, freeze the bar near full. These
            // phases are quick; a solid cap reads better than a jump.
            ComparePhase::FetchingHistory => Some(0.92),
            ComparePhase::PopulatingList => Some(0.96),
            ComparePhase::RenderingFirstFile => Some(0.99),
            _ => None,
        },
    };

    if let Some(fill) = determinate_fill {
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
