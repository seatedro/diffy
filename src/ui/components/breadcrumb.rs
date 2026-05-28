use halogen::view;

use crate::actions::Action;
use crate::ui::design::Sp;
use crate::ui::element::{
    AnyElement, ElementContext, IntoAnyElement, RenderOnce, div, svg_icon, text,
};
use crate::ui::icons::lucide;
use crate::ui::style::Styled;

pub struct Breadcrumb {
    segments: Vec<String>,
    on_click_segment: Option<fn(usize) -> Action>,
}

pub fn breadcrumb(segments: impl IntoIterator<Item = impl Into<String>>) -> Breadcrumb {
    Breadcrumb {
        segments: segments.into_iter().map(|s| s.into()).collect(),
        on_click_segment: None,
    }
}

impl Breadcrumb {
    pub fn on_click_segment(mut self, f: fn(usize) -> Action) -> Self {
        self.on_click_segment = Some(f);
        self
    }
}

impl RenderOnce for Breadcrumb {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let m = &cx.theme.metrics;
        let scale = m.ui_scale();
        let icon_size = (m.ui_small_font_size - Sp::XXS * scale).max(Sp::SM * scale);
        let last = self.segments.len().saturating_sub(1);

        view! { scale,
            <div class="flex-row items-center" gap={m.spacing_xs}>
                for (i, segment) in self.segments.into_iter().enumerate() {
                    <fragment>
                        if i > 0 {
                            <icon svg={lucide::CHEVRON_RIGHT} size={icon_size} color={tc.text_muted} />
                        }
                        <div px={m.spacing_xs}
                             py={Sp::XXS}
                             rounded={m.control_radius - Sp::XS * scale}
                             @when {i != last && self.on_click_segment.is_some()} { on_click={(self.on_click_segment.unwrap())(i)} }
                             @when {i != last && self.on_click_segment.is_some()} {
                                 accessibility_role={accesskit::Role::Button}
                                 accessibility_id={format!("breadcrumb:{i}:{segment}")}
                                 accessibility_label={segment.clone()}
                             }
                             @when {i != last} { hover_bg={tc.ghost_element_hover} }>
                            <text class="text-sm"
                                  color={if i == last { tc.text_strong } else { tc.text_muted }}
                                  @when {i == last} { medium }>
                                {segment}
                            </text>
                        </div>
                    </fragment>
                }
            </div>
        }
    }
}
