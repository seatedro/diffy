use halogen::view;

use crate::actions::Action;
use crate::ui::design::{Alpha, Shadow, Sp, Sz};
use crate::ui::element::{
    AnyElement, ElementContext, IntoAnyElement, RenderOnce, div, svg_icon, text,
};
use crate::ui::icons::lucide;
use crate::ui::style::Styled;
use crate::ui::theme::Color;

pub struct Checkbox {
    checked: bool,
    label: Option<String>,
    on_toggle: Option<Action>,
    disabled: bool,
}

pub fn checkbox(checked: bool) -> Checkbox {
    Checkbox {
        checked,
        label: None,
        on_toggle: None,
        disabled: false,
    }
}

impl Checkbox {
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn on_toggle(mut self, action: Action) -> Self {
        self.on_toggle = Some(action);
        self
    }

    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let m = &cx.theme.metrics;
        let scale = m.ui_scale();
        let size = (m.ui_font_size * 1.125).round();
        let icon_size = size - Sp::XS * scale;
        let radius = (m.control_radius * 0.5).max(Sz::CHECKBOX_RAD_MIN * scale);

        let (box_bg, box_border, check_color) = if self.disabled {
            (tc.element_background, tc.border_variant, tc.text_muted)
        } else if self.checked {
            (tc.accent, tc.accent, Color::rgba(255, 255, 255, 255))
        } else {
            (Color::TRANSPARENT, tc.border, tc.icon)
        };

        let can_hover = !self.disabled && !self.checked;
        let check_box = view! {
            <div class="shrink-0 items-center justify-center"
                 w={size} h={size}
                 bg={box_bg} border={box_border} rounded={radius}
                 @when {can_hover} { hover_bg={tc.ghost_element_hover} }>
                if self.checked {
                    <icon svg={lucide::CHECK} size={icon_size} color={check_color} />
                }
            </div>
        };

        let click_action = self.on_toggle.filter(|_| !self.disabled);
        let label_color = if self.disabled { tc.text_muted } else { tc.text };

        view! {
            <div class="flex-row items-center" gap={m.spacing_sm}
                 @when {click_action.is_some()} { on_click={click_action.unwrap()} }>
                {check_box}
                if let Some(label_text) = self.label {
                    <text class="text-sm" color={label_color}>{label_text}</text>
                }
            </div>
        }
    }
}

pub struct Toggle {
    on: bool,
    label: Option<String>,
    on_toggle: Option<Action>,
    disabled: bool,
}

pub fn toggle(on: bool) -> Toggle {
    Toggle {
        on,
        label: None,
        on_toggle: None,
        disabled: false,
    }
}

impl Toggle {
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn on_toggle(mut self, action: Action) -> Self {
        self.on_toggle = Some(action);
        self
    }

    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }
}

impl RenderOnce for Toggle {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let m = &cx.theme.metrics;
        let scale = m.ui_scale();
        let thumb_inset = Sp::XXS * scale;
        let xs_scaled = Sp::XS * scale;

        let track_w = (m.ui_font_size * 2.25).round();
        let track_h = (m.ui_font_size * 1.25).round();
        let thumb_size = track_h - xs_scaled;
        let thumb_left = if self.on {
            track_w - thumb_size - thumb_inset
        } else {
            thumb_inset
        };

        let (track_bg, thumb_bg) = if self.disabled {
            (tc.element_background, tc.text_muted)
        } else if self.on {
            (tc.accent, Color::rgba(255, 255, 255, 255))
        } else {
            (tc.element_background, tc.icon)
        };

        let hover_bg = if self.on {
            tc.accent.with_alpha(Alpha::HOVER_ALT)
        } else {
            tc.element_hover
        };

        let enabled = !self.disabled;

        let thumb = view! {
            <div absolute top={thumb_inset} left={thumb_left}
                 w={thumb_size} h={thumb_size}
                 bg={thumb_bg} rounded={thumb_size / 2.0}
                 shadow_preset={Shadow::SUBTLE} />
        };

        let track = view! {
            <div flex_shrink_0
                 w={track_w} h={track_h}
                 bg={track_bg} rounded={track_h / 2.0}
                 @when {enabled} { hover_bg={hover_bg} }>
                {thumb}
            </div>
        };

        let click_action = self.on_toggle.filter(|_| !self.disabled);
        let label_color = if self.disabled { tc.text_muted } else { tc.text };

        view! {
            <div class="flex-row items-center" gap={m.spacing_sm}
                 @when {click_action.is_some()} { on_click={click_action.unwrap()} }>
                {track}
                if let Some(label_text) = self.label {
                    <text class="text-sm" color={label_color}>{label_text}</text>
                }
            </div>
        }
    }
}
