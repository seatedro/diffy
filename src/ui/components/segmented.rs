use crate::actions::Action;
use crate::ui::design::{Rad, Sp};
use crate::ui::element::*;
use crate::ui::shell::CursorHint;
use crate::ui::style::Styled;
use halogen::view;

pub struct SegmentedItem {
    pub label: String,
    pub action: Action,
    pub selected: bool,
    pub tooltip_text: Option<String>,
}

impl SegmentedItem {
    pub fn new(label: impl Into<String>, action: Action, selected: bool) -> Self {
        Self {
            label: label.into(),
            action,
            selected,
            tooltip_text: None,
        }
    }

    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip_text = Some(text.into());
        self
    }
}

pub struct SegmentedControl {
    items: Vec<SegmentedItem>,
}

impl SegmentedControl {
    pub fn new(items: Vec<SegmentedItem>) -> Self {
        Self { items }
    }
}

impl RenderOnce for SegmentedControl {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let scale = cx.theme.metrics.ui_scale();

        view! { scale,
            <div class="flex-row shrink-0 items-center overflow-hidden"
                 bg={tc.element_background}
                 rounded={Rad::XL}
                 p={Sp::XXS} gap={Sp::XXS}>
                for item in self.items {
                    <div class="flex-1 items-center justify-center"
                         px={Sp::MD} py={Sp::XXS}
                         rounded={Rad::LG}
                         bg={if item.selected { tc.ghost_element_hover }}
                         hover_bg={if !item.selected { tc.ghost_element_hover }}
                         hover_text_color={if !item.selected { tc.text }}
                         on_click={item.action}
                         cursor={CursorHint::Pointer}
                         @when { item.tooltip_text.is_some() } {
                             tooltip={item.tooltip_text.as_deref().unwrap_or_default()}
                         }>
                        <text class="text-sm font-medium"
                              color={if item.selected { tc.text } else { tc.text_muted }}>
                            {&item.label}
                        </text>
                    </div>
                }
            </div>
        }
    }
}
