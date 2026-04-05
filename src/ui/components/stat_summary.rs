use halogen::view;

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
            return view! {
                <div class="flex-row items-center" gap={m.spacing_sm}>
                    <text class="text-xs" color={tc.line_add_text}>{format!("+{}", self.additions)}</text>
                    <text class="text-xs" color={tc.line_del_text}>{format!("-{}", self.deletions)}</text>
                </div>
            };
        }

        let files_label = if self.file_count == 1 {
            "1 file changed".to_string()
        } else {
            format!("{} files changed", self.file_count)
        };

        view! {
            <div class="flex-row items-center" gap={m.spacing_md}>
                <text class="text-sm" color={tc.text_muted}>{files_label}</text>
                <div class="flex-row items-center" gap={m.spacing_sm}>
                    <text class="text-sm medium" color={tc.line_add_text}>{format!("+{}", self.additions)}</text>
                    <text class="text-sm medium" color={tc.line_del_text}>{format!("-{}", self.deletions)}</text>
                </div>
                {diff_stat_bar(self.additions, self.deletions)}
            </div>
        }
    }
}
