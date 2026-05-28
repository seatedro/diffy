use halogen::view;

use crate::actions::Action;
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
        let chevron_icon = svg_icon(chevron, icon_size - Sp::XXS * scale).color(tc.text_muted);
        let trigger_py = m.spacing_xs + (Sp::XXS * scale).round();
        let trigger_label = self.label.clone();
        let trigger_id = format!("dropdown-trigger:{:?}:{trigger_label}", self.on_toggle);

        view! {
            <div class="flex-col">
                <div class="flex-row items-center"
                     gap={m.spacing_sm} px={m.spacing_md} py={trigger_py}
                     bg={tc.element_background} border={tc.border_variant}
                     rounded={m.control_radius} hover_bg={tc.element_hover}
                     accessibility_role={accesskit::Role::ComboBox}
                     accessibility_id={trigger_id}
                     accessibility_label={trigger_label}
                     accessibility_expanded={self.open}
                     @when {self.width.is_some()} { w={self.width.unwrap()} }
                     @when {self.on_toggle.is_some()} { on_click={self.on_toggle.unwrap()} }>
                    <div class="flex-1">
                        <text class="text-sm" color={tc.text}>{self.label}</text>
                    </div>
                    {chevron_icon}
                </div>
                if self.open {
                    <div class="flex-col w-full"
                         py={m.spacing_xs}
                         bg={tc.elevated_surface}
                         border={tc.border}
                         rounded={m.control_radius}
                         shadow_preset={Shadow::DROPDOWN}>
                        for item in self.items {
                            <div class="flex-row items-center"
                                 gap={m.spacing_sm} px={m.spacing_md}
                                 py={m.spacing_xs + (Sp::XXS * scale).round()}
                                 bg={if item.selected { tc.ghost_element_selected } else { Color::TRANSPARENT }}
                                 hover_bg={tc.ghost_element_hover}
                                 accessibility_role={accesskit::Role::MenuItem}
                                 accessibility_id={format!("dropdown-item:{:?}:{}", item.action, item.label)}
                                 accessibility_label={item.label.clone()}
                                 @when {item.description.is_some()} {
                                     accessibility_description={item.description.as_deref().unwrap_or_default()}
                                 }
                                 accessibility_selected={item.selected}
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
                }
            </div>
        }
    }
}
