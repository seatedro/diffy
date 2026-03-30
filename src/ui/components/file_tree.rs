use std::collections::{BTreeMap, HashSet};

use crate::ui::actions::Action;
use crate::ui::element::{div, svg_icon, text, AnyElement, ElementContext, IntoAnyElement, RenderOnce};
use crate::ui::icons::lucide;
use crate::ui::style::Styled;
use crate::ui::theme::Color;

pub struct FileTreeEntry {
    pub path: String,
    pub status: String,
    pub additions: i32,
    pub deletions: i32,
}

pub struct FileTree {
    entries: Vec<FileTreeEntry>,
    expanded_folders: HashSet<String>,
    selected: Option<usize>,
    on_select_file: fn(usize) -> Action,
    on_toggle_folder: Option<fn(String) -> Action>,
}

pub fn file_tree(entries: Vec<FileTreeEntry>) -> FileTree {
    FileTree {
        entries,
        expanded_folders: HashSet::new(),
        selected: None,
        on_select_file: Action::SelectFile,
        on_toggle_folder: None,
    }
}

impl FileTree {
    pub fn expanded(mut self, folders: HashSet<String>) -> Self {
        self.expanded_folders = folders;
        self
    }

    pub fn selected(mut self, idx: Option<usize>) -> Self {
        self.selected = idx;
        self
    }

    pub fn on_select_file(mut self, f: fn(usize) -> Action) -> Self {
        self.on_select_file = f;
        self
    }

    pub fn on_toggle_folder(mut self, f: fn(String) -> Action) -> Self {
        self.on_toggle_folder = Some(f);
        self
    }
}

struct TreeNode {
    children_dirs: BTreeMap<String, TreeNode>,
    files: Vec<(usize, String, String, i32, i32)>,
}

impl TreeNode {
    fn new() -> Self {
        Self {
            children_dirs: BTreeMap::new(),
            files: Vec::new(),
        }
    }

    fn insert(&mut self, parts: &[&str], original_index: usize, file_name: &str, status: &str, adds: i32, dels: i32) {
        if parts.is_empty() {
            self.files.push((original_index, file_name.to_string(), status.to_string(), adds, dels));
        } else {
            let dir = parts[0];
            let child = self.children_dirs.entry(dir.to_string()).or_insert_with(TreeNode::new);
            child.insert(&parts[1..], original_index, file_name, status, adds, dels);
        }
    }
}

enum FlatRow {
    Folder {
        name: String,
        path: String,
        depth: usize,
        expanded: bool,
    },
    File {
        name: String,
        original_index: usize,
        depth: usize,
        status: String,
        additions: i32,
        deletions: i32,
        selected: bool,
    },
}

fn flatten_tree(
    node: &TreeNode,
    prefix: &str,
    depth: usize,
    expanded: &HashSet<String>,
    selected: Option<usize>,
    out: &mut Vec<FlatRow>,
) {
    for (dir_name, child) in &node.children_dirs {
        let full_path = if prefix.is_empty() {
            dir_name.clone()
        } else {
            format!("{prefix}/{dir_name}")
        };
        let is_expanded = expanded.contains(&full_path);

        out.push(FlatRow::Folder {
            name: dir_name.clone(),
            path: full_path.clone(),
            depth,
            expanded: is_expanded,
        });

        if is_expanded {
            flatten_tree(child, &full_path, depth + 1, expanded, selected, out);
        }
    }

    for &(orig_idx, ref name, ref status, adds, dels) in &node.files {
        out.push(FlatRow::File {
            name: name.clone(),
            original_index: orig_idx,
            depth,
            status: status.clone(),
            additions: adds,
            deletions: dels,
            selected: selected == Some(orig_idx),
        });
    }
}

impl RenderOnce for FileTree {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let m = &cx.theme.metrics;
        let row_height = (m.ui_font_size * 2.0).round();
        let indent_unit = m.spacing_lg;
        let icon_size = m.ui_small_font_size;

        let Self {
            entries,
            expanded_folders,
            selected,
            on_select_file,
            on_toggle_folder,
        } = self;

        let mut root = TreeNode::new();
        for (i, entry) in entries.iter().enumerate() {
            let parts: Vec<&str> = entry.path.split('/').collect();
            if parts.len() > 1 {
                let dir_parts = &parts[..parts.len() - 1];
                let file_name = parts[parts.len() - 1];
                root.insert(dir_parts, i, file_name, &entry.status, entry.additions, entry.deletions);
            } else {
                root.files.push((i, entry.path.clone(), entry.status.clone(), entry.additions, entry.deletions));
            }
        }

        let mut flat_rows = Vec::new();
        flatten_tree(&root, "", 0, &expanded_folders, selected, &mut flat_rows);

        let mut container = div().flex_col().w_full();

        for row in flat_rows {
            match row {
                FlatRow::Folder {
                    name,
                    path,
                    depth,
                    expanded,
                } => {
                    let chevron = if expanded {
                        lucide::CHEVRON_DOWN
                    } else {
                        lucide::CHEVRON_RIGHT
                    };
                    let folder_icon = if expanded {
                        lucide::FOLDER_OPEN
                    } else {
                        lucide::FOLDER
                    };

                    let mut row_div = div()
                        .flex_row()
                        .items_center()
                        .w_full()
                        .h(row_height)
                        .gap(m.spacing_xs)
                        .px(m.spacing_sm)
                        .hover_bg(tc.sidebar_row_hover);

                    if depth > 0 {
                        row_div = row_div.child(
                            div()
                                .w(indent_unit * depth as f32)
                                .flex_shrink_0(),
                        );
                    }

                    if let Some(f) = on_toggle_folder {
                        row_div = row_div.on_click(f(path));
                    }

                    row_div = row_div
                        .child(svg_icon(chevron, icon_size - 2.0).color(tc.text_muted))
                        .child(svg_icon(folder_icon, icon_size).color(tc.text_muted))
                        .child(text(name).text_sm().color(tc.text).medium());

                    container = container.child(row_div);
                }
                FlatRow::File {
                    name,
                    original_index,
                    depth,
                    status,
                    additions,
                    deletions,
                    selected: is_selected,
                } => {
                    let fg = if is_selected {
                        tc.text_strong
                    } else {
                        tc.text
                    };
                    let icon_color = if is_selected {
                        tc.accent
                    } else {
                        tc.text_muted
                    };
                    let bg = if is_selected {
                        tc.sidebar_row_selected
                    } else {
                        Color::TRANSPARENT
                    };

                    let mut row_div = div()
                        .flex_row()
                        .items_center()
                        .w_full()
                        .h(row_height)
                        .gap(m.spacing_xs)
                        .px(m.spacing_sm)
                        .bg(bg)
                        .on_click(on_select_file(original_index));

                    if !is_selected {
                        row_div = row_div.hover_bg(tc.sidebar_row_hover);
                    }

                    let indent_w = indent_unit * depth as f32 + icon_size;
                    if indent_w > 0.1 {
                        row_div = row_div.child(
                            div().w(indent_w).flex_shrink_0(),
                        );
                    }

                    row_div = row_div
                        .child(svg_icon(lucide::FILE_CODE, icon_size).color(icon_color))
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .child(text(name).text_sm().color(fg).truncate()),
                        );

                    if additions > 0 || deletions > 0 {
                        let mut stats = div().flex_row().flex_shrink_0().gap(m.spacing_xs);
                        if additions > 0 {
                            stats = stats.child(
                                text(format!("+{additions}"))
                                    .text_xs()
                                    .color(tc.line_add_text),
                            );
                        }
                        if deletions > 0 {
                            stats = stats.child(
                                text(format!("-{deletions}"))
                                    .text_xs()
                                    .color(tc.line_del_text),
                            );
                        }
                        row_div = row_div.child(stats);
                    }

                    if !status.is_empty() {
                        row_div = row_div.child(super::badge::status_badge(status));
                    }

                    container = container.child(row_div);
                }
            }
        }

        container.into_any()
    }
}
