use crate::actions::Action;
use crate::ui::animation::{AnimationKey, AnimationState};
use crate::ui::design::{Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::shell::CursorHint;
use crate::ui::style::Styled;
use halogen::view;

/// Per-toast visual properties computed by the stack.
struct ToastVisuals {
    message: String,
    index: usize,
    bottom: f32,
    left: f32,
    width: f32,
    z: i32,
}

impl RenderOnce for ToastVisuals {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let scale = cx.theme.metrics.ui_scale();

        view! { scale,
            <div class="absolute flex-row items-center"
                h={Sz::TOAST}
                w={self.width}
                bottom={self.bottom}
                left={self.left}
                bg={tc.elevated_surface}
                rounded={Rad::XL}
                border={tc.border}
                shadow_preset={Shadow::TOAST}
                on_click={Action::DismissToast(self.index)}
                cursor={CursorHint::Pointer}
                z_index={self.z}
                px={Sp::LG}
            >
                <div class="flex-1">
                    <text class="text-sm truncate" color={tc.text}>{&self.message}</text>
                </div>
            </div>
        }
    }
}

pub struct ToastStack<'a> {
    pub toasts: &'a [crate::ui::state::Toast],
    pub animation: &'a AnimationState,
    pub window_width: f32,
    pub window_height: f32,
    pub ui_scale: f32,
    pub status_bar_height: f32,
}

/// Sonner-style stacking constants.
const STACK_PEEK: f32 = 10.0;
const STACK_SHRINK: f32 = 12.0;
const MAX_VISIBLE_BEHIND: usize = 2;
const TOAST_Z_BASE: i32 = 300;
/// Vertical stride between toasts when fanned out (less than TOAST height = overlap).
const FAN_STRIDE: f32 = 40.0;

impl<'a> ToastStack<'a> {
    pub fn new(
        toasts: &'a [crate::ui::state::Toast],
        animation: &'a AnimationState,
        window_width: f32,
        window_height: f32,
        ui_scale: f32,
        status_bar_height: f32,
    ) -> Self {
        Self {
            toasts,
            animation,
            window_width,
            window_height,
            ui_scale,
            status_bar_height,
        }
    }

    pub fn build(self) -> Div {
        let scale = self.ui_scale;
        let toast_width =
            Sz::TOAST_MAX_W.min((self.window_width - Sz::TOAST_MARGIN).max(Sz::TOAST_MIN_W));
        let toast_height = (Sz::TOAST * scale).round();
        let status_bar_height = self.status_bar_height;
        let peek = (STACK_PEEK * scale).round();
        let fan_stride = (FAN_STRIDE * scale).round();

        // Animated fan progress: 0.0 = collapsed, 1.0 = fully fanned out.
        let fan_t = self
            .animation
            .progress(AnimationKey::ToastStackFan)
            .unwrap_or(0.0);

        let count = self.toasts.len();
        let visible = count.min(MAX_VISIBLE_BEHIND + 1);

        // Interpolate container height between collapsed and fanned.
        let collapsed_height = toast_height + (MAX_VISIBLE_BEHIND as f32) * peek;
        let fanned_height =
            toast_height + (visible.saturating_sub(1) as f32) * fan_stride;
        let stack_height = collapsed_height + fan_t * (fanned_height - collapsed_height);

        let mut container = div()
            .absolute()
            .bottom(status_bar_height + (Sp::LG * scale).round())
            .right((Sp::XL * scale).round())
            .w(toast_width)
            .h(stack_height)
            .z_index(TOAST_Z_BASE);

        // Render from deepest to shallowest so front toast paints last.
        for depth in (0..visible).rev() {
            let toast = &self.toasts[count - 1 - depth];

            // Interpolate between collapsed and fanned positions.
            let shrink = (depth as f32) * (STACK_SHRINK * scale).round();
            let collapsed_bottom = (depth as f32) * peek;
            let collapsed_width = toast_width - shrink;
            let collapsed_left = shrink / 2.0;

            let fanned_bottom = (depth as f32) * fan_stride;

            let bottom = collapsed_bottom + fan_t * (fanned_bottom - collapsed_bottom);
            let width = collapsed_width + fan_t * (toast_width - collapsed_width);
            let left = collapsed_left * (1.0 - fan_t);

            // Entrance slide for front toast only.
            let bottom = if depth == 0 {
                let anim_t = self
                    .animation
                    .progress(AnimationKey::ToastEntrance(toast.id))
                    .unwrap_or(1.0);
                bottom - (1.0 - anim_t) * toast_height
            } else {
                bottom
            };

            let z = TOAST_Z_BASE + (visible - depth) as i32;

            container = container.child(ToastVisuals {
                message: toast.message.clone(),
                index: count - 1 - depth,
                bottom,
                left,
                width,
                z,
            });
        }

        container
    }
}
