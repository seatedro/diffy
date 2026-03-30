use crate::ui::actions::Action;
use crate::ui::element::{div, svg_icon, text, AnyElement, ElementContext, IntoAnyElement, RenderOnce};
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
        let icon_size = (m.ui_small_font_size - 2.0).max(8.0);
        let last = self.segments.len().saturating_sub(1);

        let mut row = div().flex_row().items_center().gap(m.spacing_xs);

        for (i, segment) in self.segments.into_iter().enumerate() {
            if i > 0 {
                row = row.child(svg_icon(lucide::CHEVRON_RIGHT, icon_size).color(tc.text_muted));
            }

            let is_last = i == last;
            let color = if is_last {
                tc.text_strong
            } else {
                tc.text_muted
            };

            let mut label = text(segment).text_sm().color(color);
            if is_last {
                label = label.medium();
            }

            let mut seg = div()
                .px(m.spacing_xs)
                .py(2.0)
                .rounded(m.control_radius - 4.0)
                .child(label);

            if !is_last {
                if let Some(f) = self.on_click_segment {
                    seg = seg.on_click(f(i));
                }
                seg = seg.hover_bg(tc.ghost_element_hover);
            }

            row = row.child(seg);
        }

        row.into_any()
    }
}
