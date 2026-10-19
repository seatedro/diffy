use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use quark_ui::FocusId;
use quark_ui::element::ScrollbarVisibility;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome, TextField};

use super::rows::{self, Row, RowCache};

use crate::app::text_targets::{SIDEBAR_FILTER, TextCx, TextHost};
use crate::contracts::commands::{CommandsAction, RefPickerRequest, SettingsSnapshot, ToastUpdate};
use crate::contracts::navigation::{
    CompareField, HistoryRequest, NavigationAction, NavigationEffect, NavigationEvent,
    RepositoryChangeKind, RepositoryContext, RepositoryRevision, RepositorySnapshot,
    RepositorySyncReason, ResolveRefRequest, SelectedFile, SelectionOrigin, SidebarMode,
    SidebarTab, SyncRepositoryRequest,
};
use crate::contracts::review::ReviewAction;
use crate::contracts::shared::{CompareGeneration, FileKey, JobId, SharedContext};
use crate::contracts::shell::{
    Action, AppStartup, Controller, Effect, Intent, LogicalFocus, Reduction, ShellEffect,
};
use crate::contracts::viewport::CompareIntent;
use crate::contracts::viewport::pipeline::{
    CompareManifestSnapshot, CompareWorkPriority, FileStats, ManifestFile, PipelineAction,
};
use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::core::ref_catalog::WORKING_TREE_LABEL;
use crate::core::vcs::model::{RefKind, VcsChange, VcsCompareRequest, VcsCompareSpec, VcsKind};
use crate::platform::persistence::{PersistedCompare, Settings};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoStatus {
    Closed,
    Opening,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareSelection {
    pub left: String,
    pub right: String,
    pub mode: CompareMode,
    pub layout: LayoutMode,
    pub renderer: RendererKind,
    pub working_copy: bool,
}

impl Default for CompareSelection {
    fn default() -> Self {
        Self {
            left: String::new(),
            right: String::new(),
            mode: CompareMode::default(),
            layout: LayoutMode::default(),
            renderer: RendererKind::default(),
            working_copy: false,
        }
    }
}

impl CompareSelection {
    pub fn refs_valid(&self) -> bool {
        match self.mode {
            CompareMode::SingleCommit => !self.left.is_empty() || !self.right.is_empty(),
            CompareMode::TwoDot | CompareMode::ThreeDot => {
                !self.left.is_empty() && !self.right.is_empty()
            }
        }
    }

    pub fn request(&self) -> VcsCompareRequest {
        let spec = if self.working_copy {
            VcsCompareSpec::WorkingCopy
        } else {
            match self.mode {
                CompareMode::SingleCommit => VcsCompareSpec::Change {
                    revision: if self.right.is_empty() {
                        self.left.clone()
                    } else {
                        self.right.clone()
                    },
                },
                CompareMode::TwoDot => VcsCompareSpec::Range {
                    from: self.left.clone(),
                    to: self.right.clone(),
                },
                CompareMode::ThreeDot => VcsCompareSpec::MergeBaseRange {
                    base: self.left.clone(),
                    head: self.right.clone(),
                },
            }
        };
        VcsCompareRequest {
            spec,
            layout: self.layout,
            renderer: self.renderer,
        }
    }

    fn auto_select_mode(&mut self) {
        if self.left.is_empty() || self.right.is_empty() {
            return;
        }
        if self.left == self.right {
            self.mode = CompareMode::SingleCommit;
            return;
        }
        let trunk = |r: &str| matches!(r, "main" | "master" | "develop" | "development");
        if trunk(&self.left) != trunk(&self.right) {
            self.mode = CompareMode::ThreeDot;
        }
    }
}

#[derive(Debug, Clone, Default)]
struct StartupPlan {
    auto_compare: bool,
    bootstrap_compare: bool,
    pr_pending: bool,
    file_index: Option<usize>,
    file_path: Option<String>,
}

#[derive(Debug, Clone)]
struct Selection {
    key: FileKey,
    path: String,
}

#[derive(Debug, Default)]
pub struct HistoryState {
    generation: u64,
    range: Option<(String, String)>,
    pub commits: Option<Arc<[VcsChange]>>,
    pub loading: bool,
    pub error: Option<String>,
    pub selected: Option<String>,
    pre_drill: Option<CompareSelection>,
    pub scroll_px: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ListMetrics {
    pub row_height: f32,
    pub viewport: f32,
    pub history_row: f32,
}

#[derive(Debug)]
pub struct NavigationController {
    filter: TextField,
    repo_path: Option<PathBuf>,
    status: RepoStatus,
    repository: Option<Rc<RepositoryContext>>,
    compare: CompareSelection,
    startup: Option<StartupPlan>,
    visible: bool,
    tab: SidebarTab,
    mode: SidebarMode,
    expanded: HashSet<String>,
    expanded_revision: u64,
    viewed: HashSet<String>,
    scroll_px: f32,
    manifest: Option<Rc<CompareManifestSnapshot>>,
    selection: Option<Selection>,
    rows: RowCache,
    history: HistoryState,
    selected_operation: Option<String>,
    metrics: ListMetrics,
    resolved: [Option<String>; 2],
    next_job: u64,
    compare_requested: bool,
    stats_requested: HashSet<FileKey>,
    stats_generation: Option<CompareGeneration>,
    outbox: RefCell<Vec<Action>>,
    marked: Vec<FileKey>,
    anchor: Option<FileKey>,
    modifiers: Cell<ClickModifiers>,
    banner: RefCell<super::banner::BannerCache>,
    tags: RefCell<(usize, Rc<HashMap<String, Vec<String>>>)>,
    avatars_requested: HashSet<String>,
    scrollbars: [ScrollbarVisibility; 2],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClickModifiers {
    pub extend: bool,
    pub toggle: bool,
}

const STATS_OVERSCAN_ROWS: usize = 8;
const STATS_FALLBACK_ROWS: usize = 64;

impl Default for NavigationController {
    fn default() -> Self {
        Self {
            filter: TextField::default(),
            repo_path: None,
            status: RepoStatus::Closed,
            repository: None,
            compare: CompareSelection::default(),
            startup: None,
            visible: true,
            tab: SidebarTab::Files,
            mode: SidebarMode::List,
            expanded: HashSet::new(),
            expanded_revision: 0,
            viewed: HashSet::new(),
            scroll_px: 0.0,
            manifest: None,
            selection: None,
            rows: RowCache::default(),
            history: HistoryState::default(),
            selected_operation: None,
            metrics: ListMetrics::default(),
            resolved: [None, None],
            next_job: 0,
            compare_requested: false,
            stats_requested: HashSet::new(),
            stats_generation: None,
            outbox: RefCell::new(Vec::new()),
            marked: Vec::new(),
            anchor: None,
            modifiers: Cell::new(ClickModifiers::default()),
            banner: RefCell::new(None),
            tags: RefCell::new((0, Rc::default())),
            avatars_requested: HashSet::new(),
            scrollbars: Default::default(),
        }
    }
}

impl NavigationController {
    pub fn filter(&self) -> &TextField {
        &self.filter
    }

    pub fn repo_path(&self) -> Option<&Path> {
        self.repo_path.as_deref()
    }

    pub fn status(&self) -> &RepoStatus {
        &self.status
    }

    pub fn compare(&self) -> &CompareSelection {
        &self.compare
    }

    pub fn repository(&self) -> Option<&Rc<RepositoryContext>> {
        self.repository.as_ref()
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn tab(&self) -> SidebarTab {
        self.tab
    }

    pub fn mode(&self) -> SidebarMode {
        self.mode
    }

    pub fn history(&self) -> &HistoryState {
        &self.history
    }

    pub fn selected_operation(&self) -> Option<&str> {
        self.selected_operation.as_deref()
    }

    pub fn manifest(&self) -> Option<&Rc<CompareManifestSnapshot>> {
        self.manifest.as_ref()
    }

    pub fn merge_base_hint(&self) -> Option<(String, String)> {
        if self.compare.working_copy || self.compare.mode != CompareMode::ThreeDot {
            return None;
        }
        let manifest = self.manifest.as_ref().filter(|m| m.files.is_empty())?;
        let (base, head) = self.compare_labels();
        let detail = if manifest.resolved_left == manifest.resolved_right {
            format!("{base} already contains every commit in {head}")
        } else {
            format!("{head} has no changes since it branched from {base}")
        };
        Some(("No changes since the merge base".to_owned(), detail))
    }

    pub fn files_scrollbar(&self) -> &ScrollbarVisibility {
        &self.scrollbars[0]
    }

    pub fn history_scrollbar(&self) -> &ScrollbarVisibility {
        &self.scrollbars[1]
    }

    pub fn selected_key(&self) -> Option<FileKey> {
        self.selection.as_ref().map(|s| s.key)
    }

    pub fn scroll_px(&self) -> f32 {
        self.scroll_px
    }

    pub fn is_viewed(&self, path: &str) -> bool {
        self.viewed.contains(path)
    }

    pub fn resolved(&self, field: CompareField) -> Option<&str> {
        self.resolved[field_index(field)].as_deref()
    }

    pub fn operation_log_available(&self) -> bool {
        self.repository
            .as_ref()
            .is_some_and(|r| r.snapshot.capabilities.operation_log)
    }

    pub fn tags_by_target(&self) -> Rc<HashMap<String, Vec<String>>> {
        let Some(repo) = &self.repository else {
            return Rc::default();
        };
        let identity = Arc::as_ptr(&repo.snapshot) as usize;
        let mut cache = self.tags.borrow_mut();
        if cache.0 != identity {
            let mut map: HashMap<String, Vec<String>> = HashMap::new();
            for r in repo.snapshot.refs.iter().filter(|r| r.kind == RefKind::Tag) {
                map.entry(r.target.id.clone())
                    .or_default()
                    .push(r.name.clone());
            }
            *cache = (identity, Rc::new(map));
        }
        cache.1.clone()
    }

    pub(super) fn banner_cache(&self) -> &RefCell<super::banner::BannerCache> {
        &self.banner
    }

    pub fn find_commit(&self, revision: &str) -> Option<&VcsChange> {
        let matches =
            |c: &&VcsChange| c.revision.id.starts_with(revision) || c.short_revision == revision;
        self.history
            .commits
            .as_deref()
            .and_then(|commits| commits.iter().find(matches))
            .or_else(|| {
                self.repository
                    .as_ref()
                    .and_then(|repo| repo.snapshot.changes.iter().find(matches))
            })
    }

    pub fn viewing_commit(&self) -> bool {
        self.history.selected.is_some()
    }

    pub fn observe_modifiers(&self, modifiers: ClickModifiers) {
        self.modifiers.set(modifiers);
    }

    pub fn marked(&self) -> &[FileKey] {
        &self.marked
    }

    pub fn is_marked(&self, key: FileKey) -> bool {
        self.marked.contains(&key)
    }

    pub fn viewed_count(&self) -> (usize, usize) {
        let files = self.files();
        let viewed = files
            .iter()
            .filter(|f| self.viewed.contains(f.paths.display_path().as_ref()))
            .count();
        (viewed, files.len())
    }

    pub fn highlights(&self, file: u32) -> &[(usize, usize)] {
        self.rows.highlights(file)
    }

    fn mark(&mut self, key: FileKey) {
        let modifiers = self.modifiers.get();
        let current = self.selection.as_ref().map(|s| s.key);
        if modifiers.extend {
            let anchor = self.anchor.or(current).unwrap_or(key);
            self.ensure_rows();
            let order: Vec<FileKey> = self
                .rows
                .file_rows()
                .iter()
                .filter_map(|&row| match self.rows.rows()[row] {
                    Row::File { file, .. } => self.files().get(file as usize).map(|f| f.key),
                    _ => None,
                })
                .collect();
            let at = |k: FileKey| order.iter().position(|&o| o == k);
            if let (Some(a), Some(b)) = (at(anchor), at(key)) {
                let (lo, hi) = (a.min(b), a.max(b));
                self.marked = order[lo..=hi].to_vec();
            }
            self.anchor = Some(anchor);
            return;
        }
        if modifiers.toggle {
            if self.marked.is_empty()
                && let Some(current) = current
            {
                self.marked.push(current);
            }
            match self.marked.iter().position(|&k| k == key) {
                Some(index) => {
                    self.marked.remove(index);
                }
                None => self.marked.push(key),
            }
            self.anchor = Some(key);
            return;
        }
        self.marked.clear();
        self.anchor = Some(key);
    }

    pub fn compare_labels(&self) -> (String, String) {
        if self.compare.working_copy {
            let base = self
                .repository
                .as_ref()
                .and_then(|repo| {
                    repo.snapshot
                        .refs
                        .iter()
                        .find(|r| r.active && matches!(r.kind, RefKind::Branch | RefKind::Bookmark))
                        .map(|r| r.name.clone())
                })
                .unwrap_or_else(|| "HEAD".to_owned());
            return (base, WORKING_TREE_LABEL.to_owned());
        }
        let shown = |reference: &str| match reference {
            "" => String::new(),
            r if is_working_copy_ref(r) => WORKING_TREE_LABEL.to_owned(),
            r if r.len() >= 12 && r.bytes().all(|b| b.is_ascii_hexdigit()) => r[..7].to_owned(),
            r => r.to_owned(),
        };
        if self.compare.mode == CompareMode::SingleCommit {
            let commit = if self.compare.right.is_empty() {
                &self.compare.left
            } else {
                &self.compare.right
            };
            return match self.find_commit(commit) {
                Some(found) => {
                    let parent = found.parents.first().map_or_else(
                        || format!("{}^", found.short_revision),
                        |p| p[..p.len().min(7)].to_owned(),
                    );
                    (parent, found.short_revision.clone())
                }
                None => {
                    let commit = shown(commit);
                    (format!("{commit}^"), commit)
                }
            };
        }
        (shown(&self.compare.left), shown(&self.compare.right))
    }

    pub fn take_dispatch(&self) -> Vec<Action> {
        self.outbox.take()
    }

    fn queue_visible_avatars(&mut self) {
        if self.tab != SidebarTab::Commits || !self.visible {
            return;
        }
        let row = self.metrics.history_row;
        let (first, count) = if row > 0.0 && self.metrics.viewport > 0.0 {
            (
                (self.history.scroll_px / row).floor().max(0.0) as usize,
                (self.metrics.viewport / row).ceil() as usize + STATS_OVERSCAN_ROWS,
            )
        } else {
            (0, STATS_FALLBACK_ROWS)
        };
        let emails: Vec<String> = self
            .history_commits()
            .map(|commits| {
                commits
                    .iter()
                    .skip(first)
                    .take(count)
                    .filter_map(|c| c.author_email.clone())
                    .collect()
            })
            .unwrap_or_default();
        let fresh: Vec<String> = emails
            .into_iter()
            .filter(|email| self.avatars_requested.insert(email.clone()))
            .collect();
        if !fresh.is_empty() {
            self.outbox
                .borrow_mut()
                .push(ReviewAction::RequestAuthorAvatars(fresh).into());
        }
    }

    fn queue_visible_stats(&mut self) {
        let Some(manifest) = self.manifest.clone() else {
            return;
        };
        if self.tab != SidebarTab::Files || !self.visible {
            return;
        }
        if self.stats_generation != Some(manifest.generation) {
            self.stats_generation = Some(manifest.generation);
            self.stats_requested.clear();
        }
        self.ensure_rows();
        let rows = self.rows.rows();
        let row_height = self.metrics.row_height;
        let range = if row_height > 0.0 && self.metrics.viewport > 0.0 {
            let first = (self.scroll_px / row_height).floor().max(0.0) as usize;
            let count = (self.metrics.viewport / row_height).ceil() as usize;
            let start = first.saturating_sub(STATS_OVERSCAN_ROWS).min(rows.len());
            start..(first + count + STATS_OVERSCAN_ROWS).min(rows.len())
        } else {
            0..STATS_FALLBACK_ROWS.min(rows.len())
        };
        let mut files = Vec::new();
        for row in &rows[range] {
            let Row::File { file, .. } = row else {
                continue;
            };
            let Some(file) = manifest.files.get(*file as usize) else {
                continue;
            };
            if file.stats == FileStats::Pending && self.stats_requested.insert(file.key) {
                files.push(file.key);
            }
        }
        if !files.is_empty() {
            self.outbox.borrow_mut().push(
                PipelineAction::RequestStats {
                    files,
                    priority: CompareWorkPriority::VisibleSidebarStats,
                }
                .into(),
            );
        }
    }

    pub(super) fn set_metrics(&mut self, metrics: ListMetrics) {
        let changed = self.metrics.row_height != metrics.row_height
            || self.metrics.viewport != metrics.viewport;
        self.metrics = metrics;
        self.clamp_scroll();
        if changed {
            self.queue_visible_stats();
            self.queue_visible_avatars();
        }
    }

    pub fn rows(&mut self) -> &[Row] {
        self.ensure_rows();
        self.rows.rows()
    }

    pub fn cached_rows(&self) -> &[Row] {
        self.rows.rows()
    }

    fn ensure_rows(&mut self) {
        match &self.manifest {
            Some(manifest) => self.rows.ensure(
                &manifest.files,
                self.mode,
                self.filter.text(),
                &self.expanded,
                self.expanded_revision,
                self.selected_key()
                    .and_then(|key| manifest.files.iter().position(|f| f.key == key))
                    .map(|i| i as u32),
            ),
            None => self.rows.invalidate(),
        }
    }

    fn files(&self) -> &[ManifestFile] {
        self.manifest.as_ref().map_or(&[], |m| &m.files)
    }

    fn file_index(&self, key: FileKey) -> Option<usize> {
        self.files().iter().position(|f| f.key == key)
    }

    fn next_job(&mut self) -> JobId {
        self.next_job += 1;
        JobId(self.next_job)
    }

    fn row_count(&mut self) -> usize {
        self.ensure_rows();
        self.rows.rows().len()
    }

    fn clamp_scroll(&mut self) {
        let total = self.row_count() as f32 * self.metrics.row_height;
        let max = (total - self.metrics.viewport).max(0.0);
        self.scroll_px = self.scroll_px.clamp(0.0, max);
    }

    fn reveal_row(&mut self, row: usize) {
        let h = self.metrics.row_height;
        if h <= 0.0 {
            return;
        }
        let top = row as f32 * h;
        if top < self.scroll_px {
            self.scroll_px = top;
        } else if top + h > self.scroll_px + self.metrics.viewport {
            self.scroll_px = top + h - self.metrics.viewport;
        }
        self.clamp_scroll();
    }

    fn reveal_selected(&mut self) {
        let Some(selection) = &self.selection else {
            return;
        };
        if self.mode == SidebarMode::Tree && self.filter.text().trim().is_empty() {
            let before = self.expanded.len();
            rows::folder_prefixes(&selection.path, &mut self.expanded);
            if self.expanded.len() != before {
                self.expanded_revision += 1;
            }
        }
        let Some(index) = self.selection.as_ref().and_then(|s| self.file_index(s.key)) else {
            return;
        };
        self.ensure_rows();
        if let Some(row) = self.rows.row_of_file(index as u32) {
            self.reveal_row(row);
        }
    }

    fn select(&mut self, key: FileKey, origin: SelectionOrigin) -> Reduction {
        let Some(index) = self.file_index(key) else {
            return Reduction::default();
        };
        if origin == SelectionOrigin::Sidebar {
            self.mark(key);
        }
        let path = self.files()[index].paths.display_path().into_owned();
        self.selection = Some(Selection { key, path });
        self.reveal_selected();
        let reduction = Reduction::changed();
        match origin {
            SelectionOrigin::Viewport => reduction,
            origin => reduction.intent(Intent::SelectFile(Some(SelectedFile { key, origin }))),
        }
    }

    fn shift_selection(&mut self, delta: isize) -> Reduction {
        self.ensure_rows();
        let file_rows = self.rows.file_rows();
        if file_rows.is_empty() {
            return Reduction::default();
        }
        let current = self
            .selection
            .as_ref()
            .and_then(|s| self.files().iter().position(|f| f.key == s.key))
            .and_then(|index| self.rows.row_of_file(index as u32))
            .and_then(|row| file_rows.iter().position(|&r| r == row));
        let next = match current {
            Some(pos) => pos.saturating_add_signed(delta).min(file_rows.len() - 1),
            None if delta < 0 => file_rows.len() - 1,
            None => 0,
        };
        let Row::File { file, .. } = self.rows.rows()[file_rows[next]] else {
            return Reduction::default();
        };
        let key = self.files()[file as usize].key;
        self.select(key, SelectionOrigin::Sidebar)
    }

    fn reconcile_selection(&mut self) -> Reduction {
        let files = self.files();
        if files.is_empty() {
            let had = self.selection.take().is_some();
            return if had {
                Reduction::changed().intent(Intent::SelectFile(None))
            } else {
                Reduction::default()
            };
        }
        let by_path = |path: &str| {
            files.iter().find(|f| {
                f.paths.display_path() == path || f.paths.old_path().is_some_and(|old| old == path)
            })
        };
        let startup = self.startup.as_ref();
        let preferred = startup
            .and_then(|s| s.file_path.as_deref())
            .and_then(by_path)
            .or_else(|| {
                startup
                    .and_then(|s| s.file_index)
                    .and_then(|i| files.get(i))
            });
        let current = self.selection.as_ref().and_then(|s| {
            files
                .iter()
                .find(|f| f.key == s.key)
                .or_else(|| by_path(&s.path))
        });
        let target = preferred.or(current).unwrap_or(&files[0]);
        let key = target.key;
        let unchanged = self.selection.as_ref().is_some_and(|s| s.key == key)
            && preferred.is_none()
            && current.is_some();
        let path = target.paths.display_path().into_owned();
        if let Some(plan) = &mut self.startup {
            plan.file_index = None;
            plan.file_path = None;
        }
        if unchanged {
            if let Some(selection) = &mut self.selection {
                selection.path = path;
            }
            self.reveal_selected();
            return Reduction::changed();
        }
        self.select(key, SelectionOrigin::Command)
    }

    fn compare_intent(&mut self, cx: &SharedContext) -> Reduction {
        let Some(repo_path) = self.repo_path.clone() else {
            return Reduction::default().intent(Intent::Toast(ToastUpdate::error(
                "Open a repository before starting a compare.",
            )));
        };
        if !self.compare.working_copy && !self.compare.refs_valid() {
            return Reduction::default().intent(Intent::Toast(ToastUpdate::error(
                "Provide the required refs for the selected mode.",
            )));
        }
        self.history.commits = None;
        self.compare_requested = true;
        self.history.range = None;
        let mut reduction = Reduction::changed().intent(Intent::RequestCompare(CompareIntent {
            repo_path: repo_path.clone(),
            request: self.compare.request(),
        }));
        if let Some(intent) = self.persist_compare(&repo_path, cx) {
            reduction = reduction.intent(intent);
        }
        if self.tab == SidebarTab::Commits {
            reduction.merge(self.load_history());
        }
        reduction
    }

    fn persist_compare(&self, repo_path: &Path, cx: &SharedContext) -> Option<Intent> {
        if self.compare.working_copy {
            return None;
        }
        let compare = PersistedCompare {
            repo_path: Some(repo_path.to_path_buf()),
            left_ref: self.compare.left.clone(),
            right_ref: self.compare.right.clone(),
            mode: self.compare.mode,
            layout: self.compare.layout,
            renderer: self.compare.renderer,
        };
        let current = &cx.settings.settings;
        if current.last_compare.as_ref() == Some(&compare) {
            return None;
        }
        let settings = Settings {
            last_compare: Some(compare),
            ..current.clone()
        };
        Some(Intent::SettingsChanged(Rc::new(SettingsSnapshot {
            revision: cx.settings.revision + 1,
            settings,
        })))
    }

    fn history_range(&self) -> Option<(String, String)> {
        if self.compare.working_copy || self.history.pre_drill.is_some() {
            return None;
        }
        match self.compare.mode {
            CompareMode::TwoDot | CompareMode::ThreeDot if self.compare.refs_valid() => {
                let right = if is_working_copy_ref(&self.compare.right) {
                    "HEAD".to_owned()
                } else {
                    self.compare.right.clone()
                };
                Some((self.compare.left.clone(), right))
            }
            _ => None,
        }
    }

    fn load_history(&mut self) -> Reduction {
        let (Some(repo_path), Some(range)) = (self.repo_path.clone(), self.history_range()) else {
            return Reduction::default();
        };
        if self.history.range.as_ref() == Some(&range)
            && (self.history.loading || self.history.commits.is_some())
        {
            return Reduction::default();
        }
        self.history.generation += 1;
        self.history.range = Some(range.clone());
        self.history.loading = true;
        self.history.error = None;
        Reduction::changed().effect(NavigationEffect::LoadHistory(HistoryRequest {
            generation: CompareGeneration(self.history.generation),
            repo_path,
            left_ref: range.0,
            right_ref: range.1,
        }))
    }

    pub fn history_commits(&self) -> Option<&[VcsChange]> {
        if self.history_range().is_some() {
            return self.history.commits.as_deref();
        }
        self.repository
            .as_ref()
            .map(|r| r.snapshot.changes.as_slice())
    }

    fn startup(&mut self, startup: &AppStartup) -> Reduction {
        let options = &startup.options;
        let args = &options.args;
        let settings = &startup.settings;
        let persisted = settings
            .last_compare
            .as_ref()
            .filter(|c| args.repo.is_some() && c.repo_path.as_ref() == args.repo.as_ref());
        self.compare = CompareSelection {
            left: args
                .left
                .clone()
                .or_else(|| persisted.map(|c| c.left_ref.clone()))
                .unwrap_or_default(),
            right: args
                .right
                .clone()
                .or_else(|| persisted.map(|c| c.right_ref.clone()))
                .unwrap_or_default(),
            mode: args
                .compare_mode
                .or(persisted.map(|c| c.mode))
                .unwrap_or_default(),
            layout: args
                .layout
                .or(persisted.map(|c| c.layout))
                .unwrap_or(settings.viewport.layout),
            renderer: args
                .renderer
                .or(persisted.map(|c| c.renderer))
                .unwrap_or_default(),
            working_copy: false,
        };
        let auto =
            options.wants_compare(self.compare.mode, &self.compare.left, &self.compare.right);
        let pr_pending = args.open_pr.is_some();
        let bootstrap = args.repo.is_some()
            && !pr_pending
            && auto
            && (args.left.is_some() || args.right.is_some() || args.compare_mode.is_some());
        self.startup = Some(StartupPlan {
            auto_compare: auto && !bootstrap && !pr_pending,
            bootstrap_compare: bootstrap,
            pr_pending,
            file_index: args.file_index,
            file_path: args.file_path.clone(),
        });
        let Some(repo) = args.repo.clone() else {
            return Reduction::default();
        };
        Reduction::changed().intent(Intent::ActivateWorkspace {
            path: normalize_repository_path(repo),
        })
    }

    fn activate(&mut self, path: &Path, cx: &SharedContext) -> Reduction {
        let startup_pending = self.startup.as_ref().is_some_and(|s| {
            s.bootstrap_compare
                || s.auto_compare
                || s.pr_pending
                || s.file_path.is_some()
                || s.file_index.is_some()
        });
        if !startup_pending {
            let layout = self.compare.layout;
            let renderer = self.compare.renderer;
            self.compare = CompareSelection {
                layout,
                renderer,
                ..CompareSelection::default()
            };
            self.startup = None;
        }
        self.repo_path = Some(path.to_path_buf());
        self.status = RepoStatus::Opening;
        self.repository = None;
        self.manifest = None;
        self.selection = None;
        self.rows.invalidate();
        self.history = HistoryState::default();
        self.selected_operation = None;
        self.expanded.clear();
        self.expanded_revision += 1;
        self.viewed.clear();
        self.scroll_px = 0.0;
        self.resolved = [None, None];
        self.compare_requested = false;
        self.filter.set_text("");
        let mut reduction = Reduction::changed()
            .effect(NavigationEffect::SyncRepository(SyncRepositoryRequest {
                path: path.to_path_buf(),
                reason: RepositorySyncReason::Open,
                progress: None,
            }))
            .effect(NavigationEffect::WatchRepository {
                path: Some(path.to_path_buf()),
            });
        if self.startup.as_ref().is_some_and(|s| s.bootstrap_compare) {
            if let Some(plan) = &mut self.startup {
                plan.bootstrap_compare = false;
            }
            reduction.merge(self.compare_intent(cx));
        }
        reduction
    }

    fn snapshot_ready(
        &mut self,
        revision: RepositoryRevision,
        snapshot: Arc<RepositorySnapshot>,
        cx: &SharedContext,
    ) -> Reduction {
        if self.repo_path.is_none() {
            return Reduction::default();
        }
        if self
            .repository
            .as_ref()
            .is_some_and(|current| current.revision >= revision)
        {
            return Reduction::default();
        }
        self.repo_path = Some(snapshot.path.clone());
        self.status = RepoStatus::Ready;
        let context = Rc::new(RepositoryContext {
            revision,
            snapshot: snapshot.clone(),
        });
        self.repository = Some(context.clone());
        let mut reduction = Reduction::changed().intent(Intent::RepositoryChanged(context));
        match snapshot.reason {
            RepositorySyncReason::Open => reduction.merge(self.after_open(&snapshot, cx)),
            RepositorySyncReason::Dirty | RepositorySyncReason::Rescan => {
                let refresh = match snapshot.change_kind {
                    _ if snapshot.reason == RepositorySyncReason::Rescan => true,
                    Some(RepositoryChangeKind::Metadata | RepositoryChangeKind::Both) => {
                        self.compare.working_copy || self.compare.refs_valid()
                    }
                    Some(RepositoryChangeKind::Worktree) => {
                        self.compare.working_copy || is_working_copy_ref(&self.compare.right)
                    }
                    None => false,
                };
                if refresh && self.manifest.is_some() {
                    reduction.merge(self.compare_intent(cx));
                }
            }
        }
        reduction
    }

    fn after_open(&mut self, snapshot: &RepositorySnapshot, cx: &SharedContext) -> Reduction {
        let plan = self.startup.clone().unwrap_or_default();
        if let Some(startup) = &mut self.startup {
            startup.auto_compare = false;
            startup.pr_pending = false;
        }
        if plan.pr_pending {
            return Reduction::default();
        }
        if plan.auto_compare {
            return self.compare_intent(cx);
        }
        if self.compare_requested {
            return Reduction::default();
        }
        if let Some(persisted) = cx.settings.settings.last_compare.as_ref().filter(|c| {
            c.repo_path.as_deref() == Some(snapshot.path.as_path())
                || c.repo_path.as_ref() == self.repo_path.as_ref()
        }) {
            let candidate = CompareSelection {
                left: persisted.left_ref.clone(),
                right: persisted.right_ref.clone(),
                mode: persisted.mode,
                layout: persisted.layout,
                renderer: persisted.renderer,
                working_copy: false,
            };
            if candidate.refs_valid() {
                self.compare = candidate;
                return self.compare_intent(cx);
            }
        }
        self.compare.working_copy = true;
        self.compare_intent(cx)
    }

    fn publish_manifest(&mut self, manifest: &Rc<CompareManifestSnapshot>) -> Reduction {
        if self
            .manifest
            .as_ref()
            .is_some_and(|m| Rc::ptr_eq(m, manifest))
        {
            return Reduction::default();
        }
        let generation_changed = self
            .manifest
            .as_ref()
            .is_none_or(|m| m.generation != manifest.generation);
        self.manifest = Some(manifest.clone());
        self.rows.invalidate();
        if generation_changed {
            self.scroll_px = 0.0;
            self.marked.clear();
            self.anchor = None;
            if self.history.pre_drill.is_none() {
                self.resolved = [
                    non_empty(&manifest.resolved_left),
                    non_empty(&manifest.resolved_right),
                ];
            }
        }
        self.reconcile_selection()
    }

    fn set_tab(&mut self, tab: SidebarTab) -> Reduction {
        if tab == SidebarTab::OperationLog && !self.operation_log_available() {
            return Reduction::default();
        }
        self.tab = tab;
        self.filter.set_text("");
        self.rows.invalidate();
        let mut reduction = Reduction::changed();
        if tab == SidebarTab::Commits {
            reduction.merge(self.load_history());
        }
        reduction
    }

    fn drill_into(&mut self, revision: String) -> Reduction {
        if self.history.pre_drill.is_none() {
            self.history.pre_drill = Some(self.compare.clone());
        }
        self.history.selected = Some(revision.clone());
        self.compare.left = revision;
        self.compare.right.clear();
        self.compare.mode = CompareMode::SingleCommit;
        self.compare.working_copy = false;
        let Some(repo_path) = self.repo_path.clone() else {
            return Reduction::default();
        };
        Reduction::changed().intent(Intent::RequestCompare(CompareIntent {
            repo_path,
            request: self.compare.request(),
        }))
    }

    fn restore_drill(&mut self, cx: &SharedContext) -> Reduction {
        let Some(previous) = self.history.pre_drill.take() else {
            return Reduction::default();
        };
        self.history.selected = None;
        self.compare = previous;
        self.compare_intent(cx)
    }

    fn sync(&self, reason: RepositorySyncReason) -> Reduction {
        match &self.repo_path {
            Some(path) => Reduction::default().effect(NavigationEffect::SyncRepository(
                SyncRepositoryRequest {
                    path: path.clone(),
                    reason,
                    progress: None,
                },
            )),
            None => Reduction::default().intent(Intent::Toast(ToastUpdate::error(
                "Open a repository before refreshing.",
            ))),
        }
    }
}

fn field_index(field: CompareField) -> usize {
    match field {
        CompareField::Left => 0,
        CompareField::Right => 1,
    }
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn is_working_copy_ref(reference: &str) -> bool {
    matches!(reference, "" | "@" | "@workdir" | "WORKTREE" | "worktree")
}

pub fn normalize_repository_path(path: PathBuf) -> PathBuf {
    if path
        .file_name()
        .is_some_and(|name| name == ".git" || name == ".jj")
        && let Some(parent) = path.parent()
    {
        return parent.to_path_buf();
    }
    path
}

impl Controller for NavigationController {
    type Action = NavigationAction;
    type Event = NavigationEvent;

    fn reduce_action(&mut self, action: Self::Action, cx: &SharedContext) -> Reduction {
        let reduction = self.action(action, cx);
        self.queue_visible_stats();
        self.queue_visible_avatars();
        reduction
    }

    fn reduce_event(&mut self, event: Self::Event, cx: &SharedContext) -> Reduction {
        let reduction = self.event(event, cx);
        self.queue_visible_stats();
        self.queue_visible_avatars();
        reduction
    }

    fn apply_intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        if matches!(intent, Intent::AccountChanged) {
            self.avatars_requested.clear();
        }
        let reduction = self.intent(intent, cx);
        self.queue_visible_stats();
        self.queue_visible_avatars();
        reduction
    }
}

impl NavigationController {
    fn action(&mut self, action: NavigationAction, cx: &SharedContext) -> Reduction {
        use NavigationAction as A;
        match action {
            A::OpenRepository(path) => Reduction::changed().intent(Intent::ActivateWorkspace {
                path: normalize_repository_path(path),
            }),
            A::OpenRepositoryDialog => {
                Reduction::default().effect(Effect::Shell(ShellEffect::OpenRepositoryDialog))
            }
            A::ShowWorkingTree => {
                self.history.pre_drill = None;
                self.history.selected = None;
                self.compare.working_copy = true;
                self.compare_intent(cx)
            }
            A::NewTextCompare => Reduction::default().intent(Intent::OpenTextCompare),
            A::RefreshRepository => {
                if self.manifest.is_some() {
                    let mut reduction = self.compare_intent(cx);
                    reduction.merge(self.sync(RepositorySyncReason::Rescan));
                    reduction
                } else {
                    self.sync(RepositorySyncReason::Rescan)
                }
            }
            A::SetRef { field, value } => {
                self.history.pre_drill = None;
                self.history.selected = None;
                self.compare.working_copy = false;
                match field {
                    CompareField::Left => self.compare.left = value,
                    CompareField::Right => self.compare.right = value,
                }
                self.resolved[field_index(field)] = None;
                self.compare.auto_select_mode();
                Reduction::changed()
            }
            A::ChooseRef(side) => {
                let (left, right) = self.compare_labels();
                let request = RefPickerRequest {
                    side,
                    left,
                    right,
                    working_copy: self.compare.working_copy,
                    mode: self.compare.mode,
                    renderer: self.compare.renderer,
                };
                self.outbox
                    .borrow_mut()
                    .push(CommandsAction::ShowRefPicker(Box::new(request)).into());
                Reduction::default()
            }
            A::SwapRefs => {
                if self.compare.left.trim().is_empty() || self.compare.right.trim().is_empty() {
                    return Reduction::default();
                }
                std::mem::swap(&mut self.compare.left, &mut self.compare.right);
                self.resolved.swap(0, 1);
                self.history.pre_drill = None;
                if self.repo_path.is_some() && self.compare.refs_valid() {
                    self.compare.working_copy = false;
                    return self.compare_intent(cx);
                }
                Reduction::changed()
            }
            A::ResolveRef { field, query } => match self.repo_path.clone() {
                Some(repo_path) => {
                    let request = self.next_job();
                    Reduction::default().effect(NavigationEffect::ResolveRef(ResolveRefRequest {
                        request,
                        repo_path,
                        field,
                        query,
                    }))
                }
                None => Reduction::default(),
            },
            A::SetCompareMode(mode) => {
                self.compare.mode = mode;
                self.compare.working_copy = false;
                if self.repo_path.is_some() && self.compare.refs_valid() {
                    return self.compare_intent(cx);
                }
                Reduction::changed()
            }
            A::CycleCompareMode => {
                self.compare.mode = match self.compare.mode {
                    CompareMode::ThreeDot => CompareMode::TwoDot,
                    CompareMode::TwoDot => CompareMode::SingleCommit,
                    CompareMode::SingleCommit => CompareMode::ThreeDot,
                };
                Reduction::changed()
            }
            A::ApplyComparePreset(preset) => match preset.split_once("..") {
                Some((left, right)) => {
                    let (right, mode) = match right.strip_prefix('.') {
                        Some(right) => (right, CompareMode::ThreeDot),
                        None => (right, CompareMode::TwoDot),
                    };
                    self.compare.left = left.to_owned();
                    self.compare.right = right.to_owned();
                    self.compare.mode = mode;
                    self.compare.working_copy = false;
                    self.compare_intent(cx)
                }
                None => {
                    self.compare.left = preset;
                    self.compare.right.clear();
                    self.compare.mode = CompareMode::SingleCommit;
                    self.compare.working_copy = false;
                    self.compare_intent(cx)
                }
            },
            A::StartCompare => self.compare_intent(cx),
            A::CancelCompare => Reduction::default(),
            A::SelectFile(selected) => self.select(selected.key, selected.origin),
            A::SelectFilePath(path) => {
                let key = self
                    .files()
                    .iter()
                    .find(|f| f.paths.display_path() == path.as_str())
                    .map(|f| f.key);
                match key {
                    Some(key) => self.select(key, SelectionOrigin::Command),
                    None => {
                        let plan = self.startup.get_or_insert_with(StartupPlan::default);
                        plan.file_path = Some(path);
                        Reduction::default()
                    }
                }
            }
            A::SelectNextFile => self.shift_selection(1),
            A::SelectPreviousFile => self.shift_selection(-1),
            A::ToggleFolder(path) => {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
                self.expanded_revision += 1;
                self.clamp_scroll();
                Reduction::changed()
            }
            A::ExpandAllFolders => {
                let mut expanded = std::mem::take(&mut self.expanded);
                for file in self.files() {
                    rows::folder_prefixes(&file.paths.display_path(), &mut expanded);
                }
                self.expanded = expanded;
                self.expanded_revision += 1;
                Reduction::changed()
            }
            A::CollapseAllFolders => {
                self.expanded.clear();
                self.expanded_revision += 1;
                self.clamp_scroll();
                Reduction::changed()
            }
            A::ToggleFileViewed(key) => {
                let Some(index) = self.file_index(key) else {
                    return Reduction::default();
                };
                let path = self.files()[index].paths.display_path().into_owned();
                if !self.viewed.remove(&path) {
                    self.viewed.insert(path);
                }
                Reduction::changed()
            }
            A::SetFilter(query) => {
                self.filter.set_text(query);
                self.scroll_px = 0.0;
                Reduction::changed()
            }
            A::ClearFilter => {
                if self.filter.text().is_empty() {
                    return Reduction::default();
                }
                self.filter.set_text("");
                self.scroll_px = 0.0;
                self.reveal_selected();
                Reduction::changed()
            }
            A::EscapeFilter => {
                self.filter.set_text("");
                self.scroll_px = 0.0;
                self.reveal_selected();
                Reduction::changed().effect(ShellEffect::Focus(Some(LogicalFocus::Sidebar)))
            }
            A::ScrollFiles(lines) => {
                self.scroll_px += lines as f32 * self.metrics.row_height;
                self.clamp_scroll();
                Reduction::changed()
            }
            A::ScrollFilesTo(px) => {
                self.scroll_px = px as f32;
                self.clamp_scroll();
                Reduction::changed()
            }
            A::ToggleSidebarMode => {
                self.mode = match self.mode {
                    SidebarMode::List => SidebarMode::Tree,
                    SidebarMode::Tree => SidebarMode::List,
                };
                self.scroll_px = 0.0;
                self.reveal_selected();
                Reduction::changed()
            }
            A::ToggleSidebar => {
                self.visible = !self.visible;
                Reduction::changed()
            }
            A::SetSidebarTab(tab) => self.set_tab(tab),
            A::SelectHistoryCommit(revision) => self.drill_into(revision),
            A::ClearHistoryCommit => self.restore_drill(cx),
            A::CompareWithWorkingTree(revision) => {
                let workdir = match &self.repository {
                    Some(repo) if repo.snapshot.location.kind != VcsKind::GIT => "@",
                    _ => crate::core::vcs::git::WORKDIR_REF,
                };
                self.history.pre_drill = None;
                self.history.selected = None;
                self.compare.working_copy = false;
                self.compare.left = revision;
                self.compare.right = workdir.to_owned();
                self.compare.mode = CompareMode::TwoDot;
                self.resolved = [None, None];
                self.compare_intent(cx)
            }
            A::ScrollHistory(lines) => {
                let rows = self.history_commits().map_or(0, <[VcsChange]>::len) as f32;
                let max = (rows * self.metrics.history_row - self.metrics.viewport).max(0.0);
                self.history.scroll_px = (self.history.scroll_px
                    + lines as f32 * self.metrics.row_height)
                    .clamp(0.0, max);
                Reduction::changed()
            }
            A::SelectOperation(id) => {
                self.selected_operation = Some(id);
                Reduction::changed()
            }
        }
    }

    fn event(&mut self, event: NavigationEvent, cx: &SharedContext) -> Reduction {
        match event {
            NavigationEvent::SnapshotReady { revision, snapshot } => {
                self.snapshot_ready(revision, snapshot, cx)
            }
            NavigationEvent::SnapshotFailed {
                path,
                reason,
                message,
            } => {
                if reason == RepositorySyncReason::Open {
                    self.status = RepoStatus::Failed(message.clone());
                }
                tracing::warn!(path = %path.display(), ?reason, %message, "repository sync failed");
                Reduction::changed().intent(Intent::Toast(ToastUpdate::error(message)))
            }
            NavigationEvent::HistoryReady {
                generation,
                commits,
            } => {
                if generation.0 != self.history.generation {
                    return Reduction::default();
                }
                self.history.loading = false;
                self.history.commits = Some(commits);
                Reduction::changed()
            }
            NavigationEvent::HistoryFailed {
                generation,
                message,
            } => {
                if generation.0 != self.history.generation {
                    return Reduction::default();
                }
                self.history.loading = false;
                self.history.error = Some(message);
                Reduction::changed()
            }
            NavigationEvent::RefResolved {
                request,
                field,
                short_oid,
                summary,
                ..
            } => {
                if request.0 != self.next_job {
                    return Reduction::default();
                }
                self.resolved[field_index(field)] = Some(if summary.is_empty() {
                    short_oid
                } else {
                    format!("{short_oid} {summary}")
                });
                Reduction::changed()
            }
            NavigationEvent::RefResolveFailed { .. } => Reduction::default(),
        }
    }

    fn intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        match intent {
            Intent::Startup(startup) => self.startup(startup),
            Intent::ActivateWorkspace { path } => self.activate(path, cx),
            Intent::PublishManifest(manifest) => self.publish_manifest(manifest),
            Intent::SelectFile(Some(selected)) if selected.origin == SelectionOrigin::Viewport => {
                if self
                    .selection
                    .as_ref()
                    .is_some_and(|s| s.key == selected.key)
                {
                    return Reduction::default();
                }
                self.select(selected.key, SelectionOrigin::Viewport)
            }
            Intent::InvalidateRepository => self.sync(RepositorySyncReason::Dirty),
            Intent::SettingsChanged(settings) => match settings.settings.last_compare.as_ref() {
                Some(last) if last.renderer != self.compare.renderer => {
                    self.compare.renderer = last.renderer;
                    Reduction::changed()
                }
                _ => Reduction::default(),
            },
            _ => Reduction::default(),
        }
    }
}

impl crate::contracts::commands::CommandSource for NavigationController {}

impl TextHost for NavigationController {
    fn owns_text(&self, target: FocusId) -> bool {
        target == SIDEBAR_FILTER
    }

    fn edit_text(
        &mut self,
        _target: FocusId,
        command: TextEditCommand,
        cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        let outcome = self.filter.apply_at(command, cx.now_ms);
        if outcome.text_changed {
            self.scroll_px = 0.0;
            self.queue_visible_stats();
        }
        Some(outcome)
    }

    fn set_preedit(
        &mut self,
        _target: FocusId,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> bool {
        self.filter.set_preedit(text, cursor);
        true
    }

    fn set_text_value(
        &mut self,
        _target: FocusId,
        value: String,
        _cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        let text_changed = self.filter.text() != value;
        self.filter.set_text(value);
        if text_changed {
            self.scroll_px = 0.0;
            self.queue_visible_stats();
        }
        Some(TextEditOutcome {
            text_changed,
            selection_changed: true,
            ..TextEditOutcome::default()
        })
    }
}
