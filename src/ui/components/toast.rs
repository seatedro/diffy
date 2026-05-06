use crate::ui::animation::{AnimationKey, AnimationState};
use crate::ui::design::{Alpha, Ico, Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{Toast, ToastKind};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme, ThemeColors};
use halogen::view;

/// Sonner-style stacking constants (unscaled).
const STACK_PEEK: f32 = 10.0;
const MAX_VISIBLE_BEHIND: usize = 2;
const TOAST_Z_BASE: i32 = 300;
/// Vertical gap between toasts when fanned out.
const FAN_GAP: f32 = 10.0;
/// Mirrors state::TOAST_LIFETIME_MS.
const TOAST_LIFETIME_MS: u64 = 5_000;

/// Wide enough for actionable command/error detail without dominating the app.
pub const TOAST_WIDTH: f32 = 460.0;
pub const BADGE_SIZE: f32 = 26.0;
pub const CLOSE_SIZE: f32 = 22.0;
const PROGRESS_H: f32 = 2.0;
const CORNER_RADIUS: f32 = Rad::XL;
/// Max wrapped lines for title and description.
pub const TITLE_MAX_LINES: usize = 2;
pub const DESC_MAX_LINES: usize = 5;
/// Vertical padding inside the toast (top and bottom).
const PAD_Y: f32 = 12.0;
/// Gap between title and description lines.
const DESC_GAP: f32 = 2.0;

/// Horizontal chrome (left pad, badge, gap, gap, close, right pad) — the
/// remaining width is available for wrapped text.
pub const CHROME_W: f32 = Sp::MD + BADGE_SIZE + Sp::MD + Sp::MD + CLOSE_SIZE + Sp::MD;

/// Width used by the stack after applying the current window constraints.
pub fn toast_stack_width(window_width: f32, ui_scale: f32) -> f32 {
    let side_margin = (Sp::XL * ui_scale).round();
    let available = (window_width - side_margin * 2.0).max(1.0);
    available
        .min(TOAST_WIDTH)
        .max(available.min(Sz::TOAST_MIN_W))
}

/// Inner content width available for wrapped title / description.
pub fn toast_inner_text_width(toast_width: f32) -> f32 {
    (toast_width - CHROME_W).max(80.0)
}

/// Laid-out per-toast dimensions, computed in the shell where the font system
/// is available for wrapping. Parallel to `Toasts`.
#[derive(Debug, Clone)]
pub struct ToastLayout {
    pub title_lines: Vec<String>,
    pub description_lines: Vec<String>,
    pub height: f32,
}

/// Compute total height for a wrapped title + optional description.
pub fn compute_toast_height(theme: &Theme, title_lines: usize, desc_lines: usize) -> f32 {
    let title_lh = line_height(theme.metrics.ui_small_font_size);
    let desc_lh = line_height(theme.metrics.ui_small_font_size - 1.0);
    let title_h = title_lines.max(1) as f32 * title_lh;
    let desc_h = if desc_lines == 0 {
        0.0
    } else {
        DESC_GAP + desc_lines as f32 * desc_lh
    };
    let content_h = title_h + desc_h;
    let min_h = BADGE_SIZE + PAD_Y * 2.0;
    (content_h + PAD_Y * 2.0).max(min_h)
}

fn line_height(font_size: f32) -> f32 {
    (font_size * 1.35).ceil()
}

fn severity_color(kind: ToastKind, tc: &ThemeColors) -> Color {
    match kind {
        ToastKind::Info => tc.status_info,
        ToastKind::Error => tc.status_error,
    }
}

fn severity_icon(kind: ToastKind) -> &'static str {
    match kind {
        ToastKind::Info => lucide::INFO,
        ToastKind::Error => lucide::ALERT_CIRCLE,
    }
}

/// Per-toast visual props, built by the stack.
struct ToastVisuals {
    index: usize,
    kind: ToastKind,
    title_lines: Vec<String>,
    description_lines: Vec<String>,
    /// Fraction of lifetime consumed (0.0 = fresh, 1.0 = about to dismiss) —
    /// used when `external_progress` is `None`.
    time_progress: f32,
    /// When set, renders as an actual progress bar instead of a lifetime bar.
    /// The bar fills left-to-right as `external_progress` climbs from 0 to 1.
    external_progress: Option<f32>,
    bottom: f32,
    left: f32,
    width: f32,
    height: f32,
    z: i32,
}

impl RenderOnce for ToastVisuals {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let scale = cx.theme.metrics.ui_scale();
        let accent = severity_color(self.kind, tc);
        let icon_svg = severity_icon(self.kind);

        let progress_inset = CORNER_RADIUS;
        let progress_track_w = (self.width - progress_inset * 2.0).max(0.0);
        let fill_fraction = self
            .external_progress
            .map(|p| p.clamp(0.0, 1.0))
            .unwrap_or_else(|| self.time_progress.clamp(0.0, 1.0));
        let progress_fill_w = progress_track_w * fill_fraction;

        let badge_bg = accent.with_alpha(Alpha::TINT);
        let track_bg = tc.border.with_alpha(Alpha::SOFT);

        let title_children: Vec<AnyElement> = self
            .title_lines
            .into_iter()
            .map(|line| {
                text(line)
                    .text_sm()
                    .medium()
                    .truncate()
                    .color(tc.text_strong)
                    .into_any()
            })
            .collect();

        let desc_children: Vec<AnyElement> = self
            .description_lines
            .into_iter()
            .map(|line| {
                text(line)
                    .text_xs()
                    .truncate()
                    .color(tc.text_muted)
                    .into_any()
            })
            .collect();

        let has_description = !desc_children.is_empty();

        view! { scale,
            <div class="absolute"
                h={self.height}
                w={self.width}
                bottom={self.bottom}
                left={self.left}
                bg={tc.elevated_surface}
                rounded={CORNER_RADIUS}
                border={tc.border}
                shadow_preset={Shadow::TOAST}
                on_click={crate::actions::AppAction::DismissToast(self.index).into()}
                hit_identity={HitIdentity::Toast(self.index)}
                cursor={CursorHint::Pointer}
                z_index={self.z}
            >
                // Main row: leading badge | stacked title/description | close.
                <div class="flex-row items-center h-full w-full"
                    pl={Sp::MD}
                    pr={Sp::MD}
                    py={PAD_Y}
                    gap={Sp::MD}
                >
                    <div class="flex-row items-center justify-center shrink-0"
                        w={BADGE_SIZE} h={BADGE_SIZE}
                        rounded={BADGE_SIZE / 2.0}
                        bg={badge_bg}
                    >
                        <icon svg={icon_svg} size={Ico::SM} color={accent} />
                    </div>

                    <div class="flex-1 flex-col" min_w={0.0}>
                        {...title_children}
                        if has_description {
                            <div class="flex-col" pt={DESC_GAP} min_w={0.0}>
                                {...desc_children}
                            </div>
                        }
                    </div>

                    <div class="flex-row items-center justify-center shrink-0"
                        w={CLOSE_SIZE} h={CLOSE_SIZE}
                        rounded={Rad::MD}
                        hover_bg={tc.ghost_element_hover}
                        on_click={crate::actions::AppAction::DismissToast(self.index).into()}
                        hit_identity={HitIdentity::Toast(self.index)}
                        cursor={CursorHint::Pointer}
                    >
                        <icon svg={lucide::X} size={Ico::XS} color={tc.text_muted} />
                    </div>
                </div>

                // Time-remaining progress bar — fills left→right.
                <div class="absolute"
                    bottom={3.0} left={progress_inset}
                    h={PROGRESS_H}
                    w={progress_track_w}
                    rounded={PROGRESS_H / 2.0}
                    bg={track_bg}
                    overflow_hidden
                >
                    <div h_full w={progress_fill_w} bg={accent} />
                </div>
            </div>
        }
    }
}

pub struct ToastStack<'a> {
    pub toasts: &'a [Toast],
    pub animation: &'a AnimationState,
    pub window_width: f32,
    pub window_height: f32,
    pub ui_scale: f32,
    pub status_bar_height: f32,
    pub clock_ms: u64,
    /// Parallel to `toasts`: pre-wrapped lines + total height per toast.
    pub layouts: &'a [ToastLayout],
}

impl<'a> ToastStack<'a> {
    pub fn new(
        toasts: &'a [Toast],
        animation: &'a AnimationState,
        window_width: f32,
        window_height: f32,
        ui_scale: f32,
        status_bar_height: f32,
        clock_ms: u64,
        layouts: &'a [ToastLayout],
    ) -> Self {
        Self {
            toasts,
            animation,
            window_width,
            window_height,
            ui_scale,
            status_bar_height,
            clock_ms,
            layouts,
        }
    }

    pub fn build(self) -> Div {
        let scale = self.ui_scale;
        let peek = (STACK_PEEK * scale).round();
        let fan_gap = (FAN_GAP * scale).round();

        let container_w = toast_stack_width(self.window_width, scale);
        let side_margin = (Sp::XL * scale).round();
        let container_left = (self.window_width - container_w - side_margin).max(side_margin);

        let fan_t = self
            .animation
            .progress(AnimationKey::ToastStackFan)
            .unwrap_or(0.0);

        let count = self.toasts.len();
        let visible = count.min(MAX_VISIBLE_BEHIND + 1);

        // Resolve front toast height (it anchors both collapsed and fanned stacks).
        let front_h = if count > 0 {
            self.layouts
                .get(count - 1)
                .map(|l| l.height)
                .unwrap_or(Sz::TOAST)
        } else {
            Sz::TOAST
        };

        // Collapsed: front-toast height + MAX_VISIBLE_BEHIND peeks.
        let collapsed_height = front_h + (MAX_VISIBLE_BEHIND as f32) * peek;

        // Fanned: sum of visible toast heights + inter-toast gaps.
        let fanned_height: f32 = (0..visible)
            .map(|d| {
                self.layouts
                    .get(count - 1 - d)
                    .map(|l| l.height)
                    .unwrap_or(Sz::TOAST)
            })
            .sum::<f32>()
            + (visible.saturating_sub(1) as f32) * fan_gap;

        let stack_height = collapsed_height + fan_t * (fanned_height - collapsed_height);

        let mut container = div()
            .absolute()
            .bottom(self.status_bar_height + (Sp::LG * scale).round())
            .left(container_left)
            .w(container_w)
            .h(stack_height)
            .z_index(TOAST_Z_BASE);

        // Pre-compute cumulative fanned offsets from front (depth 0) upward.
        // fanned_bottom[d] = sum of heights of depths [0..d] + d gaps.
        let mut fanned_bottoms = Vec::with_capacity(visible);
        let mut running = 0.0_f32;
        for d in 0..visible {
            fanned_bottoms.push(running);
            let h = self
                .layouts
                .get(count - 1 - d)
                .map(|l| l.height)
                .unwrap_or(Sz::TOAST);
            running += h + fan_gap;
        }

        // Deepest first so the front toast paints last.
        for depth in (0..visible).rev() {
            let toast_idx = count - 1 - depth;
            let toast = &self.toasts[toast_idx];
            let layout = self.layouts.get(toast_idx).cloned().unwrap_or(ToastLayout {
                title_lines: vec![toast.message.clone()],
                description_lines: toast
                    .description
                    .clone()
                    .map(|d| vec![d])
                    .unwrap_or_default(),
                height: Sz::TOAST,
            });

            // Fixed width across all depths — matches Sonner. Back toasts
            // only differ by vertical offset (peek when collapsed, cumulative
            // height when fanned).
            let width = container_w;
            let collapsed_bottom = (depth as f32) * peek;
            let fanned_bottom = fanned_bottoms[depth];
            let bottom_raw = collapsed_bottom + fan_t * (fanned_bottom - collapsed_bottom);
            let left = 0.0;

            let bottom = if depth == 0 {
                let anim_t = self
                    .animation
                    .progress(AnimationKey::ToastEntrance(toast.id))
                    .unwrap_or(1.0);
                bottom_raw - (1.0 - anim_t) * layout.height
            } else {
                bottom_raw
            };

            let elapsed = self.clock_ms.saturating_sub(toast.created_at_ms);
            let time_progress = if toast.hovered {
                0.0
            } else {
                (elapsed as f32 / TOAST_LIFETIME_MS as f32).clamp(0.0, 1.0)
            };

            let z = TOAST_Z_BASE + (visible - depth) as i32;

            container = container.child(ToastVisuals {
                index: toast_idx,
                kind: toast.kind,
                title_lines: layout.title_lines,
                description_lines: layout.description_lines,
                time_progress,
                external_progress: toast.progress,
                bottom,
                left,
                width,
                height: layout.height,
                z,
            });
        }

        container
    }
}
