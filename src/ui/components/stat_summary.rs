use crate::ui::element::{div, text, AnyElement, ElementContext, IntoAnyElement, RenderOnce};
use crate::ui::style::Styled;

use super::progress::diff_stat_bar;

pub struct StatSummary {
    file_count: usize,
    additions: u32,
    deletions: u32,
    compact: bool,
}

pub fn stat_summary(file_count: usize, additions: u32, deletions: u32) -> StatSummary {
    StatSummary {
        file_count,
        additions,
        deletions,
        compact: false,
    }
}

impl StatSummary {
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
}

impl RenderOnce for StatSummary {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let m = &cx.theme.metrics;

        if self.compact {
            return div()
                .flex_row()
                .items_center()
                .gap(m.spacing_sm)
                .child(
                    text(format!("+{}", self.additions))
                        .text_xs()
                        .color(tc.line_add_text),
                )
                .child(
                    text(format!("-{}", self.deletions))
                        .text_xs()
                        .color(tc.line_del_text),
                )
                .into_any();
        }

        let files_label = if self.file_count == 1 {
            "1 file changed".to_string()
        } else {
            format!("{} files changed", self.file_count)
        };

        div()
            .flex_row()
            .items_center()
            .gap(m.spacing_md)
            .child(text(files_label).text_sm().color(tc.text_muted))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(m.spacing_sm)
                    .child(
                        text(format!("+{}", self.additions))
                            .text_sm()
                            .color(tc.line_add_text)
                            .medium(),
                    )
                    .child(
                        text(format!("-{}", self.deletions))
                            .text_sm()
                            .color(tc.line_del_text)
                            .medium(),
                    ),
            )
            .child(diff_stat_bar(self.additions, self.deletions))
            .into_any()
    }
}
