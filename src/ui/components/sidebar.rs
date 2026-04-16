use halogen::view;

use crate::actions::Action;
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
    fn build(&self, theme: &Theme) -> AnyElement {
        let tc = &theme.colors;
        let icon_color = if self.selected {
            tc.text_accent
        } else {
            tc.text_muted
        };
        let text_color = if self.selected {
            tc.text_strong
        } else {
            tc.text
        };

        view! {
            <div class="w-full flex-row items-center" gap_2
                 h={theme.metrics.ui_row_height.round()}
                 px={Sp::SM}
                 on_click={Action::SelectFile(self.index)}
                 cursor={CursorHint::Pointer}
                 @when {self.selected} { bg={tc.sidebar_row_selected} border_l={tc.accent} }
                 @when {!self.selected} { hover_bg={tc.sidebar_row_hover} }>
                <icon svg={lucide::FILE_CODE} size={Ico::MD} color={icon_color} />
                <div class="flex-1 flex-col" gap={Sz::SEPARATOR_W}>
                    <text class="text-sm truncate" color={text_color}>{&self.entry.path}</text>
                </div>
                if self.entry.additions > 0 || self.entry.deletions > 0 {
                    <div class="flex-row shrink-0" gap={Sp::XS}>
                        <text class="text-xs" color={tc.line_add_text}>{format!("+{}", self.entry.additions)}</text>
                        <text class="text-xs" color={tc.line_del_text}>{format!("\u{2212}{}", self.entry.deletions)}</text>
                    </div>
                }
            </div>
        }
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

    pub fn build(self, theme: &Theme) -> AnyElement {
        if self.width_factor < 0.001 {
            return div().w(0.0).h_full().into_any();
        }

        let tc = &theme.colors;
        let scale = theme.metrics.ui_scale();
        let sidebar_width = theme.metrics.sidebar_width * self.width_factor;
        let state = self.state;
        let files_snapshot = state.workspace.files.get(&state.store);
        let file_count = files_snapshot.len();
        let has_repo = state
            .compare
            .repo_path
            .with(&state.store, |p| p.is_some());
        let selected_index = state.workspace.selected_file_index.get(&state.store);

        let (empty_icon, empty_msg) = if has_repo {
            (lucide::GIT_COMPARE, "Run a compare to see changes.")
        } else {
            (lucide::FOLDER_OPEN, "Open a repository to start.")
        };

        let total_height = state.file_list_total_content_height(file_count);
        let scroll_px = state.file_list.scroll_offset_px.get(&state.store);

        view! { scale,
            <div class="flex-col flex-shrink-0 overflow-hidden h-full"
                 w={sidebar_width}
                 bg={tc.sidebar_background}
                 border_r={tc.border_variant}>
                <div class="flex-row items-center" px_4 py_3>
                    <text class="text-xs semibold" color={tc.text_muted}>{"FILES"}</text>
                    if file_count > 0 {
                        <div px={Sp::SM}>
                            <div class="rounded-sm"
                                 px={Sp::LG / Sp::XXS}
                                 py={Sp::XXS}
                                 bg={Color::rgba(255, 255, 255, 10)}>
                                <text class="text-xs" color={tc.text_muted}>{file_count.to_string()}</text>
                            </div>
                        </div>
                    }
                </div>
                if files_snapshot.is_empty() {
                    <div class="flex-1 items-center justify-center">
                        <div class="flex-col items-center" gap_2>
                            <icon svg={empty_icon} size={Ico::XL} color={tc.text_muted} />
                            <text class="text-sm" color={tc.text_muted}>{empty_msg}</text>
                        </div>
                    </div>
                } else {
                    <div class="flex-1 flex-col" clip
                         px={Sp::LG / Sp::XXS}
                         gap={Sp::XS}
                         scroll_y={scroll_px}
                         scroll_total={total_height}
                         on_scroll={ScrollActionBuilder::FileList}>
                        for (index, entry) in files_snapshot.iter().enumerate() {
                            {FileListItem {
                                entry,
                                selected: selected_index == Some(index),
                                index,
                            }.build(theme)}
                        }
                    </div>
                }
            </div>
        }
    }
}
