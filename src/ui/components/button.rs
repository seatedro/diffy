use halogen::view;

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

        let (icon_size, unscaled_px, unscaled_py) = match self.size {
            ButtonSize::Default => (Ico::BUTTON_DEFAULT, Sp::MD, Sp::XS),
            ButtonSize::Compact => (Ico::BUTTON_COMPACT, Sp::SM, Sp::XXS),
        };

        let (bg, hover_bg, icon_color, text_color) = match self.style {
            ButtonStyle::Filled => (tc.accent, tc.accent_strong, tc.text_strong, tc.text_strong),
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

        let disabled = self.disabled;
        let (bg, icon_color, text_color) = if disabled {
            (
                bg,
                icon_color.with_alpha(Alpha::MUTED),
                text_color.with_alpha(Alpha::MUTED),
            )
        } else {
            (bg, icon_color, text_color)
        };

        let icon_only = self.icon.is_some() && self.label.is_none();
        let actual_px = if icon_only { unscaled_py } else { unscaled_px };
        let fixed = self.fixed_size.map(|s| (s * scale).round());
        let icon = self.icon;
        let action = if disabled { Action::Noop } else { self.action };
        let tooltip_text = self.tooltip_text;
        let cursor = if disabled {
            CursorHint::Default
        } else {
            CursorHint::Pointer
        };

        let label_el = self.label.map(|label| {
            let mut txt = text(label).medium().color(text_color);
            match self.size {
                ButtonSize::Default => txt = txt.text_sm(),
                ButtonSize::Compact => txt = txt.text_xs(),
            }
            txt
        });

        view! { scale,
            <div class="shrink-0" bg={bg}
                 cursor={cursor}
                 @when { !disabled } { on_click={action} }
                 @when { fixed.is_some() } {
                     items_center justify_center
                     w={fixed.unwrap()} h={fixed.unwrap()}
                     rounded={Rad::SM}
                 }
                 @when { fixed.is_none() } {
                     class="flex-row items-center"
                     gap={Sp::SM} px={actual_px} py={unscaled_py}
                     rounded={Rad::XL}
                 }
                 @when { !disabled && icon_only } { hover_icon_color={tc.text} }
                 @when { !disabled && !icon_only } { hover_bg={hover_bg} }
                 @when { tooltip_text.is_some() } {
                     tooltip={tooltip_text.as_deref().unwrap_or_default()}
                 }>
                if icon.is_some() {
                    <icon svg={icon.unwrap()} size={icon_size} color={icon_color} />
                }
                {?label_el}
            </div>
        }
    }
}
