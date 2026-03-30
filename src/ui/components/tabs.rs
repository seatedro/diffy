use crate::ui::actions::Action;
use crate::ui::element::{div, text, AnyElement, ElementContext, IntoAnyElement, RenderOnce};
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

        let mut bar = div().flex_row().items_end().border_b(tc.border_variant);

        for item in self.items {
            let active = item.active;
            let label_color = if active {
                tc.text_strong
            } else {
                tc.text_muted
            };
            let indicator_color = if active {
                tc.accent
            } else {
                Color::TRANSPARENT
            };

            let mut content = div()
                .flex_row()
                .items_center()
                .gap(m.spacing_xs)
                .px(m.spacing_md)
                .py(m.spacing_sm);

            if let Some(svg) = item.icon {
                let icon_color = if active { tc.accent } else { tc.text_muted };
                content = content.child(
                    crate::ui::element::svg_icon(svg, icon_size).color(icon_color),
                );
            }

            content = content.child(text(item.label).text_sm().color(label_color).medium());

            if let Some(count_text) = item.count {
                content = content.child(
                    div()
                        .px(m.spacing_xs)
                        .py(1.0)
                        .bg(tc.element_background)
                        .rounded(8.0)
                        .child(text(count_text).text_xs().color(tc.text_muted)),
                );
            }

            let indicator = div().w_full().h(2.0).bg(indicator_color);

            let tab = div()
                .flex_col()
                .items_center()
                .on_click(item.action)
                .when(!active, |d| d.hover_bg(tc.ghost_element_hover))
                .when(self.fill, |d| d.flex_1())
                .child(content)
                .child(indicator);

            bar = bar.child(tab);
        }

        bar.into_any()
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

        let mut bar = div()
            .flex_row()
            .items_center()
            .gap(2.0)
            .p(2.0)
            .bg(tc.element_background)
            .rounded(m.control_radius);

        for item in self.items {
            let active = item.active;
            let fg = if active { tc.text_strong } else { tc.text_muted };

            let mut tab = div()
                .flex_row()
                .items_center()
                .justify_center()
                .flex_1()
                .px(m.spacing_md)
                .py(m.spacing_xs)
                .rounded(m.control_radius - 2.0)
                .on_click(item.action)
                .child(text(item.label).text_sm().color(fg).medium());

            if active {
                tab = tab
                    .bg(tc.surface)
                    .shadow(2.0, 1.0, Color::rgba(0, 0, 0, 20));
            } else {
                tab = tab.hover_bg(tc.ghost_element_hover);
            }

            bar = bar.child(tab);
        }

        bar.into_any()
    }
}
