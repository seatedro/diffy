use crate::ui::actions::Action;
use crate::ui::design::{Ico, Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::ToastKind;
use crate::ui::style::Styled;
use halogen::view;

pub struct Toast {
    message: String,
    kind: ToastKind,
    index: usize,
}

impl Toast {
    pub fn new(message: impl Into<String>, kind: ToastKind, index: usize) -> Self {
        Self {
            message: message.into(),
            kind,
            index,
        }
    }
}

impl RenderOnce for Toast {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let scale = cx.theme.metrics.ui_scale();

        let accent = match self.kind {
            ToastKind::Info => tc.status_info,
            ToastKind::Error => tc.status_error,
        };

        let icon = match self.kind {
            ToastKind::Info => lucide::INFO,
            ToastKind::Error => lucide::ALERT_CIRCLE,
        };

        view! { scale,
            <div class="w-full flex-row items-center"
                h={Sz::TOAST}
                bg={tc.elevated_surface}
                rounded_lg
                border={tc.border}
                shadow_preset={Shadow::TOAST}
                on_click={Action::DismissToast(self.index)}
                cursor={CursorHint::Pointer}
            >
                <div class="h-full" w={Sz::TOAST_STRIPE_W} rounded={Rad::XXL} bg={accent} />
                <div px={Sp::MD}>
                    <icon svg={icon} size={Ico::SM} color={accent} />
                </div>
                <div class="flex-1">
                    <text class="text-sm truncate" color={tc.text}>{&self.message}</text>
                </div>
                <div px={Sp::MD}>
                    <text color={tc.text_muted}>{"\u{00d7}"}</text>
                </div>
            </div>
        }
    }
}

pub struct ToastStack<'a> {
    pub toasts: &'a [crate::ui::state::Toast],
    pub window_width: f32,
    pub window_height: f32,
}

impl<'a> ToastStack<'a> {
    pub fn new(
        toasts: &'a [crate::ui::state::Toast],
        window_width: f32,
        window_height: f32,
    ) -> Self {
        Self {
            toasts,
            window_width,
            window_height,
        }
    }

    pub fn build(self) -> Div {
        let toast_width =
            Sz::TOAST_MAX_W.min((self.window_width - Sz::TOAST_MARGIN).max(Sz::TOAST_MIN_W));
        let status_bar_height = Sz::ROW;

        let mut stack = div()
            .absolute()
            .bottom(status_bar_height + Sp::LG)
            .right(Sp::XL)
            .w(toast_width)
            .flex_col()
            .gap(Sp::SM)
            .z_index(200);

        for (index, toast) in self.toasts.iter().enumerate().rev() {
            stack = stack.child(Toast::new(&toast.message, toast.kind, index));
        }

        stack
    }
}
