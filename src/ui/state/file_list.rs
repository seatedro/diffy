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
                self.store.update(self.ui.sidebar_visible, |v| *v = !*v);
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileListEntry {
    pub path: ComparePath,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FileListStatus {
    #[default]
    None,
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    Untracked,
    Conflicted,
    TypeChanged,
    Binary,
}

impl FileListStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Modified => "M",
            Self::Renamed => "R",
            Self::Copied => "C",
            Self::Untracked => "U",
            Self::Conflicted => "!",
            Self::TypeChanged => "T",
            Self::Binary => "B",
        }
    }

    pub fn is_empty(self) -> bool {
        matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileListEntryMeta {
    pub status: FileListStatus,
    pub additions: i32,
    pub deletions: i32,
    pub is_binary: bool,
}

pub(super) fn file_change_list_status(
    status: FileChangeStatus,
    bucket: ChangeBucket,
) -> FileListStatus {
    match (status, bucket) {
        (FileChangeStatus::Added, _) => FileListStatus::Added,
        (FileChangeStatus::Deleted, _) => FileListStatus::Deleted,
        (FileChangeStatus::Renamed, _) => FileListStatus::Renamed,
        (FileChangeStatus::Copied, _) => FileListStatus::Copied,
        (FileChangeStatus::Untracked, _) => FileListStatus::Untracked,
        (FileChangeStatus::Conflicted, _) | (_, ChangeBucket::Conflicted) => {
            FileListStatus::Conflicted
        }
        (FileChangeStatus::TypeChanged, _) => FileListStatus::TypeChanged,
        (FileChangeStatus::Binary, _) => FileListStatus::Binary,
        (FileChangeStatus::Modified, _) => FileListStatus::Modified,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarMode {
    #[default]
    FlatList,
    TreeView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarTab {
    #[default]
    Files,
    Commits,
}

#[derive(Debug, Clone, PartialEq, Store)]
pub struct FileListState {
    pub scroll_offset_px: f32,
    pub commits_scroll_offset_px: f32,
    pub hovered_index: Option<usize>,
    pub row_height: f32,
    pub gap: f32,
    pub viewport_height: f32,
    pub filter: String,
    pub mode: SidebarMode,
    pub tab: SidebarTab,
    pub expanded_folders: HashSet<String>,
    pub viewed_files: HashSet<usize>,
}

impl Default for FileListState {
    fn default() -> Self {
        Self {
            scroll_offset_px: 0.0,
            commits_scroll_offset_px: 0.0,
            hovered_index: None,
            row_height: 36.0,
            gap: 4.0,
            viewport_height: 0.0,
            filter: String::new(),
            mode: SidebarMode::FlatList,
            tab: SidebarTab::Files,
            expanded_folders: HashSet::new(),
            viewed_files: HashSet::new(),
        }
    }
}

pub(super) fn carbon_list_status(status: carbon::FileStatus) -> FileListStatus {
    match status {
        carbon::FileStatus::Added => FileListStatus::Added,
        carbon::FileStatus::Deleted => FileListStatus::Deleted,
        carbon::FileStatus::Renamed | carbon::FileStatus::RenamedModified => {
            FileListStatus::Renamed
        }
        carbon::FileStatus::Binary => FileListStatus::Binary,
        carbon::FileStatus::ModeChanged | carbon::FileStatus::Modified => FileListStatus::Modified,
    }
}

pub(super) fn build_status_file_entries(changes: &[FileChange]) -> Vec<FileListEntry> {
    changes.iter().map(FileListEntry::from).collect()
}

pub(super) fn status_section_count(changes: &[FileChange]) -> usize {
    let mut last_bucket = None;
    let mut count = 0;
    for change in changes {
        if Some(change.bucket) != last_bucket {
            count += 1;
            last_bucket = Some(change.bucket);
        }
    }
    count
}

pub(super) fn status_section_count_before(changes: &[FileChange], len: usize) -> usize {
    status_section_count(&changes[..len.min(changes.len())])
}

impl From<&FileChange> for FileListEntry {
    fn from(value: &FileChange) -> Self {
        Self {
            path: ComparePath::from(value.path.as_str()),
        }
    }
}

pub(super) fn status_file_entry_meta(change: &FileChange) -> FileListEntryMeta {
    FileListEntryMeta {
        status: file_change_list_status(change.status, change.bucket),
        additions: 0,
        deletions: 0,
        is_binary: matches!(change.status, FileChangeStatus::Binary),
    }
}

impl AppState {
    pub fn workspace_file_entry_at(&self, index: usize) -> Option<FileListEntry> {
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare | WorkspaceSource::TextCompare => {
                if let Some(entry) = self.workspace.compare_output.with(&self.store, |output| {
                    output.as_ref().and_then(|output| {
                        output
                            .summary_at(index)
                            .map(|summary| compare_summary_file_entry(&summary))
                    })
                }) {
                    return Some(entry);
                }
                self.workspace
                    .files
                    .with(&self.store, |files| files.get(index).cloned())
            }
            WorkspaceSource::Status => self
                .workspace
                .status_file_changes
                .with(&self.store, |changes| {
                    changes.get(index).map(FileListEntry::from)
                })
                .or_else(|| {
                    self.workspace
                        .files
                        .with(&self.store, |files| files.get(index).cloned())
                }),
            WorkspaceSource::None => self
                .workspace
                .files
                .with(&self.store, |files| files.get(index).cloned()),
        }
    }

    pub fn for_each_workspace_file_path(&self, mut visit: impl FnMut(usize, &str)) {
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare | WorkspaceSource::TextCompare => {
                let visited = self.workspace.compare_output.with(&self.store, |output| {
                    let Some(output) = output.as_ref() else {
                        return false;
                    };
                    output.for_each_path(|index, path| visit(index, path));
                    true
                });
                if !visited {
                    self.workspace.files.with(&self.store, |files| {
                        for (index, file) in files.iter().enumerate() {
                            let path = file.path.path();
                            visit(index, path.as_ref());
                        }
                    });
                }
            }
            WorkspaceSource::Status => {
                self.workspace
                    .status_file_changes
                    .with(&self.store, |changes| {
                        for (index, change) in changes.iter().enumerate() {
                            visit(index, &change.path);
                        }
                    });
            }
            WorkspaceSource::None => {
                self.workspace.files.with(&self.store, |files| {
                    for (index, file) in files.iter().enumerate() {
                        let path = file.path.path();
                        visit(index, path.as_ref());
                    }
                });
            }
        }
    }

    pub fn workspace_max_file_path_chars(&self) -> usize {
        if matches!(
            self.workspace.source.get(&self.store),
            WorkspaceSource::Compare | WorkspaceSource::TextCompare
        ) {
            let chars = self.workspace.compare_output.with(&self.store, |output| {
                output
                    .as_ref()
                    .map(CompareOutput::max_path_chars)
                    .unwrap_or(0)
            });
            if chars > 0 {
                return chars;
            }
        }
        let mut max_chars = 0;
        self.for_each_workspace_file_path(|_, path| {
            max_chars = max_chars.max(path.chars().count());
        });
        max_chars
    }

    pub fn workspace_file_filter_matches(&self, filter: &str) -> Vec<usize> {
        let config = neo_frizbee::Config {
            max_typos: Some(2),
            sort: false,
            ..Default::default()
        };
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare | WorkspaceSource::TextCompare => {
                let matches = self.workspace.compare_output.with(&self.store, |output| {
                    let Some(output) = output.as_ref() else {
                        return None;
                    };
                    let mut matcher = neo_frizbee::Matcher::new(filter, &config);
                    let mut matches = Vec::new();
                    output.for_each_path(|index, path| {
                        if let Ok(offset) = u32::try_from(index) {
                            matcher.match_list_into(
                                std::slice::from_ref(&path),
                                offset,
                                &mut matches,
                            );
                        }
                    });
                    matches.sort_by(|a, b| b.score.cmp(&a.score));
                    Some(matches.iter().map(|m| m.index as usize).collect())
                });
                if let Some(matches) = matches {
                    matches
                } else {
                    self.workspace.files.with(&self.store, |files| {
                        let mut matcher = neo_frizbee::Matcher::new(filter, &config);
                        let mut matches = Vec::new();
                        for (index, file) in files.iter().enumerate() {
                            if let Ok(offset) = u32::try_from(index) {
                                let path = file.path.path();
                                let path_ref = path.as_ref();
                                matcher.match_list_into(
                                    std::slice::from_ref(&path_ref),
                                    offset,
                                    &mut matches,
                                );
                            }
                        }
                        matches.sort_by(|a, b| b.score.cmp(&a.score));
                        matches.iter().map(|m| m.index as usize).collect()
                    })
                }
            }
            WorkspaceSource::Status => {
                self.workspace
                    .status_file_changes
                    .with(&self.store, |changes| {
                        let haystack = changes
                            .iter()
                            .map(|change| change.path.as_str())
                            .collect::<Vec<_>>();
                        let mut matches = neo_frizbee::match_list(filter, &haystack, &config);
                        matches.sort_by(|a, b| b.score.cmp(&a.score));
                        matches.iter().map(|m| m.index as usize).collect()
                    })
            }
            WorkspaceSource::None => self.workspace.files.with(&self.store, |files| {
                let mut matcher = neo_frizbee::Matcher::new(filter, &config);
                let mut matches = Vec::new();
                for (index, file) in files.iter().enumerate() {
                    if let Ok(offset) = u32::try_from(index) {
                        let path = file.path.path();
                        let path_ref = path.as_ref();
                        matcher.match_list_into(
                            std::slice::from_ref(&path_ref),
                            offset,
                            &mut matches,
                        );
                    }
                }
                matches.sort_by(|a, b| b.score.cmp(&a.score));
                matches.iter().map(|m| m.index as usize).collect()
            }),
        }
    }

    pub fn workspace_file_tree_visible_row_count(
        &self,
        expanded_folders: &HashSet<String>,
    ) -> usize {
        crate::ui::components::file_tree_visible_row_count_by(
            |visit| {
                self.for_each_workspace_file_path(|_, path| visit(path));
            },
            expanded_folders,
        )
    }

    pub fn workspace_file_index_for_path(&self, path: &str) -> Option<usize> {
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare | WorkspaceSource::TextCompare => {
                if let Some(index) = self.workspace.compare_output.with(&self.store, |output| {
                    let output = output.as_ref()?;
                    let mut found = None;
                    output.for_each_path(|index, candidate| {
                        if found.is_none() && candidate == path {
                            found = Some(index);
                        }
                    });
                    found
                }) {
                    return Some(index);
                }
                self.workspace.files.with(&self.store, |files| {
                    files.iter().position(|file| file.path == path)
                })
            }
            WorkspaceSource::Status => self
                .workspace
                .status_file_changes
                .with(&self.store, |changes| {
                    changes.iter().position(|change| change.path == path)
                }),
            WorkspaceSource::None => self.workspace.files.with(&self.store, |files| {
                files.iter().position(|file| file.path == path)
            }),
        }
    }

    pub fn file_list_row_stride(&self) -> f32 {
        self.file_list.row_height.get(&self.store) + self.file_list.gap.get(&self.store)
    }

    pub fn file_list_total_content_height(&self, file_count: usize) -> f32 {
        if file_count == 0 {
            return 0.0;
        }
        file_count as f32 * self.file_list_row_stride() - self.file_list.gap.get(&self.store)
    }

    pub fn file_list_max_scroll_px(&self, file_count: usize) -> f32 {
        (self.file_list_total_content_height(file_count)
            - self.file_list.viewport_height.get(&self.store))
        .max(0.0)
    }

    pub fn file_list_clamp_scroll(&mut self, file_count: usize) {
        let max = self.file_list_max_scroll_px(file_count);
        let cur = self.file_list.scroll_offset_px.get(&self.store);
        self.file_list
            .scroll_offset_px
            .set_if_changed(&self.store, cur.clamp(0.0, max));
    }

    /// Scroll by a number of rows (positive = down).
    pub fn file_list_scroll_rows(&mut self, delta: i32, file_count: usize) {
        let px_delta = delta as f32 * self.file_list_row_stride();
        let cur = self.file_list.scroll_offset_px.get(&self.store);
        self.file_list
            .scroll_offset_px
            .set(&self.store, cur + px_delta);
        self.file_list_clamp_scroll(file_count);
    }

    /// Scroll by a raw pixel delta (positive = down).
    pub fn file_list_scroll_px(&mut self, delta_px: f32, file_count: usize) {
        let cur = self.file_list.scroll_offset_px.get(&self.store);
        self.file_list
            .scroll_offset_px
            .set(&self.store, cur + delta_px);
        self.file_list_clamp_scroll(file_count);
    }

    /// Reset every file-list signal back to its default value.
    pub fn reset_file_list(&mut self) {
        let d = FileListState::default();
        self.file_list
            .scroll_offset_px
            .set(&self.store, d.scroll_offset_px);
        self.file_list
            .commits_scroll_offset_px
            .set(&self.store, d.commits_scroll_offset_px);
        self.file_list
            .hovered_index
            .set(&self.store, d.hovered_index);
        self.file_list.row_height.set(&self.store, d.row_height);
        self.file_list.gap.set(&self.store, d.gap);
        self.file_list
            .viewport_height
            .set(&self.store, d.viewport_height);
        self.file_list.filter.set(&self.store, d.filter);
        self.file_list.mode.set(&self.store, d.mode);
        self.file_list.tab.set(&self.store, d.tab);
        self.file_list
            .expanded_folders
            .set(&self.store, d.expanded_folders);
        self.file_list.viewed_files.set(&self.store, d.viewed_files);
    }

    pub fn sidebar_row_count(&self) -> usize {
        if matches!(
            self.workspace.source.get(&self.store),
            WorkspaceSource::Compare | WorkspaceSource::TextCompare
        ) && self.file_list.tab.get(&self.store) == SidebarTab::Files
            && self.file_list.mode.get(&self.store) == SidebarMode::TreeView
            && self.file_list.filter.with(&self.store, |s| s.is_empty())
        {
            let expanded_folders = self.file_list.expanded_folders.get(&self.store);
            return self.workspace_file_tree_visible_row_count(&expanded_folders);
        }

        if self.workspace.source.get(&self.store) == WorkspaceSource::Status
            && self.file_list.filter.with(&self.store, |s| s.is_empty())
        {
            self.workspace.files.with(&self.store, |f| f.len())
                + self
                    .workspace
                    .status_file_changes
                    .with(&self.store, |s| status_section_count(s))
        } else {
            self.workspace_file_count()
        }
    }

    pub fn file_list_entry_meta(&self, index: usize) -> FileListEntryMeta {
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare | WorkspaceSource::TextCompare => {
                self.workspace.compare_output.with(&self.store, |output| {
                    output
                        .as_ref()
                        .and_then(|output| compare_output_file_entry_meta(output, index))
                        .unwrap_or_default()
                })
            }
            WorkspaceSource::Status => {
                self.workspace
                    .status_file_changes
                    .with(&self.store, |changes| {
                        changes
                            .get(index)
                            .map(status_file_entry_meta)
                            .unwrap_or_default()
                    })
            }
            WorkspaceSource::None => FileListEntryMeta::default(),
        }
    }

    pub(super) fn sidebar_row_index_for_file(&self, index: usize) -> usize {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status
            || !self.file_list.filter.with(&self.store, |s| s.is_empty())
        {
            return index;
        }
        index
            + self
                .workspace
                .status_file_changes
                .with(&self.store, |s| status_section_count_before(s, index + 1))
    }
}

impl AppState {
    pub(super) fn clamp_sidebar_width_px(&self, width: u32) -> u32 {
        let min_width = (280.0 * self.ui_scale_factor() * 0.64).round() as u32;
        width.max(min_width.max(120))
    }

    pub(super) fn shift_loaded_file(&mut self, delta: isize) -> Vec<Effect> {
        let file_count = self.workspace_file_count();
        if file_count == 0 {
            return Vec::new();
        }
        let current = self.reconcile_selected_file_index_from_path().unwrap_or(0);
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current
                .saturating_add(delta as usize)
                .min(file_count.saturating_sub(1))
        };
        self.select_file(next, true)
    }

    pub(super) fn select_file(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
        if self.settings.continuous_scroll
            && !matches!(
                self.workspace.source.get(&self.store),
                WorkspaceSource::None
            )
        {
            let target = self
                .file_start_offset_px(index)
                .min(self.global_max_scroll_top_px());
            self.set_viewport_anchor_for_global(target, ViewportAnchorBias::PreserveTop);
            self.workspace.global_scroll_top_px.set(&self.store, target);
        }
        self.select_file_inner(index, reveal)
    }

    pub(super) fn select_file_inner(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare => self.select_compare_file(index, reveal),
            WorkspaceSource::TextCompare => self.select_text_compare_file(index, reveal),
            WorkspaceSource::Status => self.select_status_item(index, reveal),
            WorkspaceSource::None => {
                self.startup.preferred_file_index = Some(index);
                Vec::new()
            }
        }
    }

    pub(super) fn active_file_matches_workspace_file(&self, index: usize) -> bool {
        let Some(path) = self.workspace_file_path_at(index) else {
            return false;
        };
        let source = self.workspace.source.get(&self.store);
        let selected_bucket = self.workspace.selected_change_bucket.get(&self.store);
        self.workspace.active_file.with(&self.store, |active| {
            active.as_ref().is_some_and(|active| {
                if active.index != index || active.path != path {
                    return false;
                }
                match source {
                    WorkspaceSource::Status => selected_bucket.is_some_and(|bucket| {
                        let (left_ref, right_ref) = self.status_refs_for_bucket(bucket);
                        active.left_ref == left_ref && active.right_ref == right_ref
                    }),
                    WorkspaceSource::Compare | WorkspaceSource::TextCompare => true,
                    WorkspaceSource::None => false,
                }
            })
        })
    }

    pub(super) fn select_text_compare_file(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
        let Some(entry) = self.workspace_file_entry_at(index) else {
            self.push_error("Selected file index is out of range.");
            return Vec::new();
        };
        let mut effects = vec![
            SyntaxEffect::EnsureSyntaxPackForPath {
                path: entry.path.to_string(),
            }
            .into(),
        ];
        effects.extend(self.select_loaded_compare_file(index, reveal));
        effects
    }

    pub(super) fn select_compare_file(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
        let Some(entry) = self.workspace_file_entry_at(index) else {
            self.push_error("Selected file index is out of range.");
            return Vec::new();
        };

        if !self.compare_file_is_large(index) {
            let mut effects = vec![
                SyntaxEffect::EnsureSyntaxPackForPath {
                    path: entry.path.to_string(),
                }
                .into(),
            ];
            effects.extend(self.select_loaded_compare_file(index, reveal));
            return effects;
        }

        let entry_path = entry.path.to_string();

        if let Some(mut active_file) = self.cached_compare_file_at(index, &entry_path) {
            active_file.last_used_tick = self.next_file_working_set_tick();
            self.workspace
                .selected_file_index
                .set(&self.store, Some(index));
            self.workspace
                .selected_file_path
                .set(&self.store, Some(entry_path.clone()));
            self.workspace.selected_change_bucket.set(&self.store, None);
            self.workspace.active_file_loading.set(&self.store, None);
            self.workspace
                .active_file
                .set(&self.store, Some(active_file.clone()));
            self.cache_active_file(active_file);
            self.workspace.compare_progress.set(&self.store, None);
            self.editor_clear_document();
            self.file_list.hovered_index.set(&self.store, Some(index));
            if reveal {
                self.reveal_file_list_row(index);
            }
            let mut effects = self.sync_editor_scroll_from_global();
            effects.push(SyntaxEffect::EnsureSyntaxPackForPath { path: entry_path }.into());
            effects.extend(self.request_active_file_syntax_effect());
            return effects;
        }

        let should_load = self.should_enqueue_file_load(
            index,
            &entry_path,
            CompareWorkPriority::InteractiveSelectedFile,
        );

        // If we're mid-compare (first file selection post-CompareFinished),
        // flip the phase so the progress panel reports "Preparing first
        // file…". Subsequent selections don't touch compare_progress.
        self.workspace.compare_progress.update(&self.store, |slot| {
            if let Some(p) = slot.as_mut() {
                Arc::make_mut(p).phase = ComparePhase::RenderingFirstFile;
            }
        });

        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            self.push_error("Open a repository before selecting a compare file.");
            return Vec::new();
        };
        let deferred_file = self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .and_then(|output| compare_output_deferred_summary(output, index))
        });

        self.workspace
            .selected_file_index
            .set(&self.store, Some(index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(entry_path.clone()));
        self.workspace.selected_change_bucket.set(&self.store, None);
        self.workspace.active_file.set(&self.store, None);
        self.workspace.active_file_loading.set(
            &self.store,
            Some(ActiveFileLoading {
                index,
                path: entry_path.clone(),
                priority: CompareWorkPriority::InteractiveSelectedFile,
            }),
        );
        self.mark_file_cache_loading(
            index,
            entry_path.clone(),
            CompareWorkPriority::InteractiveSelectedFile,
        );
        self.editor_clear_document();
        self.file_list.hovered_index.set(&self.store, Some(index));
        if reveal {
            self.reveal_file_list_row(index);
        }

        let mut effects = vec![
            SyntaxEffect::EnsureSyntaxPackForPath {
                path: entry_path.clone(),
            }
            .into(),
        ];
        if should_load {
            effects.push(
                CompareEffect::LoadFile(Task {
                    generation: self.workspace.compare_generation.get(&self.store),
                    request: CompareFileRequest {
                        repo_path,
                        request: vcs_compare_request(
                            self.compare.mode.get(&self.store),
                            self.compare.left_ref.get(&self.store),
                            self.compare.right_ref.get(&self.store),
                            self.compare.layout.get(&self.store),
                            self.compare.renderer.get(&self.store),
                        ),
                        path: entry_path,
                        index,
                        deferred_file,
                        priority: CompareWorkPriority::InteractiveSelectedFile,
                    },
                })
                .into(),
            );
        }
        effects
    }

    #[profiling::function]
    pub(super) fn select_loaded_compare_file(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
        let mut selected_path = None;
        let mut prepared = None;
        let mut oob = false;
        self.workspace
            .compare_output
            .update(&self.store, |maybe_output| {
                let Some(output) = maybe_output.as_mut() else {
                    return;
                };
                let Some(carbon_file) = output.carbon.files.get(index) else {
                    oob = true;
                    return;
                };
                selected_path = Some(carbon_file.path().to_owned());
                prepared = Some(prepare_active_file(index, carbon_file));
            });

        let Some(prepared) = prepared else {
            if oob {
                self.push_error("Selected file index is out of range.");
                return Vec::new();
            }
            self.startup.preferred_file_index = Some(index);
            return Vec::new();
        };

        let Some(path) = selected_path else {
            self.startup.preferred_file_index = Some(index);
            return Vec::new();
        };

        self.install_compare_active_file(index, path, prepared);
        if reveal {
            self.reveal_file_list_row(index);
        }
        let mut effects = self.sync_editor_scroll_from_global();
        effects.extend(self.request_active_file_syntax_effect());
        effects
    }

    pub(super) fn reveal_file_list_row(&mut self, index: usize) {
        let row_top = self.sidebar_row_index_for_file(index) as f32 * self.file_list_row_stride();
        let row_bottom = row_top + self.file_list.row_height.get(&self.store);
        let scroll = self.file_list.scroll_offset_px.get(&self.store);
        let viewport = self.file_list.viewport_height.get(&self.store);
        if row_top < scroll {
            self.file_list.scroll_offset_px.set(&self.store, row_top);
        } else if row_bottom > scroll + viewport {
            self.file_list
                .scroll_offset_px
                .set(&self.store, row_bottom - viewport);
        }
        self.file_list_clamp_scroll(self.sidebar_row_count());
    }

    pub fn workspace_file_count(&self) -> usize {
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare | WorkspaceSource::TextCompare => {
                let count = self.workspace.compare_output.with(&self.store, |output| {
                    output.as_ref().map(CompareOutput::file_count).unwrap_or(0)
                });
                count.max(self.workspace.files.with(&self.store, |f| f.len()))
            }
            WorkspaceSource::Status => self
                .workspace
                .status_file_changes
                .with(&self.store, |s| s.len()),
            WorkspaceSource::None => self.workspace.files.with(&self.store, |f| f.len()),
        }
    }

    pub fn workspace_file_path_at(&self, index: usize) -> Option<String> {
        self.workspace_file_entry_at(index)
            .map(|entry| entry.path.to_string())
    }

    pub fn selected_workspace_file_index(&self) -> Option<usize> {
        let count = self.workspace_file_count();
        let selected_index = self
            .workspace
            .selected_file_index
            .get(&self.store)
            .filter(|index| *index < count);

        if let Some(path) = self.workspace.selected_file_path.get(&self.store) {
            if let Some(index) = selected_index
                && self
                    .workspace_file_entry_at(index)
                    .is_some_and(|entry| entry.path == path.as_str())
            {
                return Some(index);
            }
            if let Some(index) = self.workspace_file_index_for_path(&path) {
                return Some(index);
            }
        }

        selected_index
    }

    pub(super) fn reconcile_selected_file_index_from_path(&mut self) -> Option<usize> {
        let resolved = self.selected_workspace_file_index();
        if let Some(index) = resolved
            && self.workspace.selected_file_index.get(&self.store) != Some(index)
        {
            self.workspace
                .selected_file_index
                .set(&self.store, Some(index));
        }
        resolved
    }

    pub fn workspace_render_generation(&self) -> u64 {
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare => self.workspace.compare_generation.get(&self.store),
            WorkspaceSource::TextCompare => self.text_compare.generation,
            WorkspaceSource::Status => self.workspace.status_generation.get(&self.store),
            WorkspaceSource::None => 0,
        }
    }
}
