use crate::ui::element::{div, svg_icon, text, AnyElement, ElementContext, IntoAnyElement, RenderOnce};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, ThemeColors};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeVariant {
    Default,
    Info,
    Success,
    Warning,
    Error,
    Accent,
}

pub struct Badge {
    label: String,
    variant: BadgeVariant,
    icon: Option<&'static str>,
}

pub fn badge(label: impl Into<String>) -> Badge {
    Badge {
        label: label.into(),
        variant: BadgeVariant::Default,
        icon: None,
    }
}

impl Badge {
    pub fn variant(mut self, v: BadgeVariant) -> Self {
        self.variant = v;
        self
    }

    pub fn icon(mut self, svg: &'static str) -> Self {
        self.icon = Some(svg);
        self
    }

    pub fn success(self) -> Self {
        self.variant(BadgeVariant::Success)
    }

    pub fn error(self) -> Self {
        self.variant(BadgeVariant::Error)
    }

    pub fn warning(self) -> Self {
        self.variant(BadgeVariant::Warning)
    }

    pub fn info(self) -> Self {
        self.variant(BadgeVariant::Info)
    }

    pub fn accent(self) -> Self {
        self.variant(BadgeVariant::Accent)
    }
}

fn variant_colors(variant: BadgeVariant, tc: &ThemeColors) -> (Color, Color) {
    match variant {
        BadgeVariant::Default => (tc.element_background, tc.text_muted),
        BadgeVariant::Info => (tc.status_info.with_alpha(25), tc.status_info),
        BadgeVariant::Success => (tc.line_add, tc.line_add_text),
        BadgeVariant::Warning => (tc.line_modified, tc.status_warning),
        BadgeVariant::Error => (tc.line_del, tc.line_del_text),
        BadgeVariant::Accent => (tc.accent.with_alpha(25), tc.accent),
    }
}

impl RenderOnce for Badge {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let m = &cx.theme.metrics;
        let (bg, fg) = variant_colors(self.variant, tc);
        let icon_size = (m.ui_small_font_size - 2.0).max(10.0);

        let mut row = div()
            .flex_row()
            .flex_shrink_0()
            .items_center()
            .gap(m.spacing_xs)
            .px(m.spacing_sm)
            .py(2.0)
            .bg(bg)
            .rounded(10.0);

        if let Some(svg) = self.icon {
            row = row.child(svg_icon(svg, icon_size).color(fg));
        }

        row.child(text(self.label).text_xs().color(fg).medium())
            .into_any()
    }
}

pub struct StatusBadge {
    status: String,
}

pub fn status_badge(status: impl Into<String>) -> StatusBadge {
    StatusBadge {
        status: status.into(),
    }
}

fn status_appearance(status: &str, tc: &ThemeColors) -> (Color, Color, String) {
    match status.to_lowercase().as_str() {
        "a" | "added" => (tc.line_add, tc.line_add_text, "A".into()),
        "d" | "deleted" => (tc.line_del, tc.line_del_text, "D".into()),
        "m" | "modified" | "changed" => (tc.line_modified, tc.status_warning, "M".into()),
        "r" | "renamed" => (tc.status_info.with_alpha(30), tc.status_info, "R".into()),
        "c" | "copied" => (tc.status_info.with_alpha(30), tc.status_info, "C".into()),
        "u" | "untracked" => (tc.element_background, tc.text_muted, "U".into()),
        other => {
            let label = other
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "?".into());
            (tc.element_background, tc.text_muted, label)
        }
    }
}

impl RenderOnce for StatusBadge {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let m = &cx.theme.metrics;
        let (bg, fg, label) = status_appearance(&self.status, tc);
        let size = (m.ui_small_font_size + 4.0).round();

        div()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .w(size)
            .h(size)
            .bg(bg)
            .rounded(4.0)
            .child(text(label).text_xs().color(fg).bold().text_center())
            .into_any()
    }
}
