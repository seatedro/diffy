use crate::ui::design::{Sp, Sz};
use crate::ui::element::{div, text, AnyElement, ElementContext, IntoAnyElement, RenderOnce};
use crate::ui::style::Styled;
use crate::ui::theme::Color;

pub struct ProgressBar {
    value: f32,
    color: Option<Color>,
    track_color: Option<Color>,
    height: f32,
    show_label: bool,
}

pub fn progress_bar(value: f32) -> ProgressBar {
    ProgressBar {
        value: value.clamp(0.0, 1.0),
        color: None,
        track_color: None,
        height: Sz::PROGRESS_H,
        show_label: false,
    }
}

impl ProgressBar {
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }

    pub fn track_color(mut self, c: Color) -> Self {
        self.track_color = Some(c);
        self
    }

    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }

    pub fn show_label(mut self) -> Self {
        self.show_label = true;
        self
    }
}

impl RenderOnce for ProgressBar {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let fill_color = self.color.unwrap_or(tc.accent);
        let bg_color = self.track_color.unwrap_or(tc.element_background);
        let h = self.height;
        let v = self.value;

        let mut fill = div().h_full().bg(fill_color).rounded(h / 2.0);
        fill.element_style_mut().layout.flex_grow = if v > 0.001 { v } else { 0.001 };

        let mut empty = div().h_full();
        let remainder = (1.0 - v).max(0.001);
        empty.element_style_mut().layout.flex_grow = remainder;

        let track = div()
            .w_full()
            .h(h)
            .flex_row()
            .bg(bg_color)
            .rounded(h / 2.0)
            .overflow_hidden()
            .child(fill)
            .child(empty);

        if self.show_label {
            let pct = (v * 100.0).round() as u32;
            div()
                .flex_row()
                .items_center()
                .gap(Sp::SM)
                .w_full()
                .child(div().flex_1().child(track))
                .child(text(format!("{pct}%")).text_xs().color(tc.text_muted))
                .into_any()
        } else {
            track.into_any()
        }
    }
}

pub struct DiffStatBar {
    additions: u32,
    deletions: u32,
    width: f32,
}

pub fn diff_stat_bar(additions: u32, deletions: u32) -> DiffStatBar {
    DiffStatBar {
        additions,
        deletions,
        width: Sz::DIFFSTAT_W,
    }
}

impl DiffStatBar {
    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }
}

impl RenderOnce for DiffStatBar {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let total = (self.additions + self.deletions).max(1) as f32;
        let add_ratio = self.additions as f32 / total;
        let del_ratio = self.deletions as f32 / total;
        let h = Sz::DIFFSTAT_H;

        let mut add_bar = div().h_full().bg(tc.line_add_text);
        add_bar.element_style_mut().layout.flex_grow = add_ratio.max(0.001);

        let mut del_bar = div().h_full().bg(tc.line_del_text);
        del_bar.element_style_mut().layout.flex_grow = del_ratio.max(0.001);

        let mut track = div()
            .flex_row()
            .w(self.width)
            .h(h)
            .gap(Sz::SEPARATOR_W)
            .overflow_hidden()
            .rounded(h / 2.0)
            .bg(tc.element_background);

        if self.additions > 0 {
            track = track.child(add_bar);
        }
        if self.deletions > 0 {
            track = track.child(del_bar);
        }

        track.into_any()
    }
}
