use crate::ui::actions::Action;
use crate::ui::design::{Ico, Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::ToastKind;
use crate::ui::style::Styled;

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

        div()
            .w_full()
            .h((Sz::TOAST * scale).round())
            .flex_row()
            .items_center()
            .bg(tc.elevated_surface)
            .rounded_lg()
            .border(tc.border)
            .shadow_preset(Shadow::TOAST)
            .on_click(Action::DismissToast(self.index))
            .cursor(CursorHint::Pointer)
            .child(
                div()
                    .w((Sz::TOAST_STRIPE_W * scale).round())
                    .h_full()
                    .rounded((Rad::XXL * scale).round())
                    .bg(accent),
            )
            .child(
                div()
                    .px((Sp::MD * scale).round())
                    .child(svg_icon(icon, Ico::SM).color(accent)),
            )
            .child(
                div()
                    .flex_1()
                    .child(text(&self.message).text_sm().color(tc.text).truncate()),
            )
            .child(
                div()
                    .px((Sp::MD * scale).round())
                    .child(text("\u{00d7}").color(tc.text_muted)),
            )
            .into_any()
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
        let toast_width = Sz::TOAST_MAX_W.min((self.window_width - Sz::TOAST_MARGIN).max(Sz::TOAST_MIN_W));
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
