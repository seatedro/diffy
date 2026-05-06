use crate::actions::FileListAction;
use crate::effects::Effect;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: FileListAction) -> Vec<Effect> {
    state.apply_file_list_action(action)
}

impl AppState {
    pub(super) fn apply_file_list_action(&mut self, action: FileListAction) -> Vec<Effect> {
        use FileListAction::*;
        match action {
            SelectFile(index) => self.select_file(index, false),
            SelectFilePath(path) => {
                let idx = self.workspace_file_index_for_path(&path);
                if let Some(index) = idx {
                    return self.select_file(index, true);
                } else {
                    self.startup.preferred_file_path = Some(path);
                }
                Vec::new()
            }
            SelectNextFile => self.shift_loaded_file(1),
            SelectPreviousFile => self.shift_loaded_file(-1),
            ScrollFileList(delta) => {
                self.file_list_scroll_rows(delta, self.sidebar_row_count());
                self.start_visible_compare_stats_hydration()
                    .into_iter()
                    .collect()
            }
            ScrollFileListPx(delta_px) => {
                self.file_list_scroll_px(delta_px as f32, self.sidebar_row_count());
                self.start_visible_compare_stats_hydration()
                    .into_iter()
                    .collect()
            }
            ScrollFileListToPx(px) => {
                self.file_list.scroll_offset_px.set(&self.store, px as f32);
                self.file_list_clamp_scroll(self.sidebar_row_count());
                self.start_visible_compare_stats_hydration()
                    .into_iter()
                    .collect()
            }
            HoverFile(index) => {
                use crate::ui::animation::AnimationKey;
                if let Some(prev) = self.file_list.hovered_index.get(&self.store) {
                    self.animation.set_target(
                        AnimationKey::FileListHover(prev),
                        0.0,
                        150,
                        self.clock_ms,
                    );
                }
                if let Some(next) = index {
                    self.animation.set_target(
                        AnimationKey::FileListHover(next),
                        1.0,
                        150,
                        self.clock_ms,
                    );
                }
                self.file_list.hovered_index.set(&self.store, index);
                Vec::new()
            }
            ToggleFolder(path) => {
                self.file_list.expanded_folders.update(&self.store, |set| {
                    if set.contains(&path) {
                        set.remove(&path);
                    } else {
                        set.insert(path);
                    }
                });
                self.start_visible_compare_stats_hydration()
                    .into_iter()
                    .collect()
            }
            ToggleFileViewed(index) => {
                self.file_list.viewed_files.update(&self.store, |set| {
                    if set.contains(&index) {
                        set.remove(&index);
                    } else {
                        set.insert(index);
                    }
                });
                Vec::new()
            }
            SetSidebarFilter(query) => {
                self.file_list.filter.set(&self.store, query);
                if self.file_list.tab.get(&self.store) == SidebarTab::Commits {
                    self.file_list
                        .commits_scroll_offset_px
                        .set(&self.store, 0.0);
                } else {
                    self.file_list.scroll_offset_px.set(&self.store, 0.0);
                }
                self.start_visible_compare_stats_hydration()
                    .into_iter()
                    .collect()
            }
            ClearSidebarFilter => {
                self.file_list.filter.update(&self.store, |s| s.clear());
                if self.file_list.tab.get(&self.store) == SidebarTab::Commits {
                    self.file_list
                        .commits_scroll_offset_px
                        .set(&self.store, 0.0);
                } else {
                    self.file_list.scroll_offset_px.set(&self.store, 0.0);
                }
                self.start_visible_compare_stats_hydration()
                    .into_iter()
                    .collect()
            }
            ToggleSidebar => {
                self.store.update(self.sidebar_visible, |v| *v = !*v);
                Vec::new()
            }
            ToggleSidebarMode => {
                let next = match self.file_list.mode.get(&self.store) {
                    SidebarMode::FlatList => SidebarMode::TreeView,
                    SidebarMode::TreeView => SidebarMode::FlatList,
                };
                self.file_list.mode.set(&self.store, next);
                self.file_list.scroll_offset_px.set(&self.store, 0.0);
                self.start_visible_compare_stats_hydration()
                    .into_iter()
                    .collect()
            }
            ExpandAllFolders => {
                let mut expanded = self.file_list.expanded_folders.get(&self.store);
                self.for_each_workspace_file_path(|_, path| {
                    insert_folder_prefixes(path, &mut expanded);
                });
                self.file_list.expanded_folders.set(&self.store, expanded);
                self.start_visible_compare_stats_hydration()
                    .into_iter()
                    .collect()
            }
            CollapseAllFolders => {
                self.file_list
                    .expanded_folders
                    .update(&self.store, |s| s.clear());
                self.start_visible_compare_stats_hydration()
                    .into_iter()
                    .collect()
            }
            SetSidebarTab(tab) => {
                self.file_list.tab.set(&self.store, tab);
                self.file_list.filter.update(&self.store, |s| s.clear());
                if tab == SidebarTab::Commits {
                    self.take_pending_compare_history_effect()
                        .into_iter()
                        .collect()
                } else {
                    self.start_visible_compare_stats_hydration()
                        .into_iter()
                        .collect()
                }
            }
            ScrollCommitListPx(delta) => {
                let stride = self.file_list_row_stride();
                let commit_count = self.workspace.range_commits.with(&self.store, |c| c.len());
                let total = self.file_list_total_content_height(commit_count);
                let max_scroll = (total - self.file_list.viewport_height.get(&self.store)).max(0.0);
                let cur = self.file_list.commits_scroll_offset_px.get(&self.store);
                self.file_list.commits_scroll_offset_px.set(
                    &self.store,
                    (cur + delta as f32 * stride).clamp(0.0, max_scroll),
                );
                Vec::new()
            }
        }
    }
}

fn insert_folder_prefixes(path: &str, set: &mut HashSet<String>) {
    for (index, _) in path.match_indices('/') {
        set.insert(path[..index].to_owned());
    }
}
