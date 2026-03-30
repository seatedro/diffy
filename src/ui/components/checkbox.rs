use crate::ui::actions::Action;
use crate::ui::element::{div, svg_icon, text, AnyElement, ElementContext, IntoAnyElement, RenderOnce};
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
        let size = (m.ui_font_size * 1.125).round();
        let icon_size = size - 4.0;
        let radius = (m.control_radius * 0.5).max(3.0);

        let (box_bg, box_border, check_color) = if self.disabled {
            (tc.element_background, tc.border_variant, tc.text_muted)
        } else if self.checked {
            (tc.accent, tc.accent, Color::rgba(255, 255, 255, 255))
        } else {
            (Color::TRANSPARENT, tc.border, tc.icon)
        };

        let check_box = div()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .w(size)
            .h(size)
            .bg(box_bg)
            .border(box_border)
            .rounded(radius)
            .when(!self.disabled && !self.checked, |d| {
                d.hover_bg(tc.ghost_element_hover)
            })
            .optional_child(
                self.checked
                    .then(|| svg_icon(lucide::CHECK, icon_size).color(check_color)),
            );

        let mut row = div().flex_row().items_center().gap(m.spacing_sm);

        if let Some(action) = self.on_toggle {
            if !self.disabled {
                row = row.on_click(action);
            }
        }

        row = row.child(check_box);

        if let Some(label_text) = self.label {
            let c = if self.disabled {
                tc.text_muted
            } else {
                tc.text
            };
            row = row.child(text(label_text).text_sm().color(c));
        }

        row.into_any()
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

        let track_w = (m.ui_font_size * 2.25).round();
        let track_h = (m.ui_font_size * 1.25).round();
        let thumb_size = track_h - 4.0;
        let thumb_left = if self.on {
            track_w - thumb_size - 2.0
        } else {
            2.0
        };

        let (track_bg, thumb_bg) = if self.disabled {
            (tc.element_background, tc.text_muted)
        } else if self.on {
            (tc.accent, Color::rgba(255, 255, 255, 255))
        } else {
            (tc.element_background, tc.icon)
        };

        let thumb = div()
            .absolute()
            .top(2.0)
            .left(thumb_left)
            .w(thumb_size)
            .h(thumb_size)
            .bg(thumb_bg)
            .rounded(thumb_size / 2.0)
            .shadow(2.0, 1.0, Color::rgba(0, 0, 0, 40));

        let hover_bg = if self.on {
            tc.accent.with_alpha(220)
        } else {
            tc.element_hover
        };

        let track = div()
            .flex_shrink_0()
            .w(track_w)
            .h(track_h)
            .bg(track_bg)
            .rounded(track_h / 2.0)
            .when(!self.disabled, |d| d.hover_bg(hover_bg))
            .child(thumb);

        let mut row = div().flex_row().items_center().gap(m.spacing_sm);

        if let Some(action) = self.on_toggle {
            if !self.disabled {
                row = row.on_click(action);
            }
        }

        row = row.child(track);

        if let Some(label_text) = self.label {
            let c = if self.disabled {
                tc.text_muted
            } else {
                tc.text
            };
            row = row.child(text(label_text).text_sm().color(c));
        }

        row.into_any()
    }
}
