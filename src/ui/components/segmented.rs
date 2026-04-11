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
}

impl SegmentedItem {
    pub fn new(label: impl Into<String>, action: Action, selected: bool) -> Self {
        Self {
            label: label.into(),
            action,
            selected,
        }
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
            <div class="flex-row shrink-0 items-center"
                 bg={tc.element_background}
                 rounded={Rad::XL}>
                for item in self.items {
                    <div class="shrink-0"
                         px={Sp::MD} py={Sp::XS}
                         rounded={Rad::XL}
                         bg={if item.selected { tc.ghost_element_hover }}
                         hover_bg={if !item.selected { tc.ghost_element_hover }}
                         on_click={item.action}
                         cursor={CursorHint::Pointer}>
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
