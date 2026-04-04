use crate::ui::actions::Action;
use crate::ui::design::{Ico, Sp};
use crate::ui::element::*;
use crate::ui::shell::CursorHint;
use crate::ui::style::Styled;
use crate::ui::theme::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Filled,
    Subtle,
    Ghost,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    Default,
    Compact,
}

pub struct Button {
    icon: Option<&'static str>,
    label: Option<String>,
    action: Action,
    style: ButtonStyle,
    size: ButtonSize,
    active: bool,
    disabled: bool,
}

impl Button {
    pub fn new(action: Action) -> Self {
        Self {
            icon: None,
            label: None,
            action,
            style: ButtonStyle::Ghost,
            size: ButtonSize::Default,
            active: false,
            disabled: false,
        }
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for Button {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let theme = cx.theme;
        let tc = &theme.colors;
        let scale = (theme.metrics.ui_font_size / 16.0).max(0.7);

        let (icon_size, px, py) = match self.size {
            ButtonSize::Default => (Ico::BUTTON_DEFAULT, (Sp::LG * scale).round(), (Sp::SM * scale).round()),
            ButtonSize::Compact => (Ico::BUTTON_COMPACT, (Sp::MD * scale).round(), (Sp::XS * scale).round()),
        };

        let (bg, hover_bg, icon_color, text_color) = match self.style {
            ButtonStyle::Filled => (
                tc.accent,
                tc.accent.with_alpha(230),
                tc.text_strong,
                tc.text_strong,
            ),
            ButtonStyle::Subtle => (
                tc.element_background,
                tc.element_hover,
                tc.text_muted,
                tc.text,
            ),
            ButtonStyle::Ghost => {
                if self.active {
                    (
                        tc.element_background,
                        tc.element_hover,
                        tc.text,
                        tc.text,
                    )
                } else {
                    (
                        Color::TRANSPARENT,
                        tc.ghost_element_hover,
                        tc.text_muted,
                        tc.text_muted,
                    )
                }
            }
            ButtonStyle::Danger => (
                tc.status_error.with_alpha(30),
                tc.status_error.with_alpha(50),
                tc.status_error,
                tc.status_error,
            ),
        };

        let mut btn = div()
            .flex_row()
            .flex_shrink_0()
            .items_center()
            .gap((Sp::XS * scale).round() + Sp::XXS)
            .px(px)
            .py(py)
            .rounded_md()
            .bg(bg)
            .hover_bg(hover_bg)
            .on_click(self.action)
            .cursor(CursorHint::Pointer);

        if let Some(icon) = self.icon {
            btn = btn.child(svg_icon(icon, icon_size).color(icon_color));
        }

        if let Some(label) = self.label {
            let mut txt = text(label).medium().color(text_color);
            match self.size {
                ButtonSize::Default => txt = txt.text_sm(),
                ButtonSize::Compact => txt = txt.text_xs(),
            }
            btn = btn.child(txt);
        }

        btn.into_any()
    }
}
