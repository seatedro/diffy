use crate::actions::Action;
use crate::ui::design::{Alpha, Ico, Rad, Sp};
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
    tooltip_text: Option<String>,
    fixed_size: Option<f32>,
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
            tooltip_text: None,
            fixed_size: None,
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

    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip_text = Some(text.into());
        self
    }

    pub fn fixed_size(mut self, size: f32) -> Self {
        self.fixed_size = Some(size);
        self
    }
}

impl RenderOnce for Button {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let theme = cx.theme;
        let tc = &theme.colors;
        let scale = theme.metrics.ui_scale();

        let (icon_size, px, py) = match self.size {
            ButtonSize::Default => (
                Ico::BUTTON_DEFAULT,
                (Sp::MD * scale).round(),
                (Sp::XS * scale).round(),
            ),
            ButtonSize::Compact => (
                Ico::BUTTON_COMPACT,
                (Sp::SM * scale).round(),
                (Sp::XXS * scale).round(),
            ),
        };

        let (bg, hover_bg, icon_color, text_color) = match self.style {
            ButtonStyle::Filled => (
                tc.accent,
                tc.accent.with_alpha(Alpha::HOVER),
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
                        tc.ghost_element_active,
                        tc.ghost_element_hover,
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
                tc.status_error.with_alpha(Alpha::TINT),
                tc.status_error.with_alpha(Alpha::DIM),
                tc.status_error,
                tc.status_error,
            ),
        };

        let icon_only = self.icon.is_some() && self.label.is_none();

        let mut btn = div()
            .flex_shrink_0()
            .bg(bg)
            .on_click(self.action)
            .cursor(CursorHint::Pointer);

        if let Some(size) = self.fixed_size {
            let s = (size * scale).round();
            btn = btn
                .items_center()
                .justify_center()
                .w(s)
                .h(s)
                .rounded((Rad::SM * scale).round());
        } else {
            let actual_px = if icon_only { py } else { px };
            btn = btn
                .flex_row()
                .items_center()
                .gap((Sp::SM * scale).round())
                .px(actual_px)
                .py(py)
                .rounded((Rad::XL * scale).round());
        }

        if icon_only {
            btn = btn.hover_icon_color(tc.text);
        } else {
            btn = btn.hover_bg(hover_bg);
        }

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

        if let Some(tip) = self.tooltip_text {
            btn = btn.tooltip(tip);
        }

        btn.into_any()
    }
}
