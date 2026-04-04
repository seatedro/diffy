use crate::ui::actions::Action;
use crate::ui::design::{Ico, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{AppState, FileListEntry};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

pub struct FileListItem<'a> {
    pub entry: &'a FileListEntry,
    pub selected: bool,
    pub index: usize,
}

impl<'a> FileListItem<'a> {
    fn build(&self, theme: &Theme) -> Div {
        let tc = &theme.colors;
        let scale = (theme.metrics.ui_font_size / 16.0).max(0.7);
        let icon_color = if self.selected { tc.text_accent } else { tc.text_muted };
        let text_color = if self.selected { tc.text_strong } else { tc.text };

        let mut row = div()
            .w_full()
            .h((Sz::ROW * scale).round())
            .flex_row()
            .items_center()
            .px(Sp::SM)
            .gap_2()
            .on_click(Action::SelectFile(self.index))
            .cursor(CursorHint::Pointer);

        if self.selected {
            row = row.bg(tc.sidebar_row_selected).border_l(tc.accent);
        } else {
            row = row.hover_bg(tc.sidebar_row_hover);
        }

        row = row.child(svg_icon(lucide::FILE_CODE, Ico::MD).color(icon_color));
        row = row.child(
            div()
                .flex_1()
                .flex_col()
                .gap(1.0)
                .child(text(&self.entry.path).text_sm().color(text_color).truncate()),
        );

        if self.entry.additions > 0 || self.entry.deletions > 0 {
            row = row.child(
                div()
                    .flex_row()
                    .gap(Sp::XS)
                    .flex_shrink_0()
                    .child(
                        text(format!("+{}", self.entry.additions))
                            .text_xs()
                            .color(tc.line_add_text),
                    )
                    .child(
                        text(format!("\u{2212}{}", self.entry.deletions))
                            .text_xs()
                            .color(tc.line_del_text),
                    ),
            );
        }

        row
    }
}

pub struct Sidebar<'a> {
    pub state: &'a AppState,
    pub width_factor: f32,
}

impl<'a> Sidebar<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self {
            state,
            width_factor: 1.0,
        }
    }

    pub fn width_factor(mut self, factor: f32) -> Self {
        self.width_factor = factor;
        self
    }

    pub fn build(self, theme: &Theme) -> Div {
        if self.width_factor < 0.001 {
            return div().w(0.0).h_full();
        }

        let tc = &theme.colors;
        let scale = (theme.metrics.ui_font_size / 16.0).max(0.7);
        let sidebar_width = theme.metrics.sidebar_width * self.width_factor;
        let state = self.state;
        let file_count = state.workspace.files.len();

        let header = div()
            .px_4()
            .py_3()
            .flex_row()
            .items_center()
            .child(text("FILES").text_xs().semibold().color(tc.text_muted))
            .optional_child(if file_count > 0 {
                Some(div().px(Sp::SM).child(
                    div()
                        .px((Sp::LG / Sp::XXS * scale).round())
                        .py((Sp::XXS * scale).round())
                        .rounded_sm()
                        .bg(Color::rgba(255, 255, 255, 10))
                        .child(text(file_count.to_string()).text_xs().color(tc.text_muted)),
                ))
            } else {
                None
            });

        let mut sidebar = div()
            .flex_col()
            .w(sidebar_width)
            .flex_shrink_0()
            .overflow_hidden()
            .h_full()
            .bg(tc.sidebar_background)
            .border_r(tc.border_variant)
            .child(header);

        if state.workspace.files.is_empty() {
            let (icon, msg) = if state.compare.repo_path.is_some() {
                (lucide::GIT_COMPARE, "Run a compare to see changes.")
            } else {
                (lucide::FOLDER_OPEN, "Open a repository to start.")
            };
            sidebar = sidebar.child(
                div()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(svg_icon(icon, Ico::XL).color(tc.text_muted))
                            .child(text(msg).text_sm().color(tc.text_muted)),
                    ),
            );
        } else {
            let total_height = state.file_list.total_content_height(file_count);
            let scroll_px = state.file_list.scroll_offset_px;

            let mut list = div()
                .flex_1()
                .flex_col()
                .px((Sp::LG / Sp::XXS * scale).round())
                .gap(Sp::XS)
                .clip()
                .scroll_y(scroll_px)
                .scroll_total(total_height)
                .on_scroll(ScrollActionBuilder::FileList);

            for (index, entry) in state.workspace.files.iter().enumerate() {
                let selected = state.workspace.selected_file_index == Some(index);
                let item = FileListItem { entry, selected, index };
                list = list.child(item.build(theme));
            }

            sidebar = sidebar.child(list);
        }

        sidebar
    }
}
