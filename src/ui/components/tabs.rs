use halogen::view;

use crate::actions::Action;
use crate::ui::design::{Rad, Shadow, Sp, Sz};
use crate::ui::element::{
    AnyElement, ElementContext, IntoAnyElement, RenderOnce, div, svg_icon, text,
};
use crate::ui::style::Styled;
use crate::ui::theme::Color;

pub struct TabItem {
    label: String,
    action: Action,
    active: bool,
    count: Option<String>,
    icon: Option<&'static str>,
}

impl TabItem {
    pub fn new(label: impl Into<String>, action: Action) -> Self {
        Self {
            label: label.into(),
            action,
            active: false,
            count: None,
            icon: None,
        }
    }

    pub fn active(mut self, a: bool) -> Self {
        self.active = a;
        self
    }

    pub fn count(mut self, c: impl Into<String>) -> Self {
        self.count = Some(c.into());
        self
    }

    pub fn icon(mut self, svg: &'static str) -> Self {
        self.icon = Some(svg);
        self
    }
}

pub struct TabBar {
    items: Vec<TabItem>,
    fill: bool,
}

pub fn tab_bar(items: Vec<TabItem>) -> TabBar {
    TabBar { items, fill: false }
}

impl TabBar {
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }
}

impl RenderOnce for TabBar {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let m = &cx.theme.metrics;
        let icon_size = m.ui_small_font_size;
        let fill = self.fill;

        view! {
            <div class="flex-row items-end" border_b={tc.border_variant}>
                for item in self.items {
                    <div class="flex-col items-center"
                         on_click={item.action}
                         @when {!item.active} { hover_bg={tc.ghost_element_hover} }
                         @when {fill} { flex_1 }>
                        <div class="flex-row items-center"
                             gap={m.spacing_xs} px={m.spacing_md} py={m.spacing_sm}>
                            if let Some(svg) = item.icon {
                                <icon svg={svg} size={icon_size}
                                      color={if item.active { tc.accent } else { tc.text_muted }} />
                            }
                            <text class="text-sm"
                                  color={if item.active { tc.text_strong } else { tc.text_muted }}
                                  @when {item.active} { medium }>
                                {item.label}
                            </text>
                            if let Some(count_text) = item.count {
                                <div px={m.spacing_xs} py={Sz::TAB_BADGE_PY}
                                     bg={tc.element_background} rounded={Rad::XL}>
                                    <text class="text-xs" color={tc.text_muted}>{count_text}</text>
                                </div>
                            }
                        </div>
                        <div w_full h={Sz::TAB_INDICATOR_H}
                             bg={if item.active { tc.accent } else { Color::TRANSPARENT }} />
                    </div>
                }
            </div>
        }
    }
}

pub struct SegmentedTabs {
    items: Vec<TabItem>,
}

pub fn segmented_tabs(items: Vec<TabItem>) -> SegmentedTabs {
    SegmentedTabs { items }
}

impl RenderOnce for SegmentedTabs {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let m = &cx.theme.metrics;
        let scale = m.ui_scale();
        let seg_gap = (Sp::XXS * scale).round();
        let inner_radius = m.control_radius - seg_gap;

        view! {
            <div class="flex-row items-center"
                 gap={seg_gap} p={seg_gap}
                 bg={tc.element_background} rounded={m.control_radius}>
                for item in self.items {
                    <div class="flex-row flex-1 items-center justify-center"
                         px={m.spacing_md} py={m.spacing_xs}
                         rounded={inner_radius}
                         on_click={item.action}
                         @when {item.active} { bg={tc.surface} shadow_preset={Shadow::SUBTLE} }
                         @when {!item.active} { hover_bg={tc.ghost_element_hover} }>
                        <text class="text-sm font-medium"
                              color={if item.active { tc.text_strong } else { tc.text_muted }}>
                            {item.label}
                        </text>
                    </div>
                }
            </div>
        }
    }
}
