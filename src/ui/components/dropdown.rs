use halogen::view;

use crate::ui::actions::Action;
use crate::ui::design::{Shadow, Sp};
use crate::ui::element::{
    AnyElement, ElementContext, IntoAnyElement, RenderOnce, div, svg_icon, text,
};
use crate::ui::icons::lucide;
use crate::ui::style::Styled;
use crate::ui::theme::Color;

pub struct DropdownItem {
    pub label: String,
    pub action: Action,
    pub selected: bool,
    pub icon: Option<&'static str>,
    pub description: Option<String>,
}

impl DropdownItem {
    pub fn new(label: impl Into<String>, action: Action) -> Self {
        Self {
            label: label.into(),
            action,
            selected: false,
            icon: None,
            description: None,
        }
    }

    pub fn selected(mut self, s: bool) -> Self {
        self.selected = s;
        self
    }

    pub fn icon(mut self, svg: &'static str) -> Self {
        self.icon = Some(svg);
        self
    }

    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }
}

pub struct Dropdown {
    label: String,
    items: Vec<DropdownItem>,
    open: bool,
    on_toggle: Option<Action>,
    width: Option<f32>,
}

pub fn dropdown(label: impl Into<String>, items: Vec<DropdownItem>) -> Dropdown {
    Dropdown {
        label: label.into(),
        items,
        open: false,
        on_toggle: None,
        width: None,
    }
}

impl Dropdown {
    pub fn open(mut self, o: bool) -> Self {
        self.open = o;
        self
    }

    pub fn on_toggle(mut self, action: Action) -> Self {
        self.on_toggle = Some(action);
        self
    }

    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }
}

impl RenderOnce for Dropdown {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let m = &cx.theme.metrics;
        let scale = m.ui_scale();
        let icon_size = m.ui_small_font_size;
        let chevron = if self.open {
            lucide::CHEVRON_UP
        } else {
            lucide::CHEVRON_DOWN
        };

        let mut trigger = div()
            .flex_row()
            .items_center()
            .gap(m.spacing_sm)
            .px(m.spacing_md)
            .py(m.spacing_xs + (Sp::XXS * scale).round())
            .bg(tc.element_background)
            .border(tc.border_variant)
            .rounded(m.control_radius)
            .hover_bg(tc.element_hover)
            .child(view! {
                <div class="flex-1">
                    <text class="text-sm" color={tc.text}>{self.label}</text>
                </div>
            })
            .child(svg_icon(chevron, icon_size - Sp::XXS * scale).color(tc.text_muted));

        if let Some(w) = self.width {
            trigger = trigger.w(w);
        }
        if let Some(action) = self.on_toggle {
            trigger = trigger.on_click(action);
        }

        if !self.open {
            return trigger.into_any();
        }

        view! { scale,
            <div class="flex-col">
                {trigger}
                <div class="flex-col w-full"
                     py={m.spacing_xs}
                     bg={tc.elevated_surface}
                     border={tc.border}
                     rounded={m.control_radius}
                     shadow_preset={Shadow::DROPDOWN}>
                    for item in self.items {
                        <div class="flex-row items-center"
                             gap={m.spacing_sm} px={m.spacing_md}
                             py={m.spacing_xs + Sp::XXS}
                             bg={if item.selected { tc.ghost_element_selected } else { Color::TRANSPARENT }}
                             hover_bg={tc.ghost_element_hover}
                             on_click={item.action}>
                            if let Some(svg) = item.icon {
                                <icon svg={svg} size={icon_size} color={tc.icon} />
                            }
                            <div class="flex-col flex-1">
                                <text class="text-sm"
                                      color={if item.selected { tc.text_strong } else { tc.text }}>
                                    {item.label}
                                </text>
                                if let Some(desc) = item.description {
                                    <text class="text-xs" color={tc.text_muted}>{desc}</text>
                                }
                            </div>
                            if item.selected {
                                <icon svg={lucide::CHECK} size={icon_size} color={tc.accent} />
                            }
                        </div>
                    }
                </div>
            </div>
        }
    }
}
