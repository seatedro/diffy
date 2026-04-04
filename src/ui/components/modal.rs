use crate::ui::actions::Action;
use crate::ui::design::{Ico, Sp, Sz};
use crate::ui::element::*;
use crate::ui::style::Styled;
use crate::ui::theme::Color;

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
        let scale = (cx.theme.metrics.ui_font_size / 16.0).max(0.7);

        let panel_width = self.max_width.min(self.window_width - (Sz::MODAL_MARGIN * scale).round());
        let padding = (self.padding * scale).round();
        let gap = (self.gap * scale).round();

        let mut header = div()
            .flex_col()
            .gap((Sp::SM * scale).round())
            .child(
                div()
                    .flex_row()
                    .flex_shrink_0()
                    .items_center()
                    .gap((Sp::SM * scale).round())
                    .child(svg_icon(self.icon, Ico::LG).color(tc.accent))
                    .child(text(&self.title).text_lg().semibold().color(tc.text_strong)),
            );

        if !self.subtitle.is_empty() {
            header = header.child(text(&self.subtitle).text_sm().color(tc.text_muted));
        }

        let mut panel = div()
            .w(panel_width)
            .flex_col()
            .overflow_hidden()
            .p(padding)
            .gap(gap)
            .bg(tc.elevated_surface)
            .rounded_xl()
            .border_b(tc.border)
            .shadow(24.0, 8.0, Color::rgba(0, 0, 0, 100))
            .shadow(8.0, 4.0, Color::rgba(0, 0, 0, 50))
            .shadow(2.0, 1.0, Color::rgba(0, 0, 0, 30))
            .on_click(Action::Noop)
            .child(header);

        if let Some(h) = self.height {
            panel = panel.h((h * scale).round());
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
