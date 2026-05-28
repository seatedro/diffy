use std::collections::{BTreeMap, HashSet};
use std::ops::Range;

use halogen::view;

use crate::actions::Action;
use crate::ui::design::Ico;
use crate::ui::element::{
    AnyElement, ElementContext, IntoAnyElement, RenderOnce, div, svg_icon, text,
};
use crate::ui::icons::lucide;
use crate::ui::style::Styled;
use crate::ui::theme::Color;

pub struct FileTreeEntry {
    pub path: String,
    pub status: String,
    pub scope: Option<String>,
    pub additions: i32,
    pub deletions: i32,
}

pub struct FileTree {
    rows: Vec<FlatRow>,
    total_rows: usize,
    window_start: usize,
    row_gap: f32,
    on_select_file: fn(usize) -> Action,
    on_toggle_folder: Option<fn(String) -> Action>,
}

pub struct FileTreeLayout {
    rows: Vec<FlatRow>,
}

pub fn file_tree(entries: Vec<FileTreeEntry>) -> FileTree {
    file_tree_layout(entries, &HashSet::new(), None).render_window(0..usize::MAX)
}

pub fn file_tree_layout(
    entries: Vec<FileTreeEntry>,
    expanded_folders: &HashSet<String>,
    selected: Option<usize>,
) -> FileTreeLayout {
    let mut root = TreeNode::new();
    for (i, entry) in entries.iter().enumerate() {
        insert_entry(&mut root, i, entry);
    }

    let mut rows = Vec::new();
    flatten_tree(&root, "", 0, expanded_folders, selected, &mut rows);
    FileTreeLayout { rows }
}

pub fn file_tree_visible_row_count_by(
    visit_paths: impl FnOnce(&mut dyn FnMut(&str)),
    expanded_folders: &HashSet<String>,
) -> usize {
    let mut root = TreeNode::new();
    let mut insert = |path: &str| {
        insert_path_for_count(&mut root, path);
    };
    visit_paths(&mut insert);
    count_visible_rows(&root, "", expanded_folders)
}

pub fn file_tree_visible_file_indices_by(
    visit_paths: impl FnOnce(&mut dyn FnMut(usize, &str)),
    expanded_folders: &HashSet<String>,
    window: Range<usize>,
) -> Vec<usize> {
    let mut root = TreeNode::new();
    let mut insert = |index: usize, path: &str| {
        insert_path_with_index(&mut root, index, path);
    };
    visit_paths(&mut insert);

    let mut row = 0usize;
    let mut indices = Vec::new();
    collect_visible_file_indices(&root, "", expanded_folders, &window, &mut row, &mut indices);
    indices
}

impl FileTreeLayout {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn render_window(self, window: Range<usize>) -> FileTree {
        let total_rows = self.rows.len();
        let start = window.start.min(total_rows);
        let end = window.end.min(total_rows).max(start);
        let rows = self
            .rows
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect();

        FileTree {
            rows,
            total_rows,
            window_start: start,
            row_gap: 0.0,
            on_select_file: select_file,
            on_toggle_folder: None,
        }
    }
}

fn select_file(index: usize) -> Action {
    crate::actions::FileListAction::SelectFile(index).into()
}

impl FileTree {
    pub fn row_gap(mut self, gap: f32) -> Self {
        self.row_gap = gap;
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
    files: Vec<(usize, String, String, Option<String>, i32, i32)>,
}

impl TreeNode {
    fn new() -> Self {
        Self {
            children_dirs: BTreeMap::new(),
            files: Vec::new(),
        }
    }

    fn insert(
        &mut self,
        parts: &[&str],
        original_index: usize,
        file_name: &str,
        status: &str,
        scope: Option<&str>,
        adds: i32,
        dels: i32,
    ) {
        if parts.is_empty() {
            self.files.push((
                original_index,
                file_name.to_string(),
                status.to_string(),
                scope.map(str::to_owned),
                adds,
                dels,
            ));
        } else {
            let dir = parts[0];
            let child = self
                .children_dirs
                .entry(dir.to_string())
                .or_insert_with(TreeNode::new);
            child.insert(
                &parts[1..],
                original_index,
                file_name,
                status,
                scope,
                adds,
                dels,
            );
        }
    }
}

fn insert_entry(root: &mut TreeNode, original_index: usize, entry: &FileTreeEntry) {
    let parts: Vec<&str> = entry.path.split('/').collect();
    if parts.len() > 1 {
        root.insert(
            &parts[..parts.len() - 1],
            original_index,
            parts[parts.len() - 1],
            &entry.status,
            entry.scope.as_deref(),
            entry.additions,
            entry.deletions,
        );
    } else {
        root.files.push((
            original_index,
            entry.path.clone(),
            entry.status.clone(),
            entry.scope.clone(),
            entry.additions,
            entry.deletions,
        ));
    }
}

fn insert_path_for_count(root: &mut TreeNode, path: &str) {
    insert_path_with_index(root, 0, path);
}

fn insert_path_with_index(root: &mut TreeNode, original_index: usize, path: &str) {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 1 {
        root.insert(
            &parts[..parts.len() - 1],
            original_index,
            parts[parts.len() - 1],
            "",
            None,
            0,
            0,
        );
    } else {
        root.files
            .push((original_index, path.to_owned(), String::new(), None, 0, 0));
    }
}

fn count_visible_rows(node: &TreeNode, prefix: &str, expanded: &HashSet<String>) -> usize {
    let mut count = node.files.len();
    for (dir_name, child) in &node.children_dirs {
        count += 1;
        let full_path = if prefix.is_empty() {
            dir_name.clone()
        } else {
            format!("{prefix}/{dir_name}")
        };
        if expanded.contains(&full_path) {
            count += count_visible_rows(child, &full_path, expanded);
        }
    }
    count
}

fn collect_visible_file_indices(
    node: &TreeNode,
    prefix: &str,
    expanded: &HashSet<String>,
    window: &Range<usize>,
    row: &mut usize,
    out: &mut Vec<usize>,
) {
    for (dir_name, child) in &node.children_dirs {
        let full_path = if prefix.is_empty() {
            dir_name.clone()
        } else {
            format!("{prefix}/{dir_name}")
        };
        *row = row.saturating_add(1);
        if expanded.contains(&full_path) {
            collect_visible_file_indices(child, &full_path, expanded, window, row, out);
        }
    }

    for &(original_index, _, _, _, _, _) in &node.files {
        if window.contains(&*row) {
            out.push(original_index);
        }
        *row = row.saturating_add(1);
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
        scope: Option<String>,
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

    for &(orig_idx, ref name, ref status, ref scope, adds, dels) in &node.files {
        out.push(FlatRow::File {
            name: name.clone(),
            original_index: orig_idx,
            depth,
            status: status.clone(),
            scope: scope.clone(),
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
        let row_height = m.ui_row_height.round();
        let indent_unit = m.spacing_lg;
        let scale = m.ui_scale();
        let icon_size = Ico::SM;
        let chevron_size = Ico::XS;

        let Self {
            rows,
            total_rows,
            window_start,
            row_gap,
            on_select_file,
            on_toggle_folder,
        } = self;

        let mut container = div().flex_col().w_full();

        for (offset, row) in rows.into_iter().enumerate() {
            let global_index = window_start + offset;
            let wrapper_height = if global_index + 1 == total_rows {
                row_height
            } else {
                row_height + row_gap
            };
            let row_element = match row {
                FlatRow::Folder {
                    name,
                    path,
                    depth,
                    expanded,
                } => {
                    let accessibility_label = name.clone();
                    let accessibility_id = format!("file-tree-folder:{path}");
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
                        .accessibility_role(accesskit::Role::TreeItem)
                        .accessibility_id(accessibility_id)
                        .accessibility_label(accessibility_label)
                        .accessibility_expanded(expanded)
                        .hover_bg(tc.sidebar_row_hover);

                    if depth > 0 {
                        row_div =
                            row_div.child(div().w(indent_unit * depth as f32).flex_shrink_0());
                    }

                    if let Some(f) = on_toggle_folder {
                        row_div = row_div.on_click(f(path));
                    }

                    row_div = row_div
                        .child(svg_icon(chevron, chevron_size).color(tc.text_muted))
                        .child(svg_icon(folder_icon, icon_size).color(tc.text_muted))
                        .child(text(name).text_sm().color(tc.text).medium());

                    row_div.into_any()
                }
                FlatRow::File {
                    name,
                    original_index,
                    depth,
                    status,
                    scope,
                    additions,
                    deletions,
                    selected: is_selected,
                } => {
                    let accessibility_id = format!("file-tree-file:{original_index}:{name}");
                    let accessibility_label = match (&scope, additions, deletions) {
                        (Some(scope), 0, 0) => format!("{name}, {scope}"),
                        (Some(scope), _, _) => {
                            format!("{name}, {scope}, +{additions}, -{deletions}")
                        }
                        (None, 0, 0) => name.clone(),
                        (None, _, _) => format!("{name}, +{additions}, -{deletions}"),
                    };
                    let fg = if is_selected { tc.text_strong } else { tc.text };
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
                        .accessibility_role(accesskit::Role::TreeItem)
                        .accessibility_id(accessibility_id)
                        .accessibility_label(accessibility_label)
                        .accessibility_selected(is_selected)
                        .on_click(on_select_file(original_index));

                    if !is_selected {
                        row_div = row_div.hover_bg(tc.sidebar_row_hover);
                    }

                    let indent_w = indent_unit * depth as f32 + chevron_size * scale + m.spacing_xs;
                    if indent_w > 0.1 {
                        row_div = row_div.child(div().w(indent_w).flex_shrink_0());
                    }

                    row_div = row_div
                        .child(super::file_icon::file_icon(&name, icon_size).selected(is_selected))
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .child(text(name).text_sm().color(fg).truncate()),
                        );

                    if additions > 0 || deletions > 0 {
                        let stats = view! {
                            <div class="flex-row shrink-0" gap={m.spacing_xs}>
                                if additions > 0 {
                                    <text class="text-xs" color={tc.line_add_text}>{format!("+{additions}")}</text>
                                }
                                if deletions > 0 {
                                    <text class="text-xs" color={tc.line_del_text}>{format!("-{deletions}")}</text>
                                }
                            </div>
                        };
                        row_div = row_div.child(stats);
                    }

                    if let Some(scope) = scope {
                        row_div = row_div.child(text(scope).text_xs().color(tc.text_muted));
                    }

                    if !status.is_empty() {
                        row_div = row_div.child(super::badge::status_badge(status));
                    }

                    row_div.into_any()
                }
            };

            container = container.child(
                div()
                    .w_full()
                    .h(wrapper_height)
                    .overflow_hidden()
                    .child(row_element),
            );
        }

        container.into_any()
    }
}
