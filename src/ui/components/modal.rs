use halogen::view;

use crate::ui::actions::Action;
use crate::ui::design::{Ico, Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::style::Styled;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModalAlign {
    Center,
    Top,
}

pub struct Modal {
    title: String,
    subtitle: String,
    icon: &'static str,
    max_width: f32,
    height: Option<f32>,
    gap: f32,
    padding: f32,
    align: ModalAlign,
    window_width: f32,
    window_height: f32,
    body: Vec<AnyElement>,
    footer: Vec<AnyElement>,
}

impl Modal {
    pub fn new(
        title: impl Into<String>,
        subtitle: impl Into<String>,
        icon: &'static str,
        max_width: f32,
        window_width: f32,
        window_height: f32,
    ) -> Self {
        Self {
            title: title.into(),
            subtitle: subtitle.into(),
            icon,
            max_width,
            height: None,
            gap: Sp::LG,
            padding: Sp::XXL,
            align: ModalAlign::Center,
            window_width,
            window_height,
            body: Vec::new(),
            footer: Vec::new(),
        }
    }

    pub fn height(mut self, h: f32) -> Self {
        self.height = Some(h);
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    pub fn align(mut self, align: ModalAlign) -> Self {
        self.align = align;
        self
    }

    pub fn body_child(mut self, child: impl IntoAnyElement) -> Self {
        self.body.push(child.into_any());
        self
    }

    pub fn footer_child(mut self, child: impl IntoAnyElement) -> Self {
        self.footer.push(child.into_any());
        self
    }
}

impl RenderOnce for Modal {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let scale = cx.theme.metrics.ui_scale();

        let panel_width = self.max_width.min(self.window_width - (Sz::MODAL_MARGIN * scale).round());
        let padding = (self.padding * scale).round();
        let gap = (self.gap * scale).round();

        let mut header = div()
            .flex_col()
            .gap((Sp::SM * scale).round())
            .child(view! { scale,
                <div class="flex-row shrink-0 items-center" gap={Sp::SM}>
                    <icon svg={self.icon} size={Ico::LG} color={tc.accent} />
                    <text class="text-lg font-semibold" color={tc.text_strong}>{&self.title}</text>
                </div>
            });

        if !self.subtitle.is_empty() {
            header = header.child(view! {
                <text class="text-sm" color={tc.text_muted}>{&self.subtitle}</text>
            });
        }

        let mut panel = div()
            .w(panel_width)
            .flex_col()
            .overflow_hidden()
            .p(padding)
            .gap(gap)
            .bg(tc.elevated_surface)
            .rounded((Rad::XXXL * scale).round())
            .border_b(tc.border)
            .shadow_preset(Shadow::MODAL)
            .on_click(Action::Noop)
            .child(header);

        if let Some(h) = self.height {
            let max_h = self.window_height - (Sz::MODAL_MARGIN * scale).round() * 2.0;
            panel = panel.h((h * scale).round().min(max_h));
        }

        for child in self.body {
            panel = panel.child(child);
        }

        if !self.footer.is_empty() {
            panel = panel.child(spacer());
            let mut footer_row = div().flex_row().gap((Sp::LG * scale).round());
            for child in self.footer {
                footer_row = footer_row.child(child);
            }
            panel = panel.child(footer_row);
        }

        let mut backdrop = div()
            .absolute()
            .top(0.0)
            .left(0.0)
            .w(self.window_width)
            .h(self.window_height)
            .z_index(100)
            .flex_col()
            .bg(tc.overlay_scrim)
            .on_click(Action::CloseOverlay)
            .items_center();

        match self.align {
            ModalAlign::Center => backdrop = backdrop.justify_center(),
            ModalAlign::Top => {
                backdrop = backdrop.pt((Sz::MODAL_TOP_OFFSET * scale).round());
            }
        }

        backdrop.child(panel).into_any()
    }
}
