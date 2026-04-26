mod ai;
mod app;
mod compare;
mod editor;
mod file_list;
mod github;
mod overlay;
mod repository;
mod settings;
mod syntax;
mod text_edit;
mod update;
mod workspace;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use halogen::Store;
use halogen::reactive::{Signal, SignalStore};

use crate::actions::Action;
use crate::core::compare::{CompareMode, CompareOutput, CompareSpec, LayoutMode, RendererKind};
use crate::core::frecency::FrecencyStore;
use crate::core::syntax::Highlighter;
use crate::core::syntax::annotator::{SyntaxLineTokens, SyntaxRowWindow};
use crate::core::text::TokenBuffer;
use crate::core::update::{AvailableUpdate, StagedUpdate};
use crate::core::vcs::git::patch;
use crate::core::vcs::git::{
    BranchInfo, CommitInfo, StatusItem, StatusOperation, StatusScope, TagInfo,
};
use crate::core::vcs::github::{
    CreatePullRequestReviewComment, DeviceFlowState, GitHubReviewSide, GitHubUser, PullRequestInfo,
    PullRequestReviewComment,
};
use crate::editor::Editor;
use crate::effects::{
    AiEffect, BatchStatusOperationRequest, CommitRequest, CompareEffect, CompareFileRequest,
    CompareFileStatsItem, CompareFileStatsRequest, CompareHistoryRequest, CompareRequest,
    CompareStatsRequest, Effect, FetchRemoteRequest, GitHubEffect, LoadFileSyntaxRequest,
    PatchOperationRequest, PullFfRequest, PushRequest, RepositoryEffect, SettingsEffect,
    StatusDiffRequest, StatusOperationRequest, SyntaxEffect, Task, UiEffect, UpdateEffect,
};
use crate::events::{
    AppEvent, CompareFileFinished, CompareFileStat, CompareFileStatsReady, CompareFinished,
    CompareHistoryReady, CompareStatsReady, FileSyntaxReady, RepositoryChangeKind,
    RepositorySnapshot, RepositorySyncReason, StatusDiffFinished,
};
use crate::platform::persistence::{PersistedCompare, Settings};
use crate::platform::secrets::AiKeyKind;
use crate::platform::startup::StartupOptions;
use crate::ui::design::{Sp, Sz};
use crate::ui::editor::render_doc::{CarbonStyleOverlays, RenderDoc, build_render_doc_from_carbon};
use crate::ui::editor::state::{EditorState, EditorStateStore, SearchMatch};
use crate::ui::icons::lucide;
use crate::ui::theme::ThemeMode;

const MAX_VISIBLE_TOASTS: usize = 5;
const TOAST_LIFETIME_MS: u64 = 5_000;
const TOAST_ANIM_MS: u64 = 150;
const CURSOR_BLINK_INTERVAL_MS: u64 = 530;
const LARGE_COMPARE_FILE_LINES: i32 = 1_500;
const COMPARE_STATS_CHUNK_SIZE: usize = 64;
const SYNTAX_INITIAL_ROWS: usize = 200;
const SYNTAX_OVERSCAN_ROWS: usize = 160;

fn build_pr_palette_entry(
    cache: &HashMap<PrKey, PrCacheEntry>,
    key: &PrKey,
    has_repo: bool,
) -> PaletteEntry {
    let (owner, repo, number) = key;
    let fallback_label = format!("#{number} in {owner}/{repo}");
    let entry = cache.get(key);
    let (label, rhs, detail, disabled) = match entry.map(|e| (&e.meta, &e.diff)) {
        None | Some((PrPeekMeta::Loading, _)) => (
            fallback_label,
            Some("Resolving\u{2026}".to_owned()),
            if has_repo {
                "Fetching PR metadata".to_owned()
            } else {
                "Open a repo to view this diff".to_owned()
            },
            false,
        ),
        Some((PrPeekMeta::Ready(info), diff)) => {
            let label = format!("#{} {}", info.number, info.title);
            let rhs = format!(
                "{} \u{00B7} +{} \u{2212}{} \u{00B7} @{}",
                info.state, info.additions, info.deletions, info.author_login
            );
            let detail = match diff {
                PrPeekDiff::Ready { .. } => "Ready \u{2014} press Enter to open".to_owned(),
                PrPeekDiff::Loading => "Preparing diff\u{2026}".to_owned(),
                PrPeekDiff::Failed(msg) => format!("Diff load failed: {msg}"),
                PrPeekDiff::Idle => {
                    if has_repo {
                        "Queued".to_owned()
                    } else {
                        "Open a repo to view this diff".to_owned()
                    }
                }
            };
            let disabled = !has_repo;
            (label, Some(rhs), detail, disabled)
        }
        Some((PrPeekMeta::Failed(msg), _)) => {
            (fallback_label, Some("error".to_owned()), msg.clone(), true)
        }
    };
    PaletteEntry {
        label,
        detail,
        kind: PaletteEntryKind::PullRequest(key.clone()),
        highlights: Vec::new(),
        rhs,
        disabled,
    }
}

/// Request a fixed-size avatar from GitHub by rewriting (or appending) the `s=` query
/// parameter. Returns `None` if the input URL is empty.
pub(crate) fn avatar_url_sized(base: &str, size: u32) -> Option<String> {
    let base = base.trim();
    if base.is_empty() {
        return None;
    }
    let (path, query) = match base.split_once('?') {
        Some((p, q)) => (p, q),
        None => (base, ""),
    };
    let mut parts: Vec<String> = query
        .split('&')
        .filter(|part| !part.is_empty() && !part.starts_with("s="))
        .map(|part| part.to_owned())
        .collect();
    parts.push(format!("s={size}"));
    Some(format!("{path}?{}", parts.join("&")))
}

/// Deterministic cache key for an avatar URL so the GPU texture cache dedupes it.
pub(crate) fn avatar_cache_key(url: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    "avatar".hash(&mut h);
    url.hash(&mut h);
    h.finish()
}

const DEFAULT_UI_SCALE_PCT: u16 = 100;
const MIN_UI_SCALE_PCT: u16 = 70;
const MAX_UI_SCALE_PCT: u16 = 180;
const UI_SCALE_STEP_PCT: u16 = 10;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceMode {
    #[default]
    Empty,
    Loading,
    Ready,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AppView {
    #[default]
    Workspace,
    Settings,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SettingsSection {
    #[default]
    Appearance,
    Editor,
    Behavior,
    Clankers,
    About,
}

impl SettingsSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Editor => "Editor",
            Self::Behavior => "Behavior",
            Self::Clankers => "Clankers",
            Self::About => "About",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Appearance => lucide::SUN,
            Self::Editor => lucide::FILE_CODE,
            Self::Behavior => lucide::SETTINGS,
            Self::Clankers => lucide::SPARKLES,
            Self::About => lucide::INFO,
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Appearance,
        Self::Editor,
        Self::Behavior,
        Self::Clankers,
        Self::About,
    ];
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceSource {
    #[default]
    None,
    Status,
    Compare,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AsyncStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompareField {
    #[default]
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    WorkspacePrimaryButton,
    TitleBar,
    ThemeToggle,
    FileList,
    Editor,
    PickerInput,
    PickerList,
    CommandPaletteInput,
    CommandPaletteList,
    AuthPrimaryAction,
    SidebarSearch,
    SearchInput,
    CommitEditor,
    ReviewCommentEditor,
    SettingsOpenAiKey,
    SettingsAnthropicKey,
    SettingsSteeringPrompt,
}

impl FocusTarget {
    pub fn is_text_field(self) -> bool {
        matches!(
            self,
            Self::PickerInput
                | Self::CommandPaletteInput
                | Self::SidebarSearch
                | Self::SearchInput
                | Self::CommitEditor
                | Self::ReviewCommentEditor
                | Self::SettingsOpenAiKey
                | Self::SettingsAnthropicKey
                | Self::SettingsSteeringPrompt
        )
    }
}

// Focus is stored directly as a Signal on AppState — no wrapper struct.

/// Cursor/selection state for the currently focused text field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct TextEditState {
    /// Byte offset of the caret.
    pub cursor: usize,
    /// Byte offset of the selection anchor.  Equal to `cursor` when nothing is selected.
    pub anchor: usize,
    /// Timestamp (clock_ms) when the cursor last moved — used to reset blink phase.
    pub cursor_moved_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Store)]
pub struct CompareState {
    pub repo_path: Option<PathBuf>,
    pub left_ref: String,
    pub right_ref: String,
    pub mode: CompareMode,
    pub layout: LayoutMode,
    pub renderer: RendererKind,
    pub resolved_left: Option<String>,
    pub resolved_right: Option<String>,
}

impl Default for CompareState {
    fn default() -> Self {
        Self {
            repo_path: None,
            left_ref: String::new(),
            right_ref: String::new(),
            mode: CompareMode::default(),
            layout: LayoutMode::default(),
            renderer: RendererKind::default(),
            resolved_left: None,
            resolved_right: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct RepositoryState {
    pub status: AsyncStatus,
    pub branches: Vec<BranchInfo>,
    pub tags: Vec<TagInfo>,
    pub commits: Vec<CommitInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileListEntry {
    pub path: String,
    pub status: String,
    pub additions: i32,
    pub deletions: i32,
    pub is_binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveFileLoading {
    pub index: usize,
    pub path: String,
}

pub use crate::core::compare::ComparePhase;

/// What the progress panel is about. Drives chip rendering: compare
/// shows a left⇄right ref pair, repo-open shows a single folder chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadingSubject {
    Compare {
        left_label: String,
        right_label: String,
    },
    RepoOpen {
        name: String,
    },
}

/// Transient progress state for a long-running workspace operation
/// (compare or repo open). Present iff something is in flight and the
/// reveal delay has either elapsed or was set to zero. Cleared when the
/// operation lands or the user cancels.
///
/// `reveal_at_ms` implements the "don't flash on fast ops" rule: the
/// panel is only rendered once `clock_ms >= reveal_at_ms`. For fresh
/// bootstraps this equals `started_at_ms` (show immediately); for
/// re-opens / re-compares over an already-loaded workspace it is pushed
/// out by `COMPARE_REVEAL_DELAY_MS` so a sub-half-second op never flashes
/// loading UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareProgress {
    pub generation: u64,
    pub phase: ComparePhase,
    pub subject: LoadingSubject,
    pub started_at_ms: u64,
    pub reveal_at_ms: u64,
    /// Total file count — first known from a backend `LoadingFiles`
    /// emission, re-confirmed by `CompareFinished`. Unused for RepoOpen.
    pub file_count_total: Option<u32>,
    /// Files read so far during `LoadingFiles`. Zero before, frozen
    /// after.
    pub files_loaded: u32,
}

/// Delay between kicking off an op and revealing the loading UI —
/// fast ops under this threshold show no loading flash at all.
pub const COMPARE_REVEAL_DELAY_MS: u64 = 500;

#[derive(Debug, Clone)]
pub struct PreparedActiveFile {
    pub carbon_file: carbon::FileDiff,
    pub carbon_expansion: carbon::ExpansionState,
    pub carbon_overlays: CarbonStyleOverlays,
    pub render_doc: RenderDoc,
    pub token_buffer: TokenBuffer,
}

fn refs_for_status_scope(scope: StatusScope) -> (String, String) {
    use crate::core::vcs::git::{INDEX_REF, WORKDIR_REF};
    match scope {
        StatusScope::Staged => ("HEAD".to_owned(), INDEX_REF.to_owned()),
        StatusScope::Unstaged => (INDEX_REF.to_owned(), WORKDIR_REF.to_owned()),
        StatusScope::Untracked => (String::new(), WORKDIR_REF.to_owned()),
    }
}

fn apply_syntax_tokens_to_file(
    carbon_overlays: &mut CarbonStyleOverlays,
    token_buffer: &mut TokenBuffer,
    updates: &[SyntaxLineTokens],
) {
    for update in updates {
        if let (Some(side), Some(source_index)) = (update.side, update.source_index) {
            if update.tokens.is_empty() {
                continue;
            }
            let range = token_buffer.append(&update.tokens);
            carbon_overlays.insert_syntax(update.hunk_index as u32, side, source_index, range);
        }
    }
}

fn push_syntax_covered_window(windows: &mut Vec<SyntaxRowWindow>, window: SyntaxRowWindow) {
    if window.end <= window.start {
        return;
    }
    windows.push(window);
    windows.sort_by_key(|window| window.start);
    let mut merged: Vec<SyntaxRowWindow> = Vec::with_capacity(windows.len());
    for window in windows.drain(..) {
        if let Some(last) = merged.last_mut()
            && window.start <= last.end
        {
            last.end = last.end.max(window.end);
            continue;
        }
        merged.push(window);
    }
    *windows = merged;
}

fn hydrate_carbon_full_text(
    file: &mut carbon::FileDiff,
    old_lines: &[String],
    new_lines: &[String],
) {
    if !old_lines.is_empty() {
        file.old_text = Some(carbon::TextStore::from_text(lines_to_text(old_lines)));
    }
    if !new_lines.is_empty() {
        file.new_text = Some(carbon::TextStore::from_text(lines_to_text(new_lines)));
    }
    for block in &mut file.blocks {
        block.old.start = block.old_line_start.saturating_sub(1);
        block.new.start = block.new_line_start.saturating_sub(1);
    }
    file.is_partial = false;
}

fn lines_to_text(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut text =
        String::with_capacity(lines.iter().map(|line| line.len().saturating_add(1)).sum());
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text
}

fn i32_to_u32_nonnegative(value: i32) -> u32 {
    u32::try_from(value).unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct ActiveFile {
    pub index: usize,
    pub path: String,
    pub carbon_file: carbon::FileDiff,
    pub carbon_expansion: carbon::ExpansionState,
    pub carbon_overlays: CarbonStyleOverlays,
    pub render_doc: RenderDoc,
    pub base_carbon_file: carbon::FileDiff,
    pub token_buffer: TokenBuffer,
    pub left_ref: String,
    pub right_ref: String,
    pub file_line_count: Option<u32>,
    pub old_file_lines: Option<Arc<Vec<String>>>,
    pub file_lines: Option<Arc<Vec<String>>>,
    pub syntax_request_id: u64,
    pub syntax_pending: Option<SyntaxRowWindow>,
    pub syntax_covered: Vec<SyntaxRowWindow>,
}

pub(crate) fn prepare_active_file(
    file_index: usize,
    carbon_file: &carbon::FileDiff,
) -> PreparedActiveFile {
    let token_buffer = TokenBuffer::default();
    let carbon_overlays = CarbonStyleOverlays::default();

    let carbon_expansion = carbon::ExpansionState::default();
    let render_doc = build_render_doc_from_carbon(
        carbon_file,
        file_index,
        &carbon_expansion,
        &carbon_overlays,
        &token_buffer,
    );
    PreparedActiveFile {
        carbon_file: carbon_file.clone(),
        carbon_expansion,
        carbon_overlays,
        render_doc,
        token_buffer,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidebarWidthCache {
    pub compare_generation: u64,
    pub ui_scale_pct: u16,
    pub intrinsic_width_px: f32,
}

#[derive(Debug, Clone, Default, Store)]
pub struct WorkspaceState {
    pub source: WorkspaceSource,
    pub status: AsyncStatus,
    pub status_operation_pending: bool,
    pub compare_generation: u64,
    pub status_generation: u64,
    pub files: Vec<FileListEntry>,
    pub status_items: Vec<StatusItem>,
    pub selected_file_index: Option<usize>,
    pub selected_file_path: Option<String>,
    pub selected_status_scope: Option<StatusScope>,
    pub compare_output: Option<CompareOutput>,
    pub compare_total_stats: Option<(i32, i32)>,
    pub compare_total_stats_loading: bool,
    pub compare_stats_hydration_active: bool,
    pub active_file: Option<ActiveFile>,
    pub active_file_loading: Option<ActiveFileLoading>,
    pub raw_diff_len: usize,
    pub used_fallback: bool,
    pub fallback_message: String,
    pub sidebar_auto_width: Option<SidebarWidthCache>,
    pub range_commits: Vec<CommitInfo>,
    pub pre_drill_compare: Option<(String, String, CompareMode)>,
    pub expansions: HashMap<String, carbon::ExpansionState>,
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

impl AppState {
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
            .set(&self.store, cur.clamp(0.0, max));
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
        if self.workspace.source.get(&self.store) == WorkspaceSource::Compare
            && self.file_list.tab.get(&self.store) == SidebarTab::Files
            && self.file_list.mode.get(&self.store) == SidebarMode::TreeView
            && self.file_list.filter.with(&self.store, |s| s.is_empty())
        {
            let expanded_folders = self.file_list.expanded_folders.get(&self.store);
            return self.workspace.files.with(&self.store, |files| {
                crate::ui::components::file_tree_visible_row_count(
                    files.iter().map(|file| file.path.as_str()),
                    &expanded_folders,
                )
            });
        }

        if self.workspace.source.get(&self.store) == WorkspaceSource::Status
            && self.file_list.filter.with(&self.store, |s| s.is_empty())
        {
            self.workspace.files.with(&self.store, |f| f.len())
                + self
                    .workspace
                    .status_items
                    .with(&self.store, |s| status_section_count(s))
        } else {
            self.workspace.files.with(&self.store, |f| f.len())
        }
    }

    fn sidebar_row_index_for_file(&self, index: usize) -> usize {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status
            || !self.file_list.filter.with(&self.store, |s| s.is_empty())
        {
            return index;
        }
        index
            + self
                .workspace
                .status_items
                .with(&self.store, |s| status_section_count_before(s, index + 1))
    }

    fn compare_file_is_large(&self, index: usize) -> bool {
        if self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .and_then(|output| output.carbon.files.get(index))
                .is_some_and(|file| file.is_partial && file.hunks.is_empty())
        }) {
            return true;
        }

        self.workspace.files.with(&self.store, |files| {
            files.get(index).is_some_and(|file| {
                !file.is_binary
                    && file.additions.saturating_add(file.deletions) >= LARGE_COMPARE_FILE_LINES
            })
        })
    }

    fn build_active_file(
        &self,
        index: usize,
        path: String,
        prepared: PreparedActiveFile,
        left_ref: String,
        right_ref: String,
    ) -> ActiveFile {
        ActiveFile {
            index,
            path,
            carbon_file: prepared.carbon_file.clone(),
            carbon_expansion: prepared.carbon_expansion.clone(),
            carbon_overlays: prepared.carbon_overlays,
            render_doc: prepared.render_doc,
            base_carbon_file: prepared.carbon_file,
            token_buffer: prepared.token_buffer,
            left_ref,
            right_ref,
            file_line_count: None,
            old_file_lines: None,
            file_lines: None,
            syntax_request_id: 0,
            syntax_pending: None,
            syntax_covered: Vec::new(),
        }
    }

    fn install_compare_active_file(
        &mut self,
        index: usize,
        path: String,
        prepared: PreparedActiveFile,
    ) {
        let left_ref = self
            .compare
            .resolved_left
            .get(&self.store)
            .unwrap_or_else(|| self.compare.left_ref.get(&self.store));
        let right_ref = self
            .compare
            .resolved_right
            .get(&self.store)
            .unwrap_or_else(|| self.compare.right_ref.get(&self.store));
        let active_file =
            self.build_active_file(index, path.clone(), prepared, left_ref, right_ref);
        let stats = CompareFileStat {
            index,
            path: path.clone(),
            additions: u32_to_i32_saturating(active_file.carbon_file.additions),
            deletions: u32_to_i32_saturating(active_file.carbon_file.deletions),
        };

        self.workspace
            .selected_file_index
            .set(&self.store, Some(index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(path));
        self.workspace.selected_status_scope.set(&self.store, None);
        self.workspace.active_file_loading.set(&self.store, None);
        self.workspace
            .active_file
            .set(&self.store, Some(active_file));
        self.apply_compare_file_stats(&[stats]);
        // The first real file has landed — tear down the progress panel.
        // Subsequent file loads use the sidebar row spinner, not this.
        self.compare_progress.set(&self.store, None);
        self.editor_clear_document();
        self.editor
            .line_selection
            .update(&self.store, |ls| ls.clear());
        if self.editor.search.open.get(&self.store) {
            self.recompute_search_matches();
        }
        self.file_list.hovered_index.set(&self.store, Some(index));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayListState {
    pub scroll_top_px: u32,
    pub viewport_height_px: u32,
    pub row_height_px: u32,
    pub gap_px: u32,
}

impl Default for OverlayListState {
    fn default() -> Self {
        Self {
            scroll_top_px: 0,
            viewport_height_px: 0,
            row_height_px: 36,
            gap_px: 0,
        }
    }
}

impl OverlayListState {
    pub fn stride_px(&self) -> u32 {
        self.row_height_px + self.gap_px
    }

    pub fn total_content_height_px(&self, entry_count: usize) -> u32 {
        if entry_count == 0 {
            return 0;
        }
        self.stride_px()
            .saturating_mul(entry_count as u32)
            .saturating_sub(self.gap_px)
    }

    pub fn viewport_for_max_rows(&self, max_rows: usize, entry_count: usize) -> u32 {
        let visible = entry_count.min(max_rows);
        if visible == 0 {
            return 0;
        }
        self.stride_px()
            .saturating_mul(visible as u32)
            .saturating_sub(self.gap_px)
    }

    pub fn max_scroll_top_px(&self, entry_count: usize) -> u32 {
        self.total_content_height_px(entry_count)
            .saturating_sub(self.viewport_height_px)
    }

    pub fn clamp_scroll(&mut self, entry_count: usize) {
        self.scroll_top_px = self.scroll_top_px.min(self.max_scroll_top_px(entry_count));
    }

    pub fn scroll_px(&mut self, delta_px: i32, entry_count: usize) {
        self.scroll_top_px = apply_scroll_delta_px(
            self.scroll_top_px,
            delta_px,
            self.max_scroll_top_px(entry_count),
        );
    }

    pub fn reveal_index(&mut self, index: usize, entry_count: usize) {
        let stride = self.stride_px().max(1);
        let item_top = stride.saturating_mul(index as u32);
        let item_bottom = item_top.saturating_add(self.row_height_px);
        let viewport_bottom = self.scroll_top_px.saturating_add(self.viewport_height_px);

        if item_top < self.scroll_top_px {
            self.scroll_top_px = item_top;
        } else if item_bottom > viewport_bottom {
            self.scroll_top_px = item_bottom.saturating_sub(self.viewport_height_px);
        }

        self.clamp_scroll(entry_count);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PickerKind {
    #[default]
    Repository,
    LeftRef,
    RightRef,
    Theme,
}

pub trait PickerItem {
    fn label(&self) -> &str;
    fn detail(&self) -> Option<&str>;
    fn highlight_ranges(&self) -> &[(usize, usize)] {
        &[]
    }
    fn icon_svg(&self) -> Option<&'static str> {
        None
    }
    fn is_section_header(&self) -> bool {
        false
    }
    fn rhs(&self) -> Option<&str> {
        None
    }
    fn is_disabled(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerEntry {
    pub label: String,
    pub detail: String,
    pub value: String,
    pub highlights: Vec<(usize, usize)>,
    pub icon: Option<&'static str>,
    pub section_header: bool,
}

impl PickerItem for PickerEntry {
    fn label(&self) -> &str {
        &self.label
    }
    fn detail(&self) -> Option<&str> {
        Some(&self.detail)
    }
    fn highlight_ranges(&self) -> &[(usize, usize)] {
        &self.highlights
    }
    fn icon_svg(&self) -> Option<&'static str> {
        self.icon
    }
    fn is_section_header(&self) -> bool {
        self.section_header
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct PickerState {
    pub kind: PickerKind,
    pub query: String,
    pub entries: Vec<PickerEntry>,
    pub selected_index: usize,
    pub hovered_index: Option<usize>,
    pub list: OverlayListState,
    pub browse_path: Option<PathBuf>,
    pub ref_resolve_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteCommand {
    OpenRepoPicker,
    OpenGitHubAuthModal,
    FocusFileList,
    FocusViewport,
    ToggleWrap,
    ToggleThemeMode,
    ChangeTheme,
    SetLayout(LayoutMode),
    SetTheme(String),
    FetchOrigin,
    FetchAllRemotes,
    PushCurrentBranch,
    PushCurrentBranchForceWithLease,
    PullCurrentBranch,
    OpenSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEntryKind {
    Command(PaletteCommand),
    File(usize),
    Repo(PathBuf),
    Ref(CompareField, String),
    PullRequest(PrKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    pub label: String,
    pub detail: String,
    pub kind: PaletteEntryKind,
    pub highlights: Vec<(usize, usize)>,
    /// Extra right-aligned summary (e.g. "+12 −3 · open").
    pub rhs: Option<String>,
    /// Disables the entry when set; `detail` usually explains why.
    pub disabled: bool,
}

impl PickerItem for PaletteEntry {
    fn label(&self) -> &str {
        &self.label
    }
    fn detail(&self) -> Option<&str> {
        Some(&self.detail)
    }
    fn highlight_ranges(&self) -> &[(usize, usize)] {
        &self.highlights
    }
    fn rhs(&self) -> Option<&str> {
        self.rhs.as_deref()
    }
    fn is_disabled(&self) -> bool {
        self.disabled
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct CommandPaletteState {
    pub query: String,
    pub entries: Vec<PaletteEntry>,
    pub selected_index: usize,
    pub list: OverlayListState,
}

/// Ephemeral ref-picker overlay state. `active_field` tracks which chip the
/// search input currently drives; `original_*` snapshots the refs at the moment
/// the picker opened so we can revert cleanly on cancel/backdrop.
#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct RefPickerState {
    pub active_field: CompareField,
    pub original_left: String,
    pub original_right: String,
}

pub type PrKey = (String, String, i32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrPeekMeta {
    Loading,
    Ready(PullRequestInfo),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrPeekDiff {
    Idle,
    Loading,
    Ready {
        url: String,
        left_ref: String,
        right_ref: String,
        info: PullRequestInfo,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCacheEntry {
    pub meta: PrPeekMeta,
    pub diff: PrPeekDiff,
    pub last_peek_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrReviewCommentsEntry {
    pub status: AsyncStatus,
    pub comments: Vec<PullRequestReviewComment>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCommentDraft {
    pub key: PrKey,
    pub request: CreatePullRequestReviewComment,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewCommentComposerState {
    pub draft: Option<ReviewCommentDraft>,
    pub status: AsyncStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct PullRequestState {
    pub status: AsyncStatus,
    pub cache: HashMap<PrKey, PrCacheEntry>,
    pub pending_confirm: Option<PrKey>,
    pub active: Option<PrKey>,
    pub review_comments: HashMap<PrKey, PrReviewCommentsEntry>,
    pub review_composer: ReviewCommentComposerState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarBitmap {
    pub url: String,
    pub rgba: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub cache_key: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct GitHubAuthState {
    pub status: AsyncStatus,
    pub device_flow: Option<DeviceFlowState>,
    pub token_present: bool,
    pub user: Option<GitHubUser>,
    pub avatar: Option<AvatarBitmap>,
    pub avatar_fetching: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct GitHubState {
    pub client_id: String,
    #[store(flatten)]
    pub auth: GitHubAuthState,
    #[store(flatten)]
    pub pull_request: PullRequestState,
}

/// Overlays live as normal elements in the main tree with a z-index above the
/// viewport. Occluding the viewport is the overlay's own responsibility: modal
/// surfaces (pickers, auth, shortcuts) render a full-screen `overlay_scrim`
/// backdrop; anchored dropdowns (AccountMenu, CompareMenu) render a transparent
/// backdrop and let the viewport show through. Do NOT gate viewport rendering
/// on overlay presence — let z-index handle layering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlaySurface {
    RepoPicker,
    RefPicker,
    CommandPalette,
    GitHubAuthModal,
    KeyboardShortcuts,
    ThemePicker,
    CompareMenu,
    AccountMenu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayEntry {
    pub surface: OverlaySurface,
    pub focus_return: Option<FocusTarget>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct OverlayStackState {
    pub stack: Vec<OverlayEntry>,
    #[store(flatten)]
    pub picker: PickerState,
    #[store(flatten)]
    pub command_palette: CommandPaletteState,
    #[store(flatten)]
    pub ref_picker: RefPickerState,
}

impl AppState {
    pub fn overlays_top(&self) -> Option<OverlaySurface> {
        self.overlays
            .stack
            .with(&self.store, |stack| stack.last().map(|e| e.surface))
    }

    pub fn overlays_active_name(&self) -> Option<&'static str> {
        self.overlays_top().map(overlay_name)
    }

    pub fn reset_picker(&mut self) {
        let d = PickerState::default();
        self.overlays.picker.kind.set(&self.store, d.kind);
        self.overlays.picker.query.set(&self.store, d.query);
        self.overlays.picker.entries.set(&self.store, d.entries);
        self.overlays
            .picker
            .selected_index
            .set(&self.store, d.selected_index);
        self.overlays
            .picker
            .hovered_index
            .set(&self.store, d.hovered_index);
        self.overlays.picker.list.set(&self.store, d.list);
        self.overlays
            .picker
            .browse_path
            .set(&self.store, d.browse_path);
        self.overlays
            .picker
            .ref_resolve_generation
            .set(&self.store, d.ref_resolve_generation);
    }

    pub fn reset_command_palette(&mut self) {
        let d = CommandPaletteState::default();
        self.overlays
            .command_palette
            .query
            .set(&self.store, d.query);
        self.overlays
            .command_palette
            .entries
            .set(&self.store, d.entries);
        self.overlays
            .command_palette
            .selected_index
            .set(&self.store, d.selected_index);
        self.overlays.command_palette.list.set(&self.store, d.list);
    }

    pub fn clear_overlays(&mut self) {
        self.overlays
            .stack
            .update(&self.store, |stack| stack.clear());
        self.reset_picker();
        self.reset_command_palette();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
    pub description: Option<String>,
    pub created_at_ms: u64,
    pub hovered: bool,
    /// When `Some`, the toast renders an externally-driven progress bar in
    /// place of the time-based one and is pinned (not auto-dismissed).
    pub progress: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    Available(AvailableUpdate),
    Downloading(AvailableUpdate),
    ReadyToRestart(StagedUpdate),
    Restarting(StagedUpdate),
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartupState {
    pub auto_compare_pending: bool,
    pub pending_pr_url: Option<String>,
    pub preferred_file_index: Option<usize>,
    pub preferred_file_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct DebugState {
    pub last_scene_primitive_count: usize,
    pub last_frame_time_us: u64,
}

#[derive(Debug)]
pub struct AppState {
    pub workspace_mode: Signal<WorkspaceMode>,
    pub compare_progress: Signal<Option<CompareProgress>>,
    pub app_view: Signal<AppView>,
    pub settings_section: Signal<SettingsSection>,
    pub compare: CompareStateStore,
    pub repository: RepositoryStateStore,
    pub workspace: WorkspaceStateStore,
    pub file_list: FileListStateStore,
    pub overlays: OverlayStackStateStore,
    pub focus: Signal<Option<FocusTarget>>,
    pub text_edit: TextEditStateStore,
    pub editor: EditorStateStore,
    pub github: GitHubStateStore,
    pub settings: Settings,
    pub startup: StartupState,
    pub last_error: Signal<Option<String>>,
    pub toasts: Signal<Vec<Toast>>,
    pub syntax_pack_installs: Signal<Vec<String>>,
    pub update: Signal<UpdateState>,
    /// Memoized: `true` when `focus` targets a text-editing field.
    pub text_focused: Signal<bool>,
    pub animation: crate::ui::animation::AnimationState,
    pub commit_editor: Editor,
    pub review_comment_editor: Editor,
    pub steering_prompt_editor: Editor,
    pub ai_openai_key: String,
    pub ai_anthropic_key: String,
    pub ai_openai_editing: bool,
    pub ai_anthropic_editing: bool,
    pub ai_generation_id: u64,
    pub ai_generation_active: bool,
    pub ai_generation_error: Option<String>,
    /// Shared reactive store. Signals (like `sidebar_visible`) are handles
    /// into this store. Kept in `AppState` so state methods (apply_action etc.)
    /// can freely read/write signals without threading a store parameter.
    pub store: Rc<SignalStore>,
    pub sidebar_visible: Signal<bool>,
    pub debug: DebugStateStore,
    pub clock_ms: u64,
    pub next_toast_id: u64,
    pub frecency: Option<FrecencyStore>,
    pub theme_names: Vec<String>,
    pub theme_variants: Vec<crate::core::themes::ThemeVariant>,
    pub theme_preview_original: Signal<Option<String>>,
    pub github_access_token: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        let store = Rc::new(SignalStore::default());
        let sidebar_visible = store.create(true);
        let focus = store.create(None::<FocusTarget>);
        let text_focused =
            store.create_memo(move |s| s.read(focus).is_some_and(|t| t.is_text_field()));
        let workspace_mode = store.create(WorkspaceMode::default());
        let compare_progress = store.create(None::<CompareProgress>);
        let app_view = store.create(AppView::default());
        let settings_section = store.create(SettingsSection::default());
        let last_error = store.create(None::<String>);
        let theme_preview_original = store.create(None::<String>);
        let toasts = store.create(Vec::<Toast>::new());
        let syntax_pack_installs = store.create(Vec::<String>::new());
        let update = store.create(UpdateState::default());
        let debug = DebugStateStore::new(&store, DebugState::default());
        let file_list = FileListStateStore::new_default(&store);
        let editor = EditorStateStore::new_default(&store);
        let overlays = OverlayStackStateStore::new_default(&store);
        let compare = CompareStateStore::new_default(&store);
        let repository = RepositoryStateStore::new_default(&store);
        let workspace = WorkspaceStateStore::new_default(&store);
        let text_edit = TextEditStateStore::new_default(&store);
        let github = GitHubStateStore::new_default(&store);
        Self {
            workspace_mode,
            compare_progress,
            app_view,
            settings_section,
            compare,
            repository,
            workspace,
            file_list,
            overlays,
            focus,
            text_edit,
            editor,
            github,
            settings: Settings::default(),
            startup: StartupState::default(),
            last_error,
            toasts,
            syntax_pack_installs,
            update,
            text_focused,
            animation: crate::ui::animation::AnimationState::default(),
            commit_editor: Editor::default(),
            review_comment_editor: Editor::default(),
            steering_prompt_editor: Editor::default(),
            ai_openai_key: String::new(),
            ai_anthropic_key: String::new(),
            ai_openai_editing: false,
            ai_anthropic_editing: false,
            ai_generation_id: 0,
            ai_generation_active: false,
            ai_generation_error: None,
            sidebar_visible,
            debug,
            store,
            clock_ms: 0,
            next_toast_id: 1,
            frecency: None,
            theme_names: Vec::new(),
            theme_variants: Vec::new(),
            theme_preview_original,
            github_access_token: None,
        }
    }
}

impl AppState {
    pub fn bootstrap(startup: StartupOptions, settings: Settings) -> (Self, Vec<Effect>) {
        let persisted = matching_persisted_compare(&startup, &settings).cloned();
        let repo_path = startup.args.repo.clone();
        let left_ref = startup
            .args
            .left
            .clone()
            .or_else(|| persisted.as_ref().map(|compare| compare.left_ref.clone()))
            .unwrap_or_default();
        let right_ref = startup
            .args
            .right
            .clone()
            .or_else(|| persisted.as_ref().map(|compare| compare.right_ref.clone()))
            .unwrap_or_default();
        let mode = startup
            .args
            .compare_mode
            .or_else(|| persisted.as_ref().map(|compare| compare.mode))
            .unwrap_or_default();
        let layout = startup
            .args
            .layout
            .or_else(|| persisted.as_ref().map(|compare| compare.layout))
            .unwrap_or(settings.viewport.layout);
        let renderer = startup
            .args
            .renderer
            .or_else(|| persisted.as_ref().map(|compare| compare.renderer))
            .unwrap_or_default();
        let auto_compare_pending = startup.wants_compare(mode, &left_ref, &right_ref);

        let store = Rc::new(SignalStore::default());
        let sidebar_visible = store.create(true);
        let focus = store.create(if repo_path.is_some() {
            Some(FocusTarget::TitleBar)
        } else {
            Some(FocusTarget::WorkspacePrimaryButton)
        });
        let text_focused =
            store.create_memo(move |s| s.read(focus).is_some_and(|t| t.is_text_field()));
        let workspace_mode = store.create(if repo_path.is_some() && auto_compare_pending {
            WorkspaceMode::Loading
        } else {
            WorkspaceMode::Empty
        });
        let compare_progress = store.create(None::<CompareProgress>);
        let app_view = store.create(AppView::default());
        let settings_section = store.create(SettingsSection::default());
        let last_error = store.create(None::<String>);
        let theme_preview_original = store.create(None::<String>);
        let toasts = store.create(Vec::<Toast>::new());
        let syntax_pack_installs = store.create(Vec::<String>::new());
        let update = store.create(UpdateState::default());
        let debug = DebugStateStore::new(&store, DebugState::default());
        let file_list = FileListStateStore::new_default(&store);
        let editor = EditorStateStore::new(
            &store,
            EditorState {
                layout,
                wrap_enabled: settings.viewport.wrap_enabled,
                wrap_column: settings.viewport.wrap_column,
                ..EditorState::default()
            },
        );
        let overlays = OverlayStackStateStore::new_default(&store);
        let compare = CompareStateStore::new(
            &store,
            CompareState {
                repo_path: repo_path.clone(),
                left_ref,
                right_ref,
                mode,
                layout,
                renderer,
                resolved_left: None,
                resolved_right: None,
            },
        );
        let repository = RepositoryStateStore::new_default(&store);
        let workspace = WorkspaceStateStore::new_default(&store);
        let text_edit = TextEditStateStore::new_default(&store);
        let initial_token_present = settings.github_user.is_some();
        let github = GitHubStateStore::new(
            &store,
            GitHubState {
                client_id: startup.github_client_id.clone(),
                auth: GitHubAuthState {
                    token_present: initial_token_present,
                    user: settings.github_user.clone(),
                    ..GitHubAuthState::default()
                },
                pull_request: PullRequestState::default(),
            },
        );
        let mut state = Self {
            workspace_mode,
            compare_progress,
            app_view,
            settings_section,
            compare,
            repository,
            workspace,
            file_list,
            overlays,
            focus,
            text_edit,
            editor,
            github,
            settings,
            startup: StartupState {
                auto_compare_pending,
                pending_pr_url: startup.args.open_pr.clone(),
                preferred_file_index: startup.args.file_index,
                preferred_file_path: startup.args.file_path.clone(),
            },
            last_error,
            toasts,
            syntax_pack_installs,
            update,
            text_focused,
            animation: crate::ui::animation::AnimationState::default(),
            commit_editor: Editor::default(),
            review_comment_editor: Editor::default(),
            steering_prompt_editor: Editor::default(),
            ai_openai_key: String::new(),
            ai_anthropic_key: String::new(),
            ai_openai_editing: false,
            ai_anthropic_editing: false,
            ai_generation_id: 0,
            ai_generation_active: false,
            ai_generation_error: None,
            sidebar_visible,
            debug,
            store,
            clock_ms: 0,
            next_toast_id: 1,
            frecency: crate::core::frecency::open_default_store(),
            theme_names: Vec::new(),
            theme_variants: Vec::new(),
            theme_preview_original,
            github_access_token: None,
        };
        let seed_prompt = if state.settings.ai_steering_prompt.trim().is_empty() {
            crate::ai::DEFAULT_STEERING_PROMPT
        } else {
            state.settings.ai_steering_prompt.as_str()
        };
        state.steering_prompt_editor.set_text(seed_prompt);
        state.sync_settings_snapshot();

        let mut effects = Vec::new();
        if let Some(path) = repo_path {
            state
                .repository
                .status
                .set(&state.store, AsyncStatus::Loading);

            // Bootstrap: seed the loading panel so a slow cold-boot open
            // shows staged progress. Reveal is gated by the same 500ms
            // threshold as user-initiated opens — if the whole bootstrap
            // open completes within the threshold the panel never appears
            // and the user lands straight in the ready UI.
            let boot_gen = state
                .workspace
                .compare_generation
                .get(&state.store)
                .saturating_add(1);
            state
                .workspace
                .compare_generation
                .set(&state.store, boot_gen);
            let repo_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("repository")
                .to_owned();
            state.compare_progress.set(
                &state.store,
                Some(CompareProgress {
                    generation: boot_gen,
                    phase: ComparePhase::OpeningRepo,
                    subject: LoadingSubject::RepoOpen { name: repo_name },
                    started_at_ms: 0,
                    reveal_at_ms: COMPARE_REVEAL_DELAY_MS,
                    file_count_total: None,
                    files_loaded: 0,
                }),
            );

            effects.push(
                RepositoryEffect::SyncRepository {
                    path: path.clone(),
                    reason: RepositorySyncReason::Open,
                    reporter_generation: Some(boot_gen),
                }
                .into(),
            );
            effects.push(RepositoryEffect::WatchRepository { path: Some(path) }.into());
        }
        if let Some(token) = startup.github_token.clone() {
            state.github_access_token = Some(token.clone());
            state.github.auth.token_present.set(&state.store, true);
            effects.push(GitHubEffect::SaveGitHubToken(token).into());
        } else {
            effects.push(GitHubEffect::LoadGitHubToken.into());
        }

        // Show the cached user + avatar optimistically while the token loads.
        if let Some(user) = state.settings.github_user.as_ref()
            && let Some(url) = avatar_url_sized(&user.avatar_url, 128)
        {
            state.github.auth.avatar_fetching.set(&state.store, true);
            effects.push(GitHubEffect::FetchAvatar { url }.into());
        }

        effects.push(SyntaxEffect::InstallCommonSyntaxPacks.into());
        effects.push(AiEffect::LoadAiKeys.into());
        if state.update_polling_enabled() {
            effects.push(UpdateEffect::CheckForUpdates { silent: true }.into());
        }
        (state, effects)
    }

    pub fn apply_action<A: Into<Action>>(&mut self, action: A) -> Vec<Effect> {
        let action = action.into();
        match action {
            Action::App(action) => app::reduce_action(self, action),
            Action::Workspace(action) => workspace::reduce_action(self, action),
            Action::Compare(action) => compare::reduce_action(self, action),
            Action::Repository(action) => repository::reduce_action(self, action),
            Action::FileList(action) => file_list::reduce_action(self, action),
            Action::Overlay(action) => overlay::reduce_action(self, action),
            Action::Editor(action) => editor::reduce_action(self, action),
            Action::TextEdit(action) => text_edit::reduce_action(self, action),
            Action::Settings(action) => settings::reduce_action(self, action),
            Action::GitHub(action) => github::reduce_action(self, action),
            Action::Update(action) => update::reduce_action(self, action),
            Action::Syntax(action) => syntax::reduce_action(self, action),
            Action::Ai(action) => ai::reduce_action(self, action),
            Action::Noop => Vec::new(),
        }
    }

    pub fn apply_event(&mut self, event: AppEvent) -> Vec<Effect> {
        match event {
            AppEvent::Ui(event) => app::reduce_event(self, event),
            AppEvent::Repository(event) => repository::reduce_event(self, event),
            AppEvent::Compare(event) => compare::reduce_event(self, event),
            AppEvent::GitHub(event) => github::reduce_event(self, event),
            AppEvent::Settings(event) => settings::reduce_event(self, event),
            AppEvent::Update(event) => update::reduce_event(self, event),
            AppEvent::Syntax(event) => syntax::reduce_event(self, event),
            AppEvent::Ai(event) => ai::reduce_event(self, event),
        }
    }

    pub fn window_title(&self) -> String {
        let workspace_mode = workspace_mode_name(self.workspace_mode.get(&self.store));
        let repo = self.compare.repo_path.with(&self.store, |p| {
            p.as_deref()
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .unwrap_or("native")
                .to_owned()
        });
        let selected_path = self.workspace.selected_file_path.get(&self.store);
        if let Some(path) = selected_path.as_deref() {
            format!("diffy native - {repo} [{workspace_mode}] {path}")
        } else {
            format!("diffy native - {repo} [{workspace_mode}]")
        }
    }

    pub fn update_time(&mut self, now_ms: u64) {
        self.clock_ms = now_ms;
        self.animation.tick(now_ms);
        self.toasts.update(&self.store, |toasts| {
            toasts.retain(|toast| {
                toast.hovered
                    || toast.progress.is_some()
                    || now_ms.saturating_sub(toast.created_at_ms) < TOAST_LIFETIME_MS
            });
        });
    }

    pub fn update_polling_enabled(&self) -> bool {
        self.settings.auto_update
            && crate::core::update::updates_configured()
            && !cfg!(debug_assertions)
            && !matches!(
                self.update.get(&self.store),
                UpdateState::Downloading(_)
                    | UpdateState::ReadyToRestart(_)
                    | UpdateState::Restarting(_)
            )
    }

    pub fn cursor_blink_epoch(&self) -> Option<u64> {
        self.is_text_focused().then(|| {
            self.clock_ms
                .saturating_sub(self.text_edit.cursor_moved_at_ms.get(&self.store))
                / CURSOR_BLINK_INTERVAL_MS
        })
    }

    pub fn next_cursor_blink_at_ms(&self) -> Option<u64> {
        self.is_text_focused().then(|| {
            let moved_at = self.text_edit.cursor_moved_at_ms.get(&self.store);
            let elapsed = self.clock_ms.saturating_sub(moved_at);
            let next_epoch = elapsed / CURSOR_BLINK_INTERVAL_MS + 1;
            moved_at.saturating_add(next_epoch.saturating_mul(CURSOR_BLINK_INTERVAL_MS))
        })
    }

    pub fn next_toast_expiry_at_ms(&self) -> Option<u64> {
        self.toasts.with(&self.store, |toasts| {
            toasts
                .iter()
                .filter(|toast| !toast.hovered && toast.progress.is_none())
                .map(|toast| toast.created_at_ms.saturating_add(TOAST_LIFETIME_MS))
                .min()
        })
    }

    pub fn active_overlay_name(&self) -> Option<&'static str> {
        self.overlays_active_name()
    }

    fn open_repository(&mut self, path: PathBuf) -> Vec<Effect> {
        self.workspace_mode.set(&self.store, WorkspaceMode::Loading);
        self.compare.repo_path.set(&self.store, Some(path.clone()));
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        self.repository
            .status
            .set(&self.store, AsyncStatus::Loading);
        self.workspace_clear_compare();
        self.reset_file_list();
        self.editor_clear_document();
        self.editor.focused.set(&self.store, false);
        self.last_error.set(&self.store, None);
        self.github.pull_request.cache.update(&self.store, |c| {
            c.clear();
        });
        self.github
            .pull_request
            .pending_confirm
            .set(&self.store, None);
        self.clear_overlays();
        self.focus.set(&self.store, Some(FocusTarget::TitleBar));
        self.sync_settings_snapshot();

        // Seed the progress panel with a repo-open subject. We piggy-back
        // on `compare_generation` as the loading generation — any in-flight
        // compare is invalidated when the user opens a new repo anyway,
        // and `handle_compare_progress_update` just matches on whatever
        // generation the panel records.
        let next_gen = self
            .workspace
            .compare_generation
            .get(&self.store)
            .saturating_add(1);
        self.workspace.compare_generation.set(&self.store, next_gen);
        let repo_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("repository")
            .to_owned();
        // Always delay the panel reveal — a tiny repo that opens in under
        // the threshold should finish without ever flashing a loading UI.
        // Unlike re-compare (which can preserve the old diff during the
        // grace window), repo-open has nothing to fall back to visually;
        // the empty background / previous workspace is what the user sees
        // for 500ms, which is a cheap price for zero flash on fast ops.
        let started_at_ms = self.clock_ms;
        let reveal_at_ms = started_at_ms.saturating_add(COMPARE_REVEAL_DELAY_MS);
        self.compare_progress.set(
            &self.store,
            Some(CompareProgress {
                generation: next_gen,
                phase: ComparePhase::OpeningRepo,
                subject: LoadingSubject::RepoOpen { name: repo_name },
                started_at_ms,
                reveal_at_ms,
                file_count_total: None,
                files_loaded: 0,
            }),
        );

        vec![
            SettingsEffect::SaveSettings(self.settings.clone()).into(),
            RepositoryEffect::SyncRepository {
                path: path.clone(),
                reason: RepositorySyncReason::Open,
                reporter_generation: Some(next_gen),
            }
            .into(),
            RepositoryEffect::WatchRepository { path: Some(path) }.into(),
        ]
    }

    /// Clear the workspace back to a blank "no compare loaded" state. Replaces
    /// the former `WorkspaceState::clear_compare(&mut self)` method.
    fn workspace_clear_compare(&mut self) {
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::None);
        self.workspace.status.set(&self.store, AsyncStatus::Idle);
        self.workspace
            .status_operation_pending
            .set(&self.store, false);
        self.workspace.status_generation.set(&self.store, 0);
        self.workspace.files.set(&self.store, Vec::new());
        self.workspace.status_items.set(&self.store, Vec::new());
        self.workspace.selected_file_index.set(&self.store, None);
        self.workspace.selected_file_path.set(&self.store, None);
        self.workspace.selected_status_scope.set(&self.store, None);
        self.workspace.compare_output.set(&self.store, None);
        self.workspace.compare_total_stats.set(&self.store, None);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.workspace
            .compare_stats_hydration_active
            .set(&self.store, false);
        self.workspace.active_file.set(&self.store, None);
        self.workspace.active_file_loading.set(&self.store, None);
        self.workspace.raw_diff_len.set(&self.store, 0);
        self.workspace.used_fallback.set(&self.store, false);
        self.workspace
            .fallback_message
            .set(&self.store, String::new());
        self.workspace.sidebar_auto_width.set(&self.store, None);
        self.workspace.range_commits.set(&self.store, Vec::new());
        self.workspace.pre_drill_compare.set(&self.store, None);
        self.workspace.expansions.update(&self.store, |m| m.clear());
    }

    fn handle_repository_snapshot(&mut self, payload: RepositorySnapshot) -> Vec<Effect> {
        tracing::debug!(
            path = %payload.path.display(),
            reason = ?payload.reason,
            change_kind = ?payload.change_kind,
            pending = self.workspace.status_operation_pending.get(&self.store),
            status_gen = self.workspace.status_generation.get(&self.store),
            "handle_repository_snapshot: entered"
        );
        if self
            .compare
            .repo_path
            .with(&self.store, |p| p.as_ref() != Some(&payload.path))
        {
            tracing::warn!("handle_repository_snapshot: path mismatch, ignored");
            return Vec::new();
        }

        self.repository.status.set(&self.store, AsyncStatus::Ready);
        self.repository.branches.set(&self.store, payload.branches);
        self.repository.tags.set(&self.store, payload.tags);
        self.repository.commits.set(&self.store, payload.commits);
        self.workspace
            .status_items
            .set(&self.store, payload.status_items);

        // Tear down a repo-open progress panel. Compare-subject progress
        // survives — a kickoff_compare may be queued below and will
        // replace it atomically via its own seeding path.
        self.compare_progress.update(&self.store, |slot| {
            if let Some(p) = slot.as_ref()
                && matches!(p.subject, LoadingSubject::RepoOpen { .. })
            {
                *slot = None;
            }
        });

        match payload.reason {
            RepositorySyncReason::Open => {
                if let Some(ref store) = self.frecency {
                    store.record_access(&format!("repo:{}", payload.path.display()));
                }
                let mut effects = self.persist_settings_effect();
                if let Some(url) = self.startup.pending_pr_url.clone() {
                    self.startup.pending_pr_url = None;
                    self.github
                        .pull_request
                        .status
                        .set(&self.store, AsyncStatus::Loading);
                    if let Some(parsed) = crate::core::vcs::github::parse_pr_url(&url) {
                        let key: PrKey = (parsed.owner, parsed.repo, parsed.number);
                        self.github.pull_request.cache.update(&self.store, |c| {
                            c.entry(key.clone()).or_insert_with(|| PrCacheEntry {
                                meta: PrPeekMeta::Loading,
                                diff: PrPeekDiff::Loading,
                                last_peek_ms: 0,
                            });
                        });
                        self.github
                            .pull_request
                            .pending_confirm
                            .set(&self.store, Some(key));
                    }
                    effects.push(
                        GitHubEffect::LoadPullRequest {
                            url,
                            repo_path: payload.path,
                            github_token: self.github_access_token.clone(),
                        }
                        .into(),
                    );
                } else if self.startup.auto_compare_pending {
                    self.startup.auto_compare_pending = false;
                    effects.extend(self.kickoff_compare());
                } else if let Some(persisted) = self.settings.last_compare.as_ref().filter(|c| {
                    c.repo_path.as_ref() == Some(&payload.path)
                        && compare_refs_are_valid(c.mode, &c.left_ref, &c.right_ref)
                }) {
                    self.compare
                        .left_ref
                        .set(&self.store, persisted.left_ref.clone());
                    self.compare
                        .right_ref
                        .set(&self.store, persisted.right_ref.clone());
                    self.compare.mode.set(&self.store, persisted.mode);
                    effects.extend(self.kickoff_compare());
                } else {
                    self.compare.left_ref.set(&self.store, "HEAD".to_owned());
                    self.compare.right_ref.set(
                        &self.store,
                        crate::core::vcs::git::service::WORKDIR_REF.to_owned(),
                    );
                    self.compare.mode.set(&self.store, CompareMode::ThreeDot);
                    effects.extend(self.activate_status_view(true));
                }
                effects
            }
            RepositorySyncReason::Dirty | RepositorySyncReason::Rescan => {
                if self.workspace.source.get(&self.store) == WorkspaceSource::Status {
                    return self.activate_status_view(false);
                }

                let (mode, left_ref, right_ref) = (
                    self.compare.mode.get(&self.store),
                    self.compare.left_ref.get(&self.store),
                    self.compare.right_ref.get(&self.store),
                );
                if !compare_refs_are_valid(mode, &left_ref, &right_ref) {
                    return Vec::new();
                }

                match payload.change_kind {
                    Some(RepositoryChangeKind::Git | RepositoryChangeKind::Both) => {
                        self.kickoff_compare()
                    }
                    Some(RepositoryChangeKind::Worktree)
                        if right_ref == crate::core::vcs::git::service::WORKDIR_REF =>
                    {
                        self.kickoff_compare()
                    }
                    _ => Vec::new(),
                }
            }
        }
    }

    fn expand_context(
        &mut self,
        hunk_index: usize,
        direction: crate::ui::editor::expansion::ExpandDirection,
        amount: u32,
    ) -> Vec<Effect> {
        use crate::events::ContextDirection;
        use crate::ui::editor::expansion::ExpandDirection;

        if amount == 0 {
            return Vec::new();
        }

        let ctx_direction = match direction {
            ExpandDirection::Above => ContextDirection::Above,
            ExpandDirection::Below => ContextDirection::Below,
        };
        self.dispatch_context_expansion(hunk_index, ctx_direction, amount)
    }

    fn expand_all_context(&mut self) -> Vec<Effect> {
        use crate::events::ContextDirection;
        self.dispatch_context_expansion(0, ContextDirection::All, 0)
    }

    fn dispatch_context_expansion(
        &mut self,
        hunk_index: usize,
        direction: crate::events::ContextDirection,
        amount: u32,
    ) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };

        let Some((
            file_index,
            path,
            old_reference,
            new_reference,
            generation,
            cached_old_lines,
            cached_new_lines,
        )) = self.workspace.active_file.with(&self.store, |af| {
            let active = af.as_ref()?;
            if active.base_carbon_file.hunks.is_empty() {
                return None;
            }
            Some((
                active.index,
                active.path.clone(),
                active.left_ref.clone(),
                if active.right_ref.is_empty() {
                    active.left_ref.clone()
                } else {
                    active.right_ref.clone()
                },
                self.workspace.compare_generation.get(&self.store),
                active.old_file_lines.clone(),
                active.file_lines.clone(),
            ))
        })
        else {
            return Vec::new();
        };

        if let (Some(old_lines), Some(new_lines)) = (cached_old_lines, cached_new_lines) {
            self.apply_context_expansion(direction, hunk_index, amount, old_lines, new_lines);
            return self
                .request_active_file_syntax_effect()
                .into_iter()
                .collect();
        }

        vec![
            RepositoryEffect::FetchContextLines(crate::effects::FetchContextLinesRequest {
                repo_path,
                old_reference,
                new_reference,
                path,
                generation,
                file_index,
                hunk_index,
                direction,
                amount,
            })
            .into(),
        ]
    }

    fn handle_context_lines_ready(
        &mut self,
        payload: crate::events::ContextLinesReady,
    ) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

        let matches_active = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref()
                .is_some_and(|a| a.index == payload.file_index && a.path == payload.path)
        });
        if !matches_active {
            return Vec::new();
        }

        let old_lines = Arc::new(payload.old_lines);
        let new_lines = Arc::new(payload.new_lines);
        self.apply_context_expansion(
            payload.direction,
            payload.hunk_index,
            payload.amount,
            old_lines,
            new_lines,
        );
        self.request_active_file_syntax_effect()
            .into_iter()
            .collect()
    }

    fn apply_context_expansion(
        &mut self,
        direction: crate::events::ContextDirection,
        hunk_index: usize,
        amount: u32,
        old_lines: Arc<Vec<String>>,
        new_lines: Arc<Vec<String>>,
    ) {
        use crate::events::ContextDirection;

        let Some((
            active_index,
            active_path,
            mut carbon_file,
            mut expansion,
            carbon_overlays,
            token_buffer,
        )) = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref().map(|a| {
                (
                    a.index,
                    a.path.clone(),
                    a.base_carbon_file.clone(),
                    a.carbon_expansion.clone(),
                    a.carbon_overlays.clone(),
                    a.token_buffer.clone(),
                )
            })
        })
        else {
            return;
        };

        hydrate_carbon_full_text(&mut carbon_file, &old_lines, &new_lines);
        match direction {
            ContextDirection::Above => {
                carbon::expand_context(
                    &carbon_file,
                    &mut expansion,
                    carbon::HunkId(hunk_index as u32),
                    carbon::ExpansionDirection::Above,
                    amount,
                );
            }
            ContextDirection::Below => {
                carbon::expand_context(
                    &carbon_file,
                    &mut expansion,
                    carbon::HunkId(hunk_index as u32),
                    carbon::ExpansionDirection::Below,
                    amount,
                );
            }
            ContextDirection::All => {
                let hunk_ids = carbon_file
                    .hunks
                    .iter()
                    .map(|hunk| hunk.id)
                    .collect::<Vec<_>>();
                for hunk_id in hunk_ids {
                    let caps = carbon::expansion_caps(&carbon_file, hunk_id);
                    carbon::expand_context(
                        &carbon_file,
                        &mut expansion,
                        hunk_id,
                        carbon::ExpansionDirection::Above,
                        caps.above,
                    );
                    carbon::expand_context(
                        &carbon_file,
                        &mut expansion,
                        hunk_id,
                        carbon::ExpansionDirection::Below,
                        caps.below,
                    );
                }
            }
        }
        self.workspace.expansions.update(&self.store, |map| {
            map.insert(active_path.clone(), expansion.clone());
        });

        let render_doc = build_render_doc_from_carbon(
            &carbon_file,
            active_index,
            &expansion,
            &carbon_overlays,
            &token_buffer,
        );
        let total_lines = new_lines.len() as u32;

        let preserved_scroll = self.editor.scroll_top_px.get(&self.store);

        self.workspace.active_file.update(&self.store, |af| {
            if let Some(active) = af.as_mut() {
                active.carbon_file = carbon_file;
                active.carbon_expansion = expansion;
                active.render_doc = render_doc;
                active.file_line_count = Some(total_lines);
                active.old_file_lines = Some(old_lines);
                active.file_lines = Some(new_lines);
            }
        });
        self.editor_clear_document();
        self.editor.scroll_top_px.set(&self.store, preserved_scroll);
    }

    #[profiling::function]
    fn handle_compare_finished(&mut self, payload: CompareFinished) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

        let generation = payload.generation;
        let history_left = payload.resolved_left.clone();
        let history_right = if payload.resolved_right == crate::core::vcs::git::WORKDIR_REF {
            "HEAD".to_owned()
        } else {
            payload.resolved_right.clone()
        };
        let syntax_warmup_paths = payload
            .output
            .carbon
            .files
            .iter()
            .map(|file| file.path().to_owned())
            .collect::<Vec<_>>();
        self.workspace
            .status_operation_pending
            .set(&self.store, false);
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Compare);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace_mode.set(&self.store, WorkspaceMode::Ready);
        self.compare.layout.set(&self.store, payload.spec.layout);
        self.compare
            .renderer
            .set(&self.store, payload.spec.renderer);
        self.compare
            .resolved_left
            .set(&self.store, Some(payload.resolved_left));
        self.compare
            .resolved_right
            .set(&self.store, Some(payload.resolved_right));
        self.workspace
            .raw_diff_len
            .set(&self.store, payload.output.raw_diff.len());
        self.workspace
            .used_fallback
            .set(&self.store, payload.output.used_fallback);
        self.workspace
            .fallback_message
            .set(&self.store, payload.output.fallback_message.clone());
        self.workspace.files.set(
            &self.store,
            build_carbon_file_entries(&payload.output.carbon.files),
        );
        let total_files = payload.output.carbon.files.len() as u32;
        let has_deferred_stats = payload
            .output
            .carbon
            .files
            .iter()
            .any(|file| file.stats_deferred);
        let eager_total_stats = (!has_deferred_stats).then(|| {
            payload
                .output
                .carbon
                .files
                .iter()
                .map(carbon_file_stats)
                .fold((0_i32, 0_i32), |acc, next| {
                    (acc.0.saturating_add(next.0), acc.1.saturating_add(next.1))
                })
        });
        self.workspace
            .compare_output
            .set(&self.store, Some(payload.output));
        self.workspace
            .compare_total_stats
            .set(&self.store, eager_total_stats);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.workspace
            .compare_stats_hydration_active
            .set(&self.store, false);
        self.workspace.active_file_loading.set(&self.store, None);
        self.workspace.sidebar_auto_width.set(&self.store, None);
        // Record the discovered file count + advance the phase. The progress
        // panel stays up until the first file finishes mounting (or, for
        // small-file fast paths, is cleared by install_compare_active_file).
        self.compare_progress.update(&self.store, |slot| {
            if let Some(p) = slot.as_mut() {
                p.file_count_total = Some(total_files);
                p.phase = ComparePhase::PopulatingList;
            }
        });
        if self
            .workspace
            .pre_drill_compare
            .with(&self.store, |p| p.is_none())
        {
            self.workspace
                .range_commits
                .set(&self.store, payload.range_commits);
        }
        self.file_list.scroll_offset_px.set(&self.store, 0.0);
        self.file_list
            .commits_scroll_offset_px
            .set(&self.store, 0.0);
        self.set_focus(Some(FocusTarget::FileList));
        self.editor_clear_document();
        self.clear_overlays();

        let preferred_index = self
            .startup
            .preferred_file_index
            .or(self.workspace.selected_file_index.get(&self.store));
        let preferred_path = self
            .startup
            .preferred_file_path
            .clone()
            .or_else(|| self.workspace.selected_file_path.get(&self.store));

        let (file_count, index_for_path) = self.workspace.files.with(&self.store, |files| {
            let idx = preferred_path
                .as_deref()
                .and_then(|path| files.iter().position(|file| file.path == path));
            (files.len(), idx)
        });

        let mut effects = Vec::new();
        let mut selected_syntax_paths = Vec::new();
        let history_effect = self.compare.repo_path.get(&self.store).map(|repo_path| {
            CompareEffect::LoadHistory(Task {
                generation,
                request: CompareHistoryRequest {
                    repo_path,
                    left_ref: history_left,
                    right_ref: history_right,
                },
            })
            .into()
        });
        if let Some(index) = index_for_path
            .or(preferred_index.filter(|index| *index < file_count))
            .or_else(|| (file_count > 0).then_some(0))
        {
            if let Some(path) = syntax_warmup_paths.get(index) {
                selected_syntax_paths.push(path.clone());
            }
            effects.extend(self.select_file(index, true));
        } else {
            self.workspace.selected_file_index.set(&self.store, None);
            self.workspace.selected_file_path.set(&self.store, None);
            self.workspace.selected_status_scope.set(&self.store, None);
            self.workspace.active_file.set(&self.store, None);
            self.workspace.active_file_loading.set(&self.store, None);
            // No files to select — the compare succeeded but has no diffs.
            // Tear down the progress panel; the "repo ready" hint takes over.
            self.compare_progress.set(&self.store, None);
            self.editor_clear_document();
        }
        if let Some(effect) =
            self.syntax_pack_warmup_effect_for_paths(&syntax_warmup_paths, &selected_syntax_paths)
        {
            effects.insert(0, effect);
        }
        if let Some(effect) = history_effect {
            effects.push(effect);
        }

        let (used_fallback, fallback_message) = (
            self.workspace.used_fallback.get(&self.store),
            self.workspace.fallback_message.get(&self.store),
        );
        if used_fallback && !fallback_message.is_empty() {
            self.push_info(&fallback_message);
        }
        effects
    }

    fn handle_compare_history_ready(&mut self, payload: CompareHistoryReady) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }
        if self
            .workspace
            .pre_drill_compare
            .with(&self.store, |p| p.is_some())
        {
            return Vec::new();
        }
        self.workspace
            .range_commits
            .set(&self.store, payload.range_commits);
        Vec::new()
    }

    fn handle_status_diff_finished(&mut self, payload: StatusDiffFinished) -> Vec<Effect> {
        let current_gen = self.workspace.status_generation.get(&self.store);
        tracing::debug!(
            payload_gen = payload.generation,
            current_gen,
            payload_index = payload.index,
            payload_path = %payload.item.path,
            payload_scope = ?payload.item.scope,
            "handle_status_diff_finished: entered"
        );
        if payload.generation != current_gen {
            tracing::debug!(
                "handle_status_diff_finished: generation mismatch, discarding (pending NOT cleared)"
            );
            return Vec::new();
        }
        let matches =
            self.workspace
                .status_items
                .with(&self.store, |items| match items.get(payload.index) {
                    Some(current) => {
                        current.path == payload.item.path && current.scope == payload.item.scope
                    }
                    None => false,
                });
        if !matches {
            let current_items_at_idx = self.workspace.status_items.with(&self.store, |items| {
                items
                    .get(payload.index)
                    .map(|i| format!("{}:{:?}", i.path, i.scope))
                    .unwrap_or_else(|| "<out of range>".to_owned())
            });
            tracing::debug!(
                current_items_at_idx,
                "handle_status_diff_finished: item mismatch, discarding (pending NOT cleared)"
            );
            return Vec::new();
        }

        tracing::debug!("handle_status_diff_finished: clearing status_operation_pending");
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Status);
        self.workspace
            .status_operation_pending
            .set(&self.store, false);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace_mode.set(&self.store, WorkspaceMode::Ready);
        let output = payload.output;
        self.workspace
            .used_fallback
            .set(&self.store, output.used_fallback);
        self.workspace
            .fallback_message
            .set(&self.store, output.fallback_message.clone());
        self.workspace
            .raw_diff_len
            .set(&self.store, output.raw_diff.len());
        self.workspace.compare_output.set(&self.store, None);
        self.workspace.active_file_loading.set(&self.store, None);

        let Some(carbon_file) = output.carbon.files.first() else {
            self.workspace.active_file.set(&self.store, None);
            self.editor_clear_document();
            return Vec::new();
        };
        let prepared = prepare_active_file(payload.index, carbon_file);

        self.workspace
            .selected_file_index
            .set(&self.store, Some(payload.index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(payload.item.path.clone()));
        self.workspace
            .selected_status_scope
            .set(&self.store, Some(payload.item.scope));
        let (left_ref, right_ref) = refs_for_status_scope(payload.item.scope);
        // Preserve scroll/hover/positional editor state when refreshing the
        // same file (e.g. after staging a hunk). Only reset when the path
        // changed (navigating to a different file).
        let same_file = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref().is_some_and(|a| a.path == payload.item.path)
        });
        let active_file = self.build_active_file(
            payload.index,
            payload.item.path.clone(),
            prepared,
            left_ref,
            right_ref,
        );
        self.workspace
            .active_file
            .set(&self.store, Some(active_file));
        if !same_file {
            self.editor_clear_document();
            self.editor
                .line_selection
                .update(&self.store, |ls| ls.clear());
        }
        if self.editor.search.open.get(&self.store) {
            self.recompute_search_matches();
        }
        self.request_active_file_syntax_effect()
            .into_iter()
            .collect()
    }

    #[profiling::function]
    fn handle_compare_file_finished(&mut self, payload: CompareFileFinished) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

        let matches_selected = self
            .workspace
            .selected_file_path
            .get(&self.store)
            .as_deref()
            == Some(payload.path.as_str());
        let matches_loading = self
            .workspace
            .active_file_loading
            .with(&self.store, |loading| {
                loading.as_ref().is_some_and(|loading| {
                    loading.index == payload.index && loading.path == payload.path
                })
            });
        if !matches_selected || !matches_loading {
            return Vec::new();
        }

        self.install_compare_active_file(payload.index, payload.path, payload.prepared);
        let mut effects: Vec<_> = self
            .request_active_file_syntax_effect()
            .into_iter()
            .collect();
        if let Some(effect) = self.start_compare_total_stats_if_needed() {
            effects.push(effect);
        } else if let Some(effect) = self.start_compare_stats_hydration_if_idle() {
            effects.push(effect);
        }
        effects
    }

    fn handle_compare_stats_ready(&mut self, payload: CompareStatsReady) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

        self.workspace
            .compare_total_stats
            .set(&self.store, Some((payload.additions, payload.deletions)));
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.start_compare_stats_hydration_if_idle()
            .into_iter()
            .collect()
    }

    fn handle_compare_file_stats_ready(&mut self, payload: CompareFileStatsReady) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

        self.apply_compare_file_stats(&payload.stats);
        if let Some(effect) = self.next_compare_stats_hydration_effect() {
            vec![effect]
        } else {
            self.workspace
                .compare_stats_hydration_active
                .set(&self.store, false);
            Vec::new()
        }
    }

    fn start_compare_stats_hydration_if_idle(&mut self) -> Option<Effect> {
        if self
            .workspace
            .compare_stats_hydration_active
            .get(&self.store)
        {
            return None;
        }

        let effect = self.next_compare_stats_hydration_effect()?;
        self.workspace
            .compare_stats_hydration_active
            .set(&self.store, true);
        Some(effect)
    }

    fn start_compare_total_stats_if_needed(&mut self) -> Option<Effect> {
        if self
            .workspace
            .compare_total_stats
            .get(&self.store)
            .is_some()
            || self.workspace.compare_total_stats_loading.get(&self.store)
        {
            return None;
        }
        let has_deferred_stats = self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .is_some_and(|output| output.carbon.files.iter().any(|file| file.stats_deferred))
        });
        if !has_deferred_stats {
            return None;
        }
        let repo_path = self.compare.repo_path.get(&self.store)?;
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, true);

        Some(
            CompareEffect::LoadStats(Task {
                generation: self.workspace.compare_generation.get(&self.store),
                request: CompareStatsRequest {
                    repo_path,
                    spec: CompareSpec {
                        mode: self.compare.mode.get(&self.store),
                        left_ref: self.compare.left_ref.get(&self.store),
                        right_ref: self.compare.right_ref.get(&self.store),
                        renderer: self.compare.renderer.get(&self.store),
                        layout: self.compare.layout.get(&self.store),
                    },
                },
            })
            .into(),
        )
    }

    fn next_compare_stats_hydration_effect(&self) -> Option<Effect> {
        let repo_path = self.compare.repo_path.get(&self.store)?;
        let files = self
            .workspace
            .compare_output
            .with(&self.store, |maybe_output| {
                maybe_output
                    .as_ref()
                    .map(|output| {
                        output
                            .carbon
                            .files
                            .iter()
                            .enumerate()
                            .filter(|(_, file)| file.stats_deferred)
                            .take(COMPARE_STATS_CHUNK_SIZE)
                            .map(|(index, file)| CompareFileStatsItem {
                                index,
                                file: file.clone(),
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            });
        if files.is_empty() {
            return None;
        }

        Some(
            CompareEffect::LoadFileStats(Task {
                generation: self.workspace.compare_generation.get(&self.store),
                request: CompareFileStatsRequest { repo_path, files },
            })
            .into(),
        )
    }

    fn apply_compare_file_stats(&mut self, stats: &[CompareFileStat]) {
        if stats.is_empty() {
            return;
        }

        self.workspace
            .compare_output
            .update(&self.store, |maybe_output| {
                let Some(output) = maybe_output.as_mut() else {
                    return;
                };
                for stat in stats {
                    let Some(file) = output.carbon.files.get_mut(stat.index) else {
                        continue;
                    };
                    if file.path() != stat.path {
                        continue;
                    }
                    file.additions = i32_to_u32_nonnegative(stat.additions);
                    file.deletions = i32_to_u32_nonnegative(stat.deletions);
                    file.stats_deferred = false;
                }
            });

        self.workspace.files.update(&self.store, |files| {
            for stat in stats {
                let Some(file) = files.get_mut(stat.index) else {
                    continue;
                };
                if file.path != stat.path {
                    continue;
                }
                file.additions = stat.additions;
                file.deletions = stat.deletions;
            }
        });

        self.workspace.active_file.update(&self.store, |slot| {
            let Some(active) = slot.as_mut() else {
                return;
            };
            for stat in stats {
                if active.index != stat.index || active.path != stat.path {
                    continue;
                }
                active.carbon_file.additions = i32_to_u32_nonnegative(stat.additions);
                active.carbon_file.deletions = i32_to_u32_nonnegative(stat.deletions);
                active.base_carbon_file.additions = i32_to_u32_nonnegative(stat.additions);
                active.base_carbon_file.deletions = i32_to_u32_nonnegative(stat.deletions);
                break;
            }
        });
    }

    fn handle_file_syntax_ready(&mut self, payload: FileSyntaxReady) -> Vec<Effect> {
        if payload.generation != self.active_syntax_generation() {
            return Vec::new();
        }

        let mut applied = false;
        self.workspace.active_file.update(&self.store, |slot| {
            let Some(active) = slot.as_mut() else {
                return;
            };
            if active.index != payload.file_index
                || active.path != payload.path
                || active.syntax_request_id != payload.request_id
            {
                return;
            }

            active.syntax_pending = None;
            push_syntax_covered_window(&mut active.syntax_covered, payload.window);
            apply_syntax_tokens_to_file(
                &mut active.carbon_overlays,
                &mut active.token_buffer,
                &payload.tokens,
            );
            active.render_doc = build_render_doc_from_carbon(
                &active.carbon_file,
                active.index,
                &active.carbon_expansion,
                &active.carbon_overlays,
                &active.token_buffer,
            );
            applied = true;
        });

        if !applied {
            return Vec::new();
        }

        self.request_active_file_syntax_effect()
            .into_iter()
            .collect()
    }

    fn handle_syntax_pack_install_started(&mut self, language: &str) {
        self.syntax_pack_installs.update(&self.store, |active| {
            if !active.iter().any(|item| item == language) {
                active.push(language.to_owned());
            }
        });
    }

    fn handle_syntax_pack_install_finished(&mut self, language: &str) {
        self.syntax_pack_installs
            .update(&self.store, |active| active.retain(|item| item != language));
    }

    pub fn syntax_pack_install_active(&self) -> bool {
        self.syntax_pack_installs
            .with(&self.store, |active| !active.is_empty())
    }

    fn syntax_pack_warmup_effect_for_paths(
        &self,
        paths: &[String],
        exclude_paths: &[String],
    ) -> Option<Effect> {
        let highlighter = phosphor::Highlighter::new();
        let excluded_languages = exclude_paths
            .iter()
            .filter_map(|path| highlighter.guess_language(Path::new(path)))
            .collect::<HashSet<_>>();
        let active_languages = self.syntax_pack_installs.with(&self.store, |active| {
            active.iter().cloned().collect::<HashSet<_>>()
        });

        let mut seen = HashSet::new();
        let mut warmup_paths = Vec::new();
        for path in paths {
            let Some(language) = highlighter.guess_language(Path::new(path)) else {
                continue;
            };
            if excluded_languages.contains(&language)
                || active_languages.contains(language.name())
                || highlighter.is_parser_available(language)
            {
                continue;
            }
            if seen.insert(language) {
                warmup_paths.push(path.clone());
            }
        }

        (!warmup_paths.is_empty()).then_some(
            SyntaxEffect::EnsureSyntaxPacksForPaths {
                paths: warmup_paths,
            }
            .into(),
        )
    }

    fn handle_syntax_pack_installed(&mut self, language: &str) -> Vec<Effect> {
        let Some(path) = self.workspace.selected_file_path.get(&self.store) else {
            return Vec::new();
        };
        let matches_language = Highlighter::new()
            .resolve_language(&path)
            .is_some_and(|resolved| resolved.name() == language);
        if !matches_language {
            return Vec::new();
        }

        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare => {
                let Some(index) = self.workspace.selected_file_index.get(&self.store) else {
                    return Vec::new();
                };
                self.select_loaded_compare_file(index, false)
            }
            WorkspaceSource::Status => {
                let Some(index) = self.workspace.selected_file_index.get(&self.store) else {
                    return Vec::new();
                };
                self.select_status_item(index, false)
            }
            WorkspaceSource::None => Vec::new(),
        }
    }

    fn activate_status_view(&mut self, reset_scroll: bool) -> Vec<Effect> {
        tracing::debug!(
            reset_scroll,
            pending = self.workspace.status_operation_pending.get(&self.store),
            status_gen = self.workspace.status_generation.get(&self.store),
            status_items_count = self.workspace.status_items.with(&self.store, |i| i.len()),
            "activate_status_view: entered"
        );
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Status);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace_mode.set(&self.store, WorkspaceMode::Ready);
        self.workspace.compare_output.set(&self.store, None);
        self.workspace.active_file_loading.set(&self.store, None);
        let new_files = self
            .workspace
            .status_items
            .with(&self.store, |items| build_status_file_entries(items));
        self.workspace.files.set(&self.store, new_files);
        let next_status_gen = self
            .workspace
            .status_generation
            .get(&self.store)
            .saturating_add(1);
        self.workspace
            .status_generation
            .set(&self.store, next_status_gen);
        self.workspace.sidebar_auto_width.set(&self.store, None);
        self.workspace.used_fallback.set(&self.store, false);
        self.workspace
            .fallback_message
            .set(&self.store, String::new());
        self.workspace.raw_diff_len.set(&self.store, 0);
        if reset_scroll {
            self.file_list.scroll_offset_px.set(&self.store, 0.0);
        }

        let current_path = self.workspace.selected_file_path.get(&self.store);
        let current_scope = self.workspace.selected_status_scope.get(&self.store);
        let (status_syntax_paths, selected_index) =
            self.workspace.status_items.with(&self.store, |items| {
                let paths = items
                    .iter()
                    .map(|item| item.path.clone())
                    .collect::<Vec<_>>();
                if let Some((path, scope)) = current_path.clone().zip(current_scope) {
                    if let Some(idx) = items
                        .iter()
                        .position(|item| item.path == path && item.scope == scope)
                    {
                        return (paths, Some(idx));
                    }
                }
                if let Some(path) = current_path.as_deref() {
                    if let Some(idx) = items.iter().position(|item| item.path == path) {
                        return (paths, Some(idx));
                    }
                }
                (paths, (!items.is_empty()).then_some(0))
            });

        tracing::debug!(
            ?selected_index,
            "activate_status_view: resolved selected_index"
        );
        match selected_index {
            Some(index) => {
                let selected_syntax_paths = status_syntax_paths
                    .get(index)
                    .cloned()
                    .into_iter()
                    .collect::<Vec<_>>();
                let mut effects = self.select_status_item(index, false);
                if let Some(effect) = self.syntax_pack_warmup_effect_for_paths(
                    &status_syntax_paths,
                    &selected_syntax_paths,
                ) {
                    effects.insert(0, effect);
                }
                effects
            }
            None => {
                tracing::debug!("activate_status_view: no selection, clearing pending");
                self.workspace
                    .status_operation_pending
                    .set(&self.store, false);
                self.workspace.selected_file_index.set(&self.store, None);
                self.workspace.selected_file_path.set(&self.store, None);
                self.workspace.selected_status_scope.set(&self.store, None);
                self.workspace.active_file.set(&self.store, None);
                self.workspace.active_file_loading.set(&self.store, None);
                self.editor_clear_document();
                Vec::new()
            }
        }
    }

    fn kickoff_compare(&mut self) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            self.push_error("Open a repository before starting a compare.");
            return Vec::new();
        };

        let mode = self.compare.mode.get(&self.store);
        let left_ref = self.compare.left_ref.get(&self.store);
        let right_ref = self.compare.right_ref.get(&self.store);
        if !compare_refs_are_valid(mode, &left_ref, &right_ref) {
            self.push_error("Provide the required refs for the selected mode.");
            return Vec::new();
        }

        let active_pr = self.github.pull_request.active.get(&self.store);
        let active_pr_still_matches = active_pr.as_ref().is_some_and(|key| {
            self.github.pull_request.cache.with(&self.store, |cache| {
                matches!(
                    cache.get(key).map(|entry| &entry.diff),
                    Some(PrPeekDiff::Ready {
                        left_ref: pr_left,
                        right_ref: pr_right,
                        ..
                    }) if pr_left == &left_ref && pr_right == &right_ref
                )
            })
        });
        if !active_pr_still_matches {
            self.github.pull_request.active.set(&self.store, None);
            self.github
                .pull_request
                .review_composer
                .set(&self.store, ReviewCommentComposerState::default());
            self.review_comment_editor.request_clear();
        }

        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Compare);
        let next_gen = self
            .workspace
            .compare_generation
            .get(&self.store)
            .saturating_add(1);
        self.workspace.compare_generation.set(&self.store, next_gen);
        self.workspace.compare_total_stats.set(&self.store, None);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.workspace
            .compare_stats_hydration_active
            .set(&self.store, false);
        self.workspace.expansions.update(&self.store, |m| m.clear());
        self.clear_overlays();
        self.sync_settings_snapshot();

        // Always delay the reveal — a compare that completes within the
        // threshold should never flash a loading UI, whether or not there
        // was a prior diff on screen. When there IS a prior diff it stays
        // visible during the grace window; when there isn't, the existing
        // workspace state (empty state, ready hint) stays put until the
        // panel reveals.
        let started_at_ms = self.clock_ms;
        let reveal_at_ms = started_at_ms.saturating_add(COMPARE_REVEAL_DELAY_MS);
        let has_prior_state = self
            .workspace
            .files
            .with(&self.store, |files| !files.is_empty())
            || self
                .workspace
                .active_file
                .with(&self.store, |af| af.is_some());

        // If there's no prior diff to preserve, flip workspace_mode to
        // Loading up front so `main_surface` stops rendering the editor /
        // ready-hint — the loading panel will take over at `reveal_at_ms`.
        // When there IS prior state, defer clearing it; the UI gate holds
        // the old diff on screen until the delay elapses or `CompareFinished`
        // replaces it atomically.
        if !has_prior_state {
            self.workspace_mode.set(&self.store, WorkspaceMode::Loading);
            self.workspace.status.set(&self.store, AsyncStatus::Loading);
        }

        let left_label = compare_ref_display_label(&left_ref);
        let right_label = compare_ref_display_label(&right_ref);
        self.compare_progress.set(
            &self.store,
            Some(CompareProgress {
                generation: next_gen,
                phase: ComparePhase::OpeningRepo,
                subject: LoadingSubject::Compare {
                    left_label,
                    right_label,
                },
                started_at_ms,
                reveal_at_ms,
                file_count_total: None,
                files_loaded: 0,
            }),
        );

        let renderer = self.compare.renderer.get(&self.store);
        let layout = self.compare.layout.get(&self.store);
        vec![
            SettingsEffect::SaveSettings(self.settings.clone()).into(),
            CompareEffect::Run(Task {
                generation: next_gen,
                request: CompareRequest {
                    repo_path,
                    spec: CompareSpec {
                        mode,
                        left_ref,
                        right_ref,
                        renderer,
                        layout,
                    },
                    github_token: self.github_access_token.clone(),
                },
            })
            .into(),
        ]
    }

    /// Soft-cancel an in-flight compare. Bumps the generation so any
    /// result that eventually arrives is dropped by the guard, clears the
    /// progress panel, and returns the viewport to the default empty state.
    /// We do not attempt to interrupt the worker mid-flight — git2's
    /// `Diff::new` has no clean cancellation hook, and the wasted work is
    /// bounded by the caller's diff size.
    fn cancel_compare(&mut self) -> Vec<Effect> {
        if self.compare_progress.with(&self.store, |p| p.is_none()) {
            return Vec::new();
        }
        let next_gen = self
            .workspace
            .compare_generation
            .get(&self.store)
            .saturating_add(1);
        self.workspace.compare_generation.set(&self.store, next_gen);
        self.compare_progress.set(&self.store, None);
        self.workspace.active_file_loading.set(&self.store, None);
        // Only revert the workspace mode if kickoff flipped it to Loading
        // (i.e. no prior state was preserved). When the user cancels a
        // re-compare, the old diff is still mounted and should stay visible.
        if self.workspace_mode.get(&self.store) == WorkspaceMode::Loading {
            self.workspace_mode.set(&self.store, WorkspaceMode::Empty);
            self.workspace.status.set(&self.store, AsyncStatus::Idle);
        }
        Vec::new()
    }

    fn handle_compare_progress_update(&mut self, generation: u64, phase: ComparePhase) {
        // Only apply when the progress slot matches the reporter's
        // generation — stale workers silently lose their updates.
        self.compare_progress.update(&self.store, |slot| {
            if let Some(p) = slot.as_mut()
                && p.generation == generation
            {
                // Pull counts out of LoadingFiles so the determinate bar
                // reads directly from durable struct fields (cheaper than
                // pattern-matching in the render path, and lets the total
                // survive the phase transition to PopulatingList).
                if let ComparePhase::LoadingFiles {
                    files_seen,
                    files_total,
                } = phase
                {
                    p.files_loaded = files_seen;
                    if files_total > 0 {
                        p.file_count_total = Some(files_total);
                    }
                }
                p.phase = phase;
            }
        });
    }

    fn show_working_tree(&mut self) -> Vec<Effect> {
        self.compare.left_ref.set(&self.store, "HEAD".to_owned());
        self.compare.right_ref.set(
            &self.store,
            crate::core::vcs::git::service::WORKDIR_REF.to_owned(),
        );
        self.compare.mode.set(&self.store, CompareMode::TwoDot);
        let mut effects = self.persist_settings_effect();
        effects.extend(self.activate_status_view(true));
        effects
    }

    fn swap_refs(&mut self) -> Vec<Effect> {
        let left = self.compare.left_ref.get(&self.store);
        let right = self.compare.right_ref.get(&self.store);
        if left.trim().is_empty()
            || right.trim().is_empty()
            || right == crate::core::vcs::git::service::WORKDIR_REF
            || left == crate::core::vcs::git::service::WORKDIR_REF
        {
            return Vec::new();
        }
        let resolved_left = self.compare.resolved_left.get(&self.store);
        let resolved_right = self.compare.resolved_right.get(&self.store);
        self.compare.left_ref.set(&self.store, right);
        self.compare.right_ref.set(&self.store, left);
        self.compare.resolved_left.set(&self.store, resolved_right);
        self.compare.resolved_right.set(&self.store, resolved_left);
        self.workspace.pre_drill_compare.set(&self.store, None);
        let mut effects = self.persist_settings_effect();
        let has_repo = self.compare.repo_path.with(&self.store, |p| p.is_some());
        let not_loading = self.workspace.status.get(&self.store) != AsyncStatus::Loading;
        let refs_valid = compare_refs_are_valid(
            self.compare.mode.get(&self.store),
            &self.compare.left_ref.get(&self.store),
            &self.compare.right_ref.get(&self.store),
        );
        if has_repo && not_loading && refs_valid {
            effects.extend(self.kickoff_compare());
        }
        effects
    }

    fn persist_settings_effect(&mut self) -> Vec<Effect> {
        self.sync_settings_snapshot();
        vec![SettingsEffect::SaveSettings(self.settings.clone()).into()]
    }

    fn sync_settings_snapshot(&mut self) {
        self.settings.ui_scale_pct = self.clamp_ui_scale_pct(self.settings.ui_scale_pct);
        self.settings.sidebar_width_px = self
            .settings
            .sidebar_width_px
            .map(|width| self.clamp_sidebar_width_px(width));
        self.settings.viewport.wrap_enabled = self.editor.wrap_enabled.get(&self.store);
        self.settings.viewport.wrap_column = self.editor.wrap_column.get(&self.store);
        self.settings.viewport.layout = self.compare.layout.get(&self.store);
        self.settings.last_compare = Some(PersistedCompare {
            repo_path: self.compare.repo_path.get(&self.store),
            left_ref: self.compare.left_ref.get(&self.store),
            right_ref: self.compare.right_ref.get(&self.store),
            mode: self.compare.mode.get(&self.store),
            layout: self.compare.layout.get(&self.store),
            renderer: self.compare.renderer.get(&self.store),
        });
    }

    pub fn ui_scale_factor(&self) -> f32 {
        self.clamp_ui_scale_pct(self.settings.ui_scale_pct) as f32 / DEFAULT_UI_SCALE_PCT as f32
    }

    fn clamp_ui_scale_pct(&self, scale_pct: u16) -> u16 {
        scale_pct.clamp(MIN_UI_SCALE_PCT, MAX_UI_SCALE_PCT)
    }

    fn adjust_ui_scale(&mut self, delta_pct: i16) -> Vec<Effect> {
        let current = i32::from(self.clamp_ui_scale_pct(self.settings.ui_scale_pct));
        let updated = (current + i32::from(delta_pct))
            .clamp(i32::from(MIN_UI_SCALE_PCT), i32::from(MAX_UI_SCALE_PCT))
            as u16;
        if updated == self.settings.ui_scale_pct {
            return Vec::new();
        }
        self.settings.ui_scale_pct = updated;
        self.persist_settings_effect()
    }

    fn clamp_sidebar_width_px(&self, width: u32) -> u32 {
        let min_width = (280.0 * self.ui_scale_factor() * 0.64).round() as u32;
        width.max(min_width.max(120))
    }

    fn set_focus(&mut self, target: Option<FocusTarget>) {
        if target != self.focus.get(&self.store) {
            // Reset cursor to end of the new field
            let len = target
                .and_then(|t| self.with_text_for_focus(t, |s| s.len()))
                .unwrap_or(0);
            self.reset_text_edit(len);
        }
        self.focus.set(&self.store, target);
        self.editor
            .focused
            .set(&self.store, target == Some(FocusTarget::Editor));
    }

    /// Set cursor and anchor to the same offset and refresh the blink timestamp.
    pub(super) fn reset_text_edit(&mut self, offset: usize) {
        self.text_edit.cursor.set(&self.store, offset);
        self.text_edit.anchor.set(&self.store, offset);
        self.text_edit
            .cursor_moved_at_ms
            .set(&self.store, self.clock_ms);
    }

    /// Run `f` against the text string for the given focus target, if it's a text field.
    pub(super) fn with_text_for_focus<R>(
        &self,
        target: FocusTarget,
        f: impl FnOnce(&str) -> R,
    ) -> Option<R> {
        match target {
            FocusTarget::PickerInput => match self.overlays.picker.kind.get(&self.store) {
                PickerKind::Repository | PickerKind::Theme => {
                    Some(self.overlays.picker.query.with(&self.store, |s| f(s)))
                }
                PickerKind::LeftRef => Some(self.compare.left_ref.with(&self.store, |s| f(s))),
                PickerKind::RightRef => Some(self.compare.right_ref.with(&self.store, |s| f(s))),
            },
            FocusTarget::CommandPaletteInput => Some(
                self.overlays
                    .command_palette
                    .query
                    .with(&self.store, |s| f(s)),
            ),
            FocusTarget::SidebarSearch => Some(self.file_list.filter.with(&self.store, |s| f(s))),
            FocusTarget::SearchInput => Some(self.editor.search.query.with(&self.store, |s| f(s))),
            FocusTarget::CommitEditor => None,
            FocusTarget::SettingsOpenAiKey => Some(f(&self.ai_openai_key)),
            FocusTarget::SettingsAnthropicKey => Some(f(&self.ai_anthropic_key)),
            FocusTarget::SettingsSteeringPrompt => None,
            _ => None,
        }
    }

    pub(super) fn ai_key_editable(&self, kind: AiKeyKind) -> bool {
        match kind {
            AiKeyKind::OpenAi => self.ai_openai_key.is_empty() || self.ai_openai_editing,
            AiKeyKind::Anthropic => self.ai_anthropic_key.is_empty() || self.ai_anthropic_editing,
        }
    }

    pub(super) fn with_focused_text<R>(&self, f: impl FnOnce(&str) -> R) -> Option<R> {
        let target = self.focus.get(&self.store)?;
        self.with_text_for_focus(target, f)
    }

    pub(super) fn update_focused_text<R>(&mut self, f: impl FnOnce(&mut String) -> R) -> Option<R> {
        match self.focus.get(&self.store) {
            Some(FocusTarget::PickerInput) => match self.overlays.picker.kind.get(&self.store) {
                PickerKind::Repository | PickerKind::Theme => {
                    let mut out = None;
                    self.overlays
                        .picker
                        .query
                        .update(&self.store, |s| out = Some(f(s)));
                    out
                }
                PickerKind::LeftRef => {
                    let mut out = None;
                    self.compare
                        .left_ref
                        .update(&self.store, |s| out = Some(f(s)));
                    out
                }
                PickerKind::RightRef => {
                    let mut out = None;
                    self.compare
                        .right_ref
                        .update(&self.store, |s| out = Some(f(s)));
                    out
                }
            },
            Some(FocusTarget::CommandPaletteInput) => {
                let mut out = None;
                self.overlays
                    .command_palette
                    .query
                    .update(&self.store, |s| out = Some(f(s)));
                out
            }
            Some(FocusTarget::SidebarSearch) => {
                let mut out = None;
                self.file_list
                    .filter
                    .update(&self.store, |s| out = Some(f(s)));
                out
            }
            Some(FocusTarget::SearchInput) => {
                let mut out = None;
                self.editor
                    .search
                    .query
                    .update(&self.store, |s| out = Some(f(s)));
                out
            }
            Some(FocusTarget::CommitEditor) => None,
            Some(FocusTarget::SettingsOpenAiKey) => {
                if !self.ai_key_editable(AiKeyKind::OpenAi) {
                    return None;
                }
                let result = f(&mut self.ai_openai_key);
                Some(result)
            }
            Some(FocusTarget::SettingsAnthropicKey) => {
                if !self.ai_key_editable(AiKeyKind::Anthropic) {
                    return None;
                }
                let result = f(&mut self.ai_anthropic_key);
                Some(result)
            }
            Some(FocusTarget::SettingsSteeringPrompt) => None,
            _ => None,
        }
    }

    /// Returns true if the current focus target is a text editing field.
    /// Backed by a memo; `focus` writes invalidate it automatically.
    pub fn is_text_focused(&self) -> bool {
        self.text_focused.get(&self.store)
    }

    /// Returns true when the workspace is in `Ready` mode.
    pub fn is_workspace_ready(&self) -> bool {
        self.workspace_mode.get(&self.store) == WorkspaceMode::Ready
    }

    fn touch_cursor(&mut self) {
        self.text_edit
            .cursor_moved_at_ms
            .set(&self.store, self.clock_ms);
    }

    fn clamp_cursor(&mut self) {
        let cursor_now = self.text_edit.cursor.get(&self.store);
        let anchor_now = self.text_edit.anchor.get(&self.store);
        let Some((cursor, anchor)) = self.with_focused_text(|text| {
            let len = text.len();
            let mut cursor = cursor_now.min(len);
            while cursor > 0 && !text.is_char_boundary(cursor) {
                cursor -= 1;
            }
            let mut anchor = anchor_now.min(len);
            while anchor > 0 && !text.is_char_boundary(anchor) {
                anchor -= 1;
            }
            (cursor, anchor)
        }) else {
            return;
        };
        self.text_edit.cursor.set(&self.store, cursor);
        self.text_edit.anchor.set(&self.store, anchor);
    }

    // Text editing methods are in text_edit.rs

    fn update_compare_field(&mut self, field: CompareField, value: String) -> Vec<Effect> {
        self.workspace.pre_drill_compare.set(&self.store, None);
        match field {
            CompareField::Left => {
                self.compare.left_ref.set(&self.store, value);
                self.compare.resolved_left.set(&self.store, None);
            }
            CompareField::Right => {
                self.compare.right_ref.set(&self.store, value);
                self.compare.resolved_right.set(&self.store, None);
            }
        }
        self.auto_select_compare_mode();
        let active_field = self.overlays.ref_picker.active_field.get(&self.store);
        let mut effects = if matches!(self.overlays_top(), Some(OverlaySurface::RefPicker))
            && active_field == field
        {
            self.rebuild_ref_picker(field)
        } else {
            Vec::new()
        };
        effects.extend(self.rebuild_command_palette());
        effects
    }

    fn auto_select_compare_mode(&mut self) {
        let left = self.compare.left_ref.get(&self.store);
        let right = self.compare.right_ref.get(&self.store);
        if left.is_empty() || right.is_empty() {
            return;
        }
        if left == right && right != crate::core::vcs::git::service::WORKDIR_REF {
            self.compare
                .mode
                .set(&self.store, CompareMode::SingleCommit);
            return;
        }
        let is_trunk = |r: &str| matches!(r, "main" | "master" | "develop" | "development");
        if is_trunk(&left) != is_trunk(&right) {
            self.compare.mode.set(&self.store, CompareMode::ThreeDot);
        }
    }

    fn apply_pr_compare(&mut self, left: String, right: String) -> Vec<Effect> {
        let _ = self.update_compare_field(CompareField::Left, left);
        let _ = self.update_compare_field(CompareField::Right, right);
        self.compare.mode.set(&self.store, CompareMode::ThreeDot);
        self.kickoff_compare()
    }

    fn open_repo_picker(&mut self) {
        let scale = self.ui_scale_factor();
        self.overlays
            .picker
            .kind
            .set(&self.store, PickerKind::Repository);
        self.overlays.picker.list.update(&self.store, |l| {
            l.row_height_px = (Sz::ROW * scale).round() as u32;
            l.gap_px = (Sp::XS * scale).round() as u32;
            l.scroll_top_px = 0;
        });
        self.overlays.picker.browse_path.set(&self.store, None);
        self.overlays.picker.selected_index.set(&self.store, 0);

        let has_recents = crate::core::frecency::recent_repo_paths(self.frecency.as_ref(), 1)
            .first()
            .is_some();

        if has_recents {
            self.overlays.picker.query.set(&self.store, String::new());
        } else {
            let home = dirs::home_dir()
                .map(|p| format!("{}/", p.display()))
                .unwrap_or_else(|| "/".to_owned());
            let home_len = home.len();
            self.overlays.picker.query.set(&self.store, home);
            self.reset_text_edit(home_len);
        }

        self.rebuild_repo_picker();
        self.push_overlay(OverlaySurface::RepoPicker, Some(FocusTarget::PickerInput));
    }

    fn open_theme_picker(&mut self) {
        let scale = self.ui_scale_factor();
        self.theme_preview_original
            .set(&self.store, Some(self.settings.theme_name.clone()));
        self.overlays
            .picker
            .kind
            .set(&self.store, PickerKind::Theme);
        self.overlays.picker.query.set(&self.store, String::new());
        self.overlays.picker.list.update(&self.store, |l| {
            l.scroll_top_px = 0;
            l.row_height_px = (Sz::ROW * scale).round() as u32;
            l.gap_px = (Sp::XS * scale).round() as u32;
        });
        let entries = self.build_theme_entries_grouped();
        let selected = entries.iter().position(|e| !e.section_header).unwrap_or(0);
        self.overlays.picker.entries.set(&self.store, entries);
        self.overlays
            .picker
            .selected_index
            .set(&self.store, selected);
        self.push_overlay(OverlaySurface::ThemePicker, Some(FocusTarget::PickerInput));
    }

    fn build_theme_entries_grouped(&self) -> Vec<PickerEntry> {
        use crate::core::themes::ThemeVariant;

        let original = self
            .theme_preview_original
            .get(&self.store)
            .unwrap_or_else(|| self.settings.theme_name.clone());
        let make_entry = |name: &String| PickerEntry {
            label: name.clone(),
            detail: if *name == original {
                "\u{2713}".to_owned()
            } else {
                String::new()
            },
            value: name.clone(),
            highlights: Vec::new(),
            icon: None,
            section_header: false,
        };
        let make_header = |label: &str| PickerEntry {
            label: label.to_owned(),
            detail: String::new(),
            value: String::new(),
            highlights: Vec::new(),
            icon: None,
            section_header: true,
        };

        let mut dual = Vec::new();
        let mut dark = Vec::new();
        let mut light = Vec::new();
        for (i, name) in self.theme_names.iter().enumerate() {
            let variant = self
                .theme_variants
                .get(i)
                .copied()
                .unwrap_or(ThemeVariant::Dark);
            match variant {
                ThemeVariant::Dual => dual.push(make_entry(name)),
                ThemeVariant::Dark => dark.push(make_entry(name)),
                ThemeVariant::Light => light.push(make_entry(name)),
            }
        }

        let mut entries = Vec::with_capacity(dual.len() + dark.len() + light.len() + 3);
        if !dual.is_empty() {
            entries.push(make_header("Dark & Light"));
            entries.extend(dual);
        }
        if !dark.is_empty() {
            entries.push(make_header("Dark"));
            entries.extend(dark);
        }
        if !light.is_empty() {
            entries.push(make_header("Light"));
            entries.extend(light);
        }
        entries
    }

    fn rebuild_theme_picker(&mut self) {
        let query = self
            .overlays
            .picker
            .query
            .with(&self.store, |q| q.trim().to_owned());
        let original = self
            .theme_preview_original
            .get(&self.store)
            .unwrap_or_else(|| self.settings.theme_name.clone());
        let (entries, selected) = if query.is_empty() {
            let entries = self.build_theme_entries_grouped();
            let selected = entries.iter().position(|e| !e.section_header).unwrap_or(0);
            (entries, selected)
        } else {
            let haystack: Vec<&str> = self.theme_names.iter().map(|s| s.as_str()).collect();
            let config = neo_frizbee::Config {
                max_typos: Some(2),
                sort: false,
                ..Default::default()
            };
            let mut matches = neo_frizbee::match_list_indices(&query, &haystack, &config);
            matches.sort_by(|a, b| b.score.cmp(&a.score));
            let entries: Vec<PickerEntry> = matches
                .iter()
                .map(|m| {
                    let name = &self.theme_names[m.index as usize];
                    PickerEntry {
                        label: name.clone(),
                        detail: if *name == *original {
                            "\u{2713}".to_owned()
                        } else {
                            String::new()
                        },
                        value: name.clone(),
                        highlights: highlight_ranges_from_match_indices(name, &m.indices),
                        icon: None,
                        section_header: false,
                    }
                })
                .collect();
            (entries, 0)
        };
        if let Some(entry) = entries.get(selected) {
            if !entry.section_header {
                self.settings.theme_name = entry.value.clone();
            }
        }
        let entry_count = entries.len();
        self.overlays.picker.entries.set(&self.store, entries);
        self.overlays
            .picker
            .selected_index
            .set(&self.store, selected);
        self.overlays.picker.list.update(&self.store, |l| {
            l.viewport_height_px = l.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, entry_count);
            l.scroll_top_px = 0;
        });
    }

    fn open_ref_picker(&mut self, field: CompareField) -> Vec<Effect> {
        let scale = self.ui_scale_factor();
        let already_open = self.overlays_top() == Some(OverlaySurface::RefPicker);
        // Snapshot originals only on first open; switching chips shouldn't
        // refresh the revert baseline.
        if !already_open {
            let left = self.compare.left_ref.get(&self.store);
            let right = self.compare.right_ref.get(&self.store);
            self.overlays
                .ref_picker
                .original_left
                .set(&self.store, left);
            self.overlays
                .ref_picker
                .original_right
                .set(&self.store, right);
        }
        self.overlays
            .ref_picker
            .active_field
            .set(&self.store, field);
        self.overlays.picker.kind.set(
            &self.store,
            match field {
                CompareField::Left => PickerKind::LeftRef,
                CompareField::Right => PickerKind::RightRef,
            },
        );
        self.overlays.picker.selected_index.set(&self.store, 0);
        self.overlays.picker.list.update(&self.store, |l| {
            l.scroll_top_px = 0;
            l.row_height_px = (Sz::ROW * scale).round() as u32;
            l.gap_px = (Sp::XS * scale).round() as u32;
        });
        let effects = self.rebuild_ref_picker(field);
        self.push_overlay(OverlaySurface::RefPicker, Some(FocusTarget::PickerInput));
        // Move cursor to end of the active field's current value so typing
        // continues from where the label ends.
        let len = match field {
            CompareField::Left => self.compare.left_ref.with(&self.store, |s| s.len()),
            CompareField::Right => self.compare.right_ref.with(&self.store, |s| s.len()),
        };
        self.reset_text_edit(len);
        effects
    }

    fn open_command_palette(&mut self) -> Vec<Effect> {
        let scale = self.ui_scale_factor();
        self.overlays.command_palette.list.update(&self.store, |l| {
            l.row_height_px = (Sz::ROW * scale).round() as u32;
            l.gap_px = (Sp::XS * scale).round() as u32;
            l.scroll_top_px = 0;
        });
        let effects = self.rebuild_command_palette();
        self.push_overlay(
            OverlaySurface::CommandPalette,
            Some(FocusTarget::CommandPaletteInput),
        );
        effects
    }

    fn push_overlay(&mut self, surface: OverlaySurface, focus_target: Option<FocusTarget>) {
        if self.overlays_top() == Some(surface) {
            self.set_focus(focus_target);
            return;
        }
        let focus_return = self.focus.get(&self.store);
        self.overlays.stack.update(&self.store, |stack| {
            stack.push(OverlayEntry {
                surface,
                focus_return,
            });
        });
        self.set_focus(focus_target);
    }

    fn pop_overlay(&mut self) {
        let mut popped: Option<OverlayEntry> = None;
        self.overlays.stack.update(&self.store, |stack| {
            popped = stack.pop();
        });
        let Some(entry) = popped else {
            return;
        };
        match entry.surface {
            OverlaySurface::ThemePicker => {
                let original = self.theme_preview_original.get(&self.store);
                self.theme_preview_original.set(&self.store, None);
                if let Some(original) = original {
                    self.settings.theme_name = original;
                }
                self.reset_picker();
            }
            OverlaySurface::RepoPicker | OverlaySurface::RefPicker => {
                self.reset_picker();
            }
            OverlaySurface::CommandPalette => {
                self.reset_command_palette();
            }
            _ => {}
        }
        self.set_focus(entry.focus_return);
    }

    fn move_overlay_selection(&mut self, delta: i32) {
        match self.overlays_top() {
            Some(OverlaySurface::ThemePicker) => {
                let current = self.overlays.picker.selected_index.get(&self.store);
                let (idx, len, value) = self.overlays.picker.entries.with(&self.store, |entries| {
                    let len = entries.len();
                    if len == 0 {
                        return (current, len, None);
                    }
                    let max = len.saturating_sub(1) as i32;
                    let mut idx = (current as i32 + delta).clamp(0, max) as usize;
                    while idx < len && entries[idx].section_header {
                        if delta > 0 {
                            idx = (idx + 1).min(len.saturating_sub(1));
                        } else {
                            if idx == 0 {
                                break;
                            }
                            idx -= 1;
                        }
                    }
                    let value = entries
                        .get(idx)
                        .filter(|e| !e.section_header)
                        .map(|e| e.value.clone());
                    (idx, len, value)
                });
                if len == 0 {
                    return;
                }
                self.overlays.picker.selected_index.set(&self.store, idx);
                self.overlays
                    .picker
                    .list
                    .update(&self.store, |l| l.reveal_index(idx, len));
                if let Some(value) = value {
                    tracing::debug!(theme = %value, "theme preview");
                    self.settings.theme_name = value;
                }
            }
            Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker) => {
                let current = self.overlays.picker.selected_index.get(&self.store);
                let (idx, len) = self.overlays.picker.entries.with(&self.store, |entries| {
                    let len = entries.len();
                    if len == 0 {
                        return (current, len);
                    }
                    let max = len.saturating_sub(1) as i32;
                    let mut idx = (current as i32 + delta).clamp(0, max) as usize;
                    while idx < len && entries[idx].section_header {
                        if delta > 0 {
                            idx = (idx + 1).min(len.saturating_sub(1));
                        } else {
                            if idx == 0 {
                                break;
                            }
                            idx -= 1;
                        }
                    }
                    (idx, len)
                });
                if len == 0 {
                    return;
                }
                self.overlays.picker.selected_index.set(&self.store, idx);
                self.overlays
                    .picker
                    .list
                    .update(&self.store, |l| l.reveal_index(idx, len));
            }
            Some(OverlaySurface::CommandPalette) => {
                let entry_count = self
                    .overlays
                    .command_palette
                    .entries
                    .with(&self.store, |e| e.len());
                let max = entry_count.saturating_sub(1) as i32;
                let current = self
                    .overlays
                    .command_palette
                    .selected_index
                    .get(&self.store);
                let idx = (current as i32 + delta).clamp(0, max.max(0)) as usize;
                self.overlays
                    .command_palette
                    .selected_index
                    .set(&self.store, idx);
                self.overlays
                    .command_palette
                    .list
                    .update(&self.store, |l| l.reveal_index(idx, entry_count));
            }
            _ => {}
        }
    }

    fn select_overlay_entry(&mut self, index: usize) {
        match self.overlays_top() {
            Some(OverlaySurface::ThemePicker) => {
                let (clamped, len, value) =
                    self.overlays.picker.entries.with(&self.store, |entries| {
                        let len = entries.len();
                        let clamped = index.min(len.saturating_sub(1));
                        let value = entries.get(clamped).map(|e| e.value.clone());
                        (clamped, len, value)
                    });
                self.overlays
                    .picker
                    .selected_index
                    .set(&self.store, clamped);
                if let Some(value) = value {
                    self.settings.theme_name = value;
                }
                self.overlays
                    .picker
                    .list
                    .update(&self.store, |l| l.reveal_index(clamped, len));
            }
            Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker) => {
                let (clamped, len, is_header) =
                    self.overlays.picker.entries.with(&self.store, |entries| {
                        let len = entries.len();
                        let clamped = index.min(len.saturating_sub(1));
                        let is_header = entries.get(clamped).map_or(false, |e| e.section_header);
                        (clamped, len, is_header)
                    });
                if is_header {
                    return;
                }
                self.overlays
                    .picker
                    .selected_index
                    .set(&self.store, clamped);
                self.overlays
                    .picker
                    .list
                    .update(&self.store, |l| l.reveal_index(clamped, len));
            }
            Some(OverlaySurface::CommandPalette) => {
                let len = self
                    .overlays
                    .command_palette
                    .entries
                    .with(&self.store, |e| e.len());
                let clamped = index.min(len.saturating_sub(1));
                self.overlays
                    .command_palette
                    .selected_index
                    .set(&self.store, clamped);
                self.overlays
                    .command_palette
                    .list
                    .update(&self.store, |l| l.reveal_index(clamped, len));
            }
            _ => {}
        }
    }

    fn confirm_overlay_selection(&mut self) -> Vec<Effect> {
        match self.overlays_top() {
            Some(OverlaySurface::ThemePicker) => {
                let selected = self.overlays.picker.selected_index.get(&self.store);
                let value = self.overlays.picker.entries.with(&self.store, |entries| {
                    entries.get(selected).map(|e| e.value.clone())
                });
                if let Some(value) = value {
                    tracing::info!(theme = %value, "theme confirmed");
                    self.settings.theme_name = value;
                }
                self.theme_preview_original.set(&self.store, None);
                self.pop_overlay();
                self.persist_settings_effect()
            }
            Some(OverlaySurface::RepoPicker) => self.confirm_repo_picker(),
            Some(OverlaySurface::RefPicker) => {
                let field = self.overlays.ref_picker.active_field.get(&self.store);
                self.confirm_ref_picker(field)
            }
            Some(OverlaySurface::CommandPalette) => self.confirm_command_palette(),
            Some(OverlaySurface::GitHubAuthModal) => {
                if self
                    .github
                    .auth
                    .device_flow
                    .with(&self.store, |opt| opt.is_some())
                {
                    self.apply_action(crate::actions::GitHubAction::OpenDeviceFlowBrowser)
                } else {
                    self.apply_action(crate::actions::GitHubAction::StartGitHubDeviceFlow)
                }
            }
            Some(
                OverlaySurface::KeyboardShortcuts
                | OverlaySurface::CompareMenu
                | OverlaySurface::AccountMenu,
            ) => Vec::new(),
            None => Vec::new(),
        }
    }

    fn confirm_repo_picker(&mut self) -> Vec<Effect> {
        let selected = self.overlays.picker.selected_index.get(&self.store);
        let entry = self
            .overlays
            .picker
            .entries
            .with(&self.store, |entries| entries.get(selected).cloned());

        let Some(entry) = entry else {
            let query = self
                .overlays
                .picker
                .query
                .with(&self.store, |q| q.trim().to_owned());
            if !query.is_empty() {
                let expanded = expand_tilde(&query);
                let path = PathBuf::from(&expanded);
                if path.is_dir() && path.join(".git").exists() {
                    self.pop_overlay();
                    return self.open_repository(path);
                }
                if path.is_dir() {
                    self.navigate_picker_to_dir(&path);
                    return Vec::new();
                }
            }
            return Vec::new();
        };

        if entry.section_header {
            return Vec::new();
        }

        if entry.value.starts_with("open:") {
            let path = PathBuf::from(&entry.value[5..]);
            self.pop_overlay();
            return self.open_repository(path);
        }

        let path = PathBuf::from(&entry.value);

        let browsing = self
            .overlays
            .picker
            .browse_path
            .with(&self.store, |p| p.is_some());
        if browsing {
            if entry.label == ".." {
                self.navigate_picker_to_dir(&path);
                return Vec::new();
            }
            if path.is_dir() && path.join(".git").exists() {
                self.pop_overlay();
                return self.open_repository(path);
            }
            if path.is_dir() {
                self.navigate_picker_to_dir(&path);
                return Vec::new();
            }
            return Vec::new();
        }

        self.pop_overlay();
        self.open_repository(path)
    }

    fn tab_complete_picker_dir(&mut self) {
        if self.overlays.picker.kind.get(&self.store) != PickerKind::Repository {
            return;
        }
        let selected = self.overlays.picker.selected_index.get(&self.store);
        let entry = self
            .overlays
            .picker
            .entries
            .with(&self.store, |entries| entries.get(selected).cloned());
        let Some(entry) = entry else { return };
        if entry.section_header || entry.value.is_empty() {
            return;
        }
        let path = PathBuf::from(&entry.value);
        if path.is_dir() {
            self.navigate_picker_to_dir(&path);
        }
    }

    fn navigate_picker_to_dir(&mut self, path: &Path) {
        let display = path.display().to_string();
        let new_query = if display.ends_with('/') || display.ends_with('\\') {
            display
        } else {
            format!("{}/", display)
        };
        let new_len = new_query.len();
        self.overlays.picker.query.set(&self.store, new_query);
        self.reset_text_edit(new_len);
        self.rebuild_repo_picker();
    }

    fn confirm_ref_picker(&mut self, field: CompareField) -> Vec<Effect> {
        let selected = self.overlays.picker.selected_index.get(&self.store);
        let entry = self
            .overlays
            .picker
            .entries
            .with(&self.store, |entries| entries.get(selected).cloned())
            .or_else(|| {
                let query = match field {
                    CompareField::Left => self
                        .compare
                        .left_ref
                        .with(&self.store, |s| s.trim().to_owned()),
                    CompareField::Right => self
                        .compare
                        .right_ref
                        .with(&self.store, |s| s.trim().to_owned()),
                };
                (!query.is_empty()).then(|| PickerEntry {
                    label: query.clone(),
                    detail: "Use typed ref".to_owned(),
                    value: query.clone(),
                    highlights: vec![(0, query.len())],
                    icon: None,
                    section_header: false,
                })
            });
        let Some(entry) = entry else {
            return Vec::new();
        };
        // Presets apply both refs at once; treat them as an explicit commit.
        if let Some(rest) = entry.value.strip_prefix("@preset:") {
            return self.apply_compare_preset(rest);
        }
        if let Some(ref store) = self.frecency {
            store.record_access(&format!("ref:{}", entry.value));
        }
        let _ = self.update_compare_field(field, entry.value);
        // Auto-advance to the other chip if it's still at its snapshot — the
        // user is likely changing both refs. Only commit when both chips have
        // diverged from their snapshots (or neither, which is a no-op).
        let other = match field {
            CompareField::Left => CompareField::Right,
            CompareField::Right => CompareField::Left,
        };
        let other_current = match other {
            CompareField::Left => self.compare.left_ref.get(&self.store),
            CompareField::Right => self.compare.right_ref.get(&self.store),
        };
        let other_original = match other {
            CompareField::Left => self.overlays.ref_picker.original_left.get(&self.store),
            CompareField::Right => self.overlays.ref_picker.original_right.get(&self.store),
        };
        if other_current == other_original {
            let scale = self.ui_scale_factor();
            self.overlays
                .ref_picker
                .active_field
                .set(&self.store, other);
            self.overlays.picker.kind.set(
                &self.store,
                match other {
                    CompareField::Left => PickerKind::LeftRef,
                    CompareField::Right => PickerKind::RightRef,
                },
            );
            self.overlays.picker.selected_index.set(&self.store, 0);
            self.overlays.picker.list.update(&self.store, |l| {
                l.scroll_top_px = 0;
                l.row_height_px = (Sz::ROW * scale).round() as u32;
                l.gap_px = (Sp::XS * scale).round() as u32;
            });
            let effects = self.rebuild_ref_picker(other);
            let len = match other {
                CompareField::Left => self.compare.left_ref.with(&self.store, |s| s.len()),
                CompareField::Right => self.compare.right_ref.with(&self.store, |s| s.len()),
            };
            self.reset_text_edit(len);
            return effects;
        }
        // Both chips changed — commit.
        self.commit_ref_picker()
    }

    fn commit_ref_picker(&mut self) -> Vec<Effect> {
        let original_left = self.overlays.ref_picker.original_left.get(&self.store);
        let original_right = self.overlays.ref_picker.original_right.get(&self.store);
        let current_left = self.compare.left_ref.get(&self.store);
        let current_right = self.compare.right_ref.get(&self.store);
        let changed = current_left != original_left || current_right != original_right;
        self.pop_overlay();
        let mut effects = self.persist_settings_effect();
        if !changed {
            return effects;
        }
        let has_repo = self.compare.repo_path.with(&self.store, |p| p.is_some());
        let not_loading = self.workspace.status.get(&self.store) != AsyncStatus::Loading;
        let refs_valid = compare_refs_are_valid(
            self.compare.mode.get(&self.store),
            &current_left,
            &current_right,
        );
        if has_repo && not_loading && refs_valid {
            effects.extend(self.kickoff_compare());
        }
        effects
    }

    fn cancel_ref_picker(&mut self) -> Vec<Effect> {
        let left = self.overlays.ref_picker.original_left.get(&self.store);
        let right = self.overlays.ref_picker.original_right.get(&self.store);
        self.compare.left_ref.set(&self.store, left);
        self.compare.right_ref.set(&self.store, right);
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        self.pop_overlay();
        Vec::new()
    }

    fn set_active_ref_field(&mut self, field: CompareField) -> Vec<Effect> {
        if self.overlays_top() != Some(OverlaySurface::RefPicker) {
            return Vec::new();
        }
        let scale = self.ui_scale_factor();
        self.overlays
            .ref_picker
            .active_field
            .set(&self.store, field);
        self.overlays.picker.kind.set(
            &self.store,
            match field {
                CompareField::Left => PickerKind::LeftRef,
                CompareField::Right => PickerKind::RightRef,
            },
        );
        self.overlays.picker.selected_index.set(&self.store, 0);
        self.overlays.picker.list.update(&self.store, |l| {
            l.scroll_top_px = 0;
            l.row_height_px = (Sz::ROW * scale).round() as u32;
            l.gap_px = (Sp::XS * scale).round() as u32;
        });
        let effects = self.rebuild_ref_picker(field);
        let len = match field {
            CompareField::Left => self.compare.left_ref.with(&self.store, |s| s.len()),
            CompareField::Right => self.compare.right_ref.with(&self.store, |s| s.len()),
        };
        self.reset_text_edit(len);
        effects
    }

    fn swap_draft_refs(&mut self) -> Vec<Effect> {
        if self.overlays_top() != Some(OverlaySurface::RefPicker) {
            return Vec::new();
        }
        let left = self.compare.left_ref.get(&self.store);
        let right = self.compare.right_ref.get(&self.store);
        self.compare.left_ref.set(&self.store, right);
        self.compare.right_ref.set(&self.store, left);
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        // Re-sync the search input to the active chip's new value.
        let field = self.overlays.ref_picker.active_field.get(&self.store);
        let len = match field {
            CompareField::Left => self.compare.left_ref.with(&self.store, |s| s.len()),
            CompareField::Right => self.compare.right_ref.with(&self.store, |s| s.len()),
        };
        self.reset_text_edit(len);
        self.rebuild_ref_picker(field)
    }

    fn apply_compare_preset(&mut self, preset: &str) -> Vec<Effect> {
        let parts: Vec<&str> = preset.splitn(3, ':').collect();
        if parts.len() != 3 {
            return Vec::new();
        }
        let (left, right, mode_str) = (parts[0], parts[1], parts[2]);
        let mode = match mode_str {
            "commit" => CompareMode::SingleCommit,
            "diff" => CompareMode::TwoDot,
            _ => CompareMode::ThreeDot,
        };
        self.workspace.pre_drill_compare.set(&self.store, None);
        self.compare.left_ref.set(&self.store, left.to_owned());
        self.compare.right_ref.set(&self.store, right.to_owned());
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        self.compare.mode.set(&self.store, mode);
        self.pop_overlay();
        let mut effects = self.persist_settings_effect();
        if self.compare.repo_path.with(&self.store, |p| p.is_some()) {
            effects.extend(self.kickoff_compare());
        }
        effects
    }

    fn confirm_command_palette(&mut self) -> Vec<Effect> {
        let selected = self
            .overlays
            .command_palette
            .selected_index
            .get(&self.store);
        let Some(entry) = self
            .overlays
            .command_palette
            .entries
            .with(&self.store, |entries| entries.get(selected).cloned())
        else {
            return Vec::new();
        };
        if entry.disabled {
            return Vec::new();
        }
        self.clear_overlays();
        match entry.kind {
            PaletteEntryKind::Command(command) => match command {
                PaletteCommand::OpenRepoPicker => {
                    self.open_repo_picker();
                    Vec::new()
                }
                PaletteCommand::OpenGitHubAuthModal => {
                    self.push_overlay(
                        OverlaySurface::GitHubAuthModal,
                        Some(FocusTarget::AuthPrimaryAction),
                    );
                    Vec::new()
                }
                PaletteCommand::FocusFileList => {
                    self.set_focus(Some(FocusTarget::FileList));
                    Vec::new()
                }
                PaletteCommand::FocusViewport => {
                    self.set_focus(Some(FocusTarget::Editor));
                    Vec::new()
                }
                PaletteCommand::ToggleWrap => {
                    self.apply_action(crate::actions::SettingsAction::ToggleWrap)
                }
                PaletteCommand::ToggleThemeMode => {
                    self.apply_action(crate::actions::SettingsAction::ToggleThemeMode)
                }
                PaletteCommand::SetLayout(layout) => {
                    self.apply_action(crate::actions::CompareAction::SetLayoutMode(layout))
                }
                PaletteCommand::ChangeTheme => {
                    self.apply_action(crate::actions::SettingsAction::OpenThemePicker)
                }
                PaletteCommand::SetTheme(name) => {
                    self.apply_action(crate::actions::SettingsAction::SetThemeName(name))
                }
                PaletteCommand::FetchOrigin => self.apply_action(
                    crate::actions::RepositoryAction::FetchRemote("origin".to_owned()),
                ),
                PaletteCommand::FetchAllRemotes => {
                    self.apply_action(crate::actions::RepositoryAction::FetchAllRemotes)
                }
                PaletteCommand::PushCurrentBranch => {
                    self.apply_action(crate::actions::RepositoryAction::PushCurrentBranch {
                        force_with_lease: false,
                    })
                }
                PaletteCommand::PushCurrentBranchForceWithLease => {
                    self.apply_action(crate::actions::RepositoryAction::PushCurrentBranch {
                        force_with_lease: true,
                    })
                }
                PaletteCommand::PullCurrentBranch => {
                    self.apply_action(crate::actions::RepositoryAction::PullCurrentBranch)
                }
                PaletteCommand::OpenSettings => {
                    self.apply_action(crate::actions::SettingsAction::OpenSettings)
                }
            },
            PaletteEntryKind::File(index) => self.select_file(index, true),
            PaletteEntryKind::Repo(path) => self.open_repository(path),
            PaletteEntryKind::Ref(field, value) => {
                let _ = self.update_compare_field(field, value);
                self.persist_settings_effect()
            }
            PaletteEntryKind::PullRequest(key) => self.confirm_pr_entry(key),
        }
    }

    fn confirm_pr_entry(&mut self, key: PrKey) -> Vec<Effect> {
        if self.compare.repo_path.with(&self.store, |p| p.is_none()) {
            self.push_error("Open a repository before loading a pull request.");
            return Vec::new();
        }
        let diff_state = self
            .github
            .pull_request
            .cache
            .with(&self.store, |c| c.get(&key).map(|e| e.diff.clone()));
        match diff_state {
            Some(PrPeekDiff::Ready {
                left_ref,
                right_ref,
                ..
            }) => {
                self.github
                    .pull_request
                    .pending_confirm
                    .set(&self.store, None);
                self.github.pull_request.active.set(&self.store, Some(key));
                self.apply_pr_compare(left_ref, right_ref)
            }
            Some(PrPeekDiff::Loading) | Some(PrPeekDiff::Idle) => {
                self.github
                    .pull_request
                    .pending_confirm
                    .set(&self.store, Some(key.clone()));
                self.push_info(&format!("Preparing PR #{}\u{2026}", key.2));
                Vec::new()
            }
            Some(PrPeekDiff::Failed(message)) => {
                self.push_error(&message);
                Vec::new()
            }
            None => {
                self.push_error("Pull request not available.");
                Vec::new()
            }
        }
    }

    fn rebuild_repo_picker(&mut self) {
        let query = self.overlays.picker.query.with(&self.store, |q| q.clone());
        let trimmed = query.trim();

        if query_looks_like_path(trimmed) {
            self.rebuild_repo_picker_browse(trimmed);
        } else {
            self.overlays.picker.browse_path.set(&self.store, None);
            self.rebuild_repo_picker_recent(trimmed);
        }

        let current_selected = self.overlays.picker.selected_index.get(&self.store);
        let (entry_count, new_selected) =
            self.overlays.picker.entries.with(&self.store, |entries| {
                let entry_count = entries.len();
                let new_selected = if entries.is_empty() {
                    0
                } else {
                    let first_selectable =
                        entries.iter().position(|e| !e.section_header).unwrap_or(0);
                    current_selected
                        .max(first_selectable)
                        .min(entries.len().saturating_sub(1))
                };
                (entry_count, new_selected)
            });
        self.overlays
            .picker
            .selected_index
            .set(&self.store, new_selected);
        self.overlays.picker.list.update(&self.store, |l| {
            l.viewport_height_px = l.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, entry_count);
            l.clamp_scroll(entry_count);
        });
    }

    fn rebuild_repo_picker_recent(&mut self, query: &str) {
        let mut entries = Vec::new();

        let all_repos = crate::core::frecency::recent_repo_paths(self.frecency.as_ref(), 20);

        let mut seen = HashSet::new();
        let mut unique_repos = Vec::new();
        for repo in &all_repos {
            if seen.insert(repo.clone()) {
                unique_repos.push(repo.clone());
            }
        }

        if !unique_repos.is_empty() {
            entries.push(PickerEntry {
                label: "Recent".to_owned(),
                detail: String::new(),
                value: String::new(),
                highlights: Vec::new(),
                icon: None,
                section_header: true,
            });
        }

        if query.is_empty() {
            for repo in &unique_repos {
                let display = repo.display().to_string();
                let is_git = repo.join(".git").exists();
                entries.push(PickerEntry {
                    label: repo
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(&display)
                        .to_owned(),
                    detail: display.clone(),
                    value: repo.display().to_string(),
                    highlights: Vec::new(),
                    icon: Some(if is_git {
                        lucide::FOLDER_GIT
                    } else {
                        lucide::FOLDER
                    }),
                    section_header: false,
                });
            }
        } else {
            let haystack: Vec<String> = unique_repos
                .iter()
                .map(|r| r.display().to_string())
                .collect();
            let haystack_refs: Vec<&str> = haystack.iter().map(|s| s.as_str()).collect();
            let config = neo_frizbee::Config {
                max_typos: Some(2),
                sort: false,
                ..Default::default()
            };
            let mut matches = neo_frizbee::match_list_indices(query, &haystack_refs, &config);
            matches.sort_by(|a, b| b.score.cmp(&a.score));
            if matches.is_empty() {
                entries.clear();
            }
            for m in matches {
                let repo = &unique_repos[m.index as usize];
                let display = &haystack[m.index as usize];
                let label = repo
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(display)
                    .to_owned();
                let highlights =
                    highlight_ranges_for_visible_match(query, &label, &m.indices, &config);
                let is_git = repo.join(".git").exists();
                entries.push(PickerEntry {
                    label,
                    detail: display.clone(),
                    value: repo.display().to_string(),
                    highlights,
                    icon: Some(if is_git {
                        lucide::FOLDER_GIT
                    } else {
                        lucide::FOLDER
                    }),
                    section_header: false,
                });
            }
        }
        self.overlays.picker.entries.set(&self.store, entries);
    }

    fn rebuild_repo_picker_browse(&mut self, query: &str) {
        let expanded = expand_tilde(query);
        let (dir_path, filter) = split_browse_query(&expanded);

        let dir = PathBuf::from(&dir_path);
        if !dir.is_dir() {
            self.overlays.picker.browse_path.set(&self.store, None);
            self.overlays.picker.entries.set(&self.store, Vec::new());
            return;
        }

        self.overlays
            .picker
            .browse_path
            .set(&self.store, Some(dir.clone()));

        let mut entries = Vec::new();

        if dir.join(".git").exists() {
            entries.push(PickerEntry {
                label: "open this directory".to_owned(),
                detail: String::new(),
                value: format!("open:{}", dir.display()),
                icon: Some(lucide::CORNER_UP_LEFT),
                highlights: Vec::new(),
                section_header: false,
            });
        }

        if dir.parent().is_some() {
            entries.push(PickerEntry {
                label: "..".to_owned(),
                detail: String::new(),
                value: dir
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                icon: Some(lucide::CORNER_UP_LEFT),
                highlights: Vec::new(),
                section_header: false,
            });
        }

        let mut dirs: Vec<(String, PathBuf, bool)> = Vec::new();
        if let Ok(read) = std::fs::read_dir(&dir) {
            for entry in read.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_str().unwrap_or_default().to_owned();
                if name.starts_with('.') {
                    continue;
                }
                let is_git = path.join(".git").exists();
                dirs.push((name, path, is_git));
            }
        }

        dirs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

        if filter.is_empty() {
            for (name, path, is_git) in &dirs {
                entries.push(PickerEntry {
                    label: name.clone(),
                    detail: String::new(),
                    value: path.display().to_string(),
                    highlights: Vec::new(),
                    icon: Some(if *is_git {
                        lucide::FOLDER_GIT
                    } else {
                        lucide::FOLDER
                    }),
                    section_header: false,
                });
            }
        } else {
            let haystack: Vec<&str> = dirs.iter().map(|(n, _, _)| n.as_str()).collect();
            let config = neo_frizbee::Config {
                max_typos: Some(1),
                sort: false,
                ..Default::default()
            };
            let mut matches = neo_frizbee::match_list_indices(filter, &haystack, &config);
            matches.sort_by(|a, b| b.score.cmp(&a.score));
            for m in matches {
                let (name, path, is_git) = &dirs[m.index as usize];
                entries.push(PickerEntry {
                    label: name.clone(),
                    detail: String::new(),
                    value: path.display().to_string(),
                    highlights: highlight_ranges_from_match_indices(name, &m.indices),
                    icon: Some(if *is_git {
                        lucide::FOLDER_GIT
                    } else {
                        lucide::FOLDER
                    }),
                    section_header: false,
                });
            }
        }

        self.overlays.picker.entries.set(&self.store, entries);
    }

    fn rebuild_ref_picker(&mut self, field: CompareField) -> Vec<Effect> {
        let query_owned = match field {
            CompareField::Left => self
                .compare
                .left_ref
                .with(&self.store, |s| s.trim().to_owned()),
            CompareField::Right => self
                .compare
                .right_ref
                .with(&self.store, |s| s.trim().to_owned()),
        };
        let query = query_owned.as_str();
        let mut seen = HashSet::new();

        struct RefCandidate {
            search_text: String,
            label: String,
            detail: String,
            value: String,
            ordinal: usize,
        }

        let mut all_candidates = Vec::new();
        let mut ordinal = 0_usize;

        seen.insert("@workdir".to_owned());

        let mut push = |search_text: String, label: String, detail: String, value: String| {
            if !seen.insert(value.clone()) {
                return;
            }
            all_candidates.push(RefCandidate {
                search_text,
                label,
                detail,
                value,
                ordinal,
            });
            ordinal += 1;
        };

        let branches = self.repository.branches.get(&self.store);
        let tags = self.repository.tags.get(&self.store);
        let commits = self.repository.commits.get(&self.store);

        for branch in &branches {
            let scope = if branch.is_remote {
                "Remote branch"
            } else {
                "Branch"
            };
            let mut detail = scope.to_owned();
            if branch.is_head {
                detail.push_str(" \u{2022} HEAD");
            }
            let label = branch.name.clone();
            push(format!("{label} {detail}"), label.clone(), detail, label);
        }

        for tag in &tags {
            let label = tag.name.clone();
            push(
                format!("{label} Tag"),
                label.clone(),
                "Tag".to_owned(),
                label,
            );
        }

        for commit in &commits {
            let label = commit.short_oid.clone();
            push(
                format!("{label} {} {}", commit.summary, commit.oid),
                label,
                commit.summary.clone(),
                commit.oid.clone(),
            );
        }

        let mut needs_resolve = false;

        if query.is_empty() {
            let mut entries = vec![PickerEntry {
                label: "@workdir".to_owned(),
                detail: "Uncommitted changes on disk".to_owned(),
                value: "@workdir".to_owned(),
                highlights: Vec::new(),
                icon: None,
                section_header: false,
            }];
            entries.extend(all_candidates.into_iter().take(10).map(|c| PickerEntry {
                label: c.label,
                detail: c.detail,
                value: c.value,
                highlights: Vec::new(),
                icon: None,
                section_header: false,
            }));
            self.overlays.picker.entries.set(&self.store, entries);
        } else {
            let haystack: Vec<&str> = all_candidates
                .iter()
                .map(|c| c.search_text.as_str())
                .collect();
            let config = neo_frizbee::Config {
                max_typos: Some(2),
                sort: false,
                ..Default::default()
            };
            let matches = neo_frizbee::match_list_indices(query, &haystack, &config);
            let mut scored: Vec<_> = matches
                .into_iter()
                .map(|m| {
                    let c = &all_candidates[m.index as usize];
                    (
                        m.score,
                        c.ordinal,
                        PickerEntry {
                            label: c.label.clone(),
                            detail: c.detail.clone(),
                            value: c.value.clone(),
                            highlights: highlight_ranges_for_visible_match(
                                query, &c.label, &m.indices, &config,
                            ),
                            icon: None,
                            section_header: false,
                        },
                    )
                })
                .collect();
            scored.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then(a.1.cmp(&b.1))
                    .then(a.2.label.cmp(&b.2.label))
            });
            let mut entries = vec![PickerEntry {
                label: "@workdir".to_owned(),
                detail: "Uncommitted changes on disk".to_owned(),
                value: "@workdir".to_owned(),
                highlights: Vec::new(),
                icon: None,
                section_header: false,
            }];
            entries.extend(scored.into_iter().map(|(_, _, entry)| entry).take(10));
            if !entries.iter().any(|entry| entry.value == query) {
                entries.insert(
                    0,
                    PickerEntry {
                        label: query.to_owned(),
                        detail: "Resolving\u{2026}".to_owned(),
                        value: query.to_owned(),
                        highlights: vec![(0, query.len())],
                        icon: None,
                        section_header: false,
                    },
                );
                needs_resolve = true;
            }
            self.overlays.picker.entries.set(&self.store, entries);
        }

        self.overlays.picker.entries.update(&self.store, |e| {
            e.truncate(10);
        });
        let entry_count = self.overlays.picker.entries.with(&self.store, |e| e.len());
        let current_selected = self.overlays.picker.selected_index.get(&self.store);
        self.overlays.picker.selected_index.set(
            &self.store,
            current_selected.min(entry_count.saturating_sub(1)),
        );
        self.overlays.picker.list.update(&self.store, |l| {
            l.viewport_height_px = l.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, entry_count);
            l.clamp_scroll(entry_count);
        });

        if needs_resolve {
            if let Some(repo_path) = self.compare.repo_path.get(&self.store) {
                let new_gen = self.overlays.picker.ref_resolve_generation.get(&self.store) + 1;
                self.overlays
                    .picker
                    .ref_resolve_generation
                    .set(&self.store, new_gen);
                return vec![
                    CompareEffect::ResolveRef {
                        repo_path,
                        query: query.to_owned(),
                        generation: new_gen,
                    }
                    .into(),
                ];
            }
        }
        Vec::new()
    }

    fn rebuild_command_palette_if_open(&mut self) -> Vec<Effect> {
        if self.overlays_top() == Some(OverlaySurface::CommandPalette) {
            self.rebuild_command_palette()
        } else {
            Vec::new()
        }
    }

    fn rebuild_command_palette(&mut self) -> Vec<Effect> {
        let query_owned = self
            .overlays
            .command_palette
            .query
            .with(&self.store, |q| q.trim().to_owned());
        let query = query_owned.as_str();

        let mut out_effects = Vec::new();
        let mut pr_entry: Option<PaletteEntry> = None;

        if let Some(parsed) = crate::core::vcs::github::parse_pr_url(query) {
            let key: PrKey = (parsed.owner.clone(), parsed.repo.clone(), parsed.number);
            let token = self.github_access_token.clone();
            let repo_path = self.compare.repo_path.get(&self.store);

            let already_cached = self
                .github
                .pull_request
                .cache
                .with(&self.store, |c| c.contains_key(&key));
            if !already_cached {
                self.github.pull_request.cache.update(&self.store, |c| {
                    c.insert(
                        key.clone(),
                        PrCacheEntry {
                            meta: PrPeekMeta::Loading,
                            diff: PrPeekDiff::Idle,
                            last_peek_ms: self.clock_ms,
                        },
                    );
                });
                out_effects.push(
                    GitHubEffect::PeekPullRequest {
                        owner: parsed.owner.clone(),
                        repo: parsed.repo.clone(),
                        number: parsed.number,
                        github_token: token.clone(),
                    }
                    .into(),
                );
            }

            // Speculative diff load — kick off as soon as we know the key, provided
            // a repo is open. Dedupe via the cache's diff state.
            if let Some(repo_path) = repo_path.clone() {
                let diff_idle = self.github.pull_request.cache.with(&self.store, |c| {
                    matches!(c.get(&key).map(|e| &e.diff), Some(PrPeekDiff::Idle) | None)
                });
                if diff_idle {
                    self.github.pull_request.cache.update(&self.store, |c| {
                        if let Some(e) = c.get_mut(&key) {
                            e.diff = PrPeekDiff::Loading;
                        }
                    });
                    let url = format!(
                        "https://github.com/{}/{}/pull/{}",
                        parsed.owner, parsed.repo, parsed.number
                    );
                    out_effects.push(
                        GitHubEffect::LoadPullRequest {
                            url,
                            repo_path,
                            github_token: token,
                        }
                        .into(),
                    );
                }
            }

            pr_entry = Some(build_pr_palette_entry(
                &self.github.pull_request.cache.get(&self.store),
                &key,
                repo_path.is_some(),
            ));
        }

        struct PaletteCandidate {
            search_text: String,
            label: String,
            detail: String,
            kind: PaletteEntryKind,
        }

        let mut all_candidates = Vec::new();

        for (label, detail, command) in [
            (
                "Choose Repository".to_owned(),
                "Open repository picker".to_owned(),
                PaletteCommand::OpenRepoPicker,
            ),
            (
                "GitHub Sign In".to_owned(),
                "Start device flow".to_owned(),
                PaletteCommand::OpenGitHubAuthModal,
            ),
            (
                "Focus File List".to_owned(),
                "Move keyboard focus to sidebar".to_owned(),
                PaletteCommand::FocusFileList,
            ),
            (
                "Focus Diff Viewport".to_owned(),
                "Move keyboard focus to editor".to_owned(),
                PaletteCommand::FocusViewport,
            ),
            (
                "Toggle Wrap".to_owned(),
                "Enable or disable line wrapping".to_owned(),
                PaletteCommand::ToggleWrap,
            ),
            (
                "Toggle Theme".to_owned(),
                "Switch light and dark mode".to_owned(),
                PaletteCommand::ToggleThemeMode,
            ),
            (
                "Change Theme".to_owned(),
                "Browse and preview color themes".to_owned(),
                PaletteCommand::ChangeTheme,
            ),
            (
                "Use Unified Layout".to_owned(),
                "Set unified diff mode".to_owned(),
                PaletteCommand::SetLayout(LayoutMode::Unified),
            ),
            (
                "Use Split Layout".to_owned(),
                "Set side-by-side diff mode".to_owned(),
                PaletteCommand::SetLayout(LayoutMode::Split),
            ),
            (
                "Fetch origin".to_owned(),
                "Update tracking branches from origin".to_owned(),
                PaletteCommand::FetchOrigin,
            ),
            (
                "Fetch all remotes".to_owned(),
                "Update tracking branches from every configured remote".to_owned(),
                PaletteCommand::FetchAllRemotes,
            ),
            (
                "Pull current branch".to_owned(),
                "Fast-forward the current branch from its upstream".to_owned(),
                PaletteCommand::PullCurrentBranch,
            ),
            (
                "Push current branch".to_owned(),
                "Push the current branch to its upstream".to_owned(),
                PaletteCommand::PushCurrentBranch,
            ),
            (
                "Push current branch (force with lease)".to_owned(),
                "Force-push the current branch; refuses if upstream moved".to_owned(),
                PaletteCommand::PushCurrentBranchForceWithLease,
            ),
            (
                "Open Settings".to_owned(),
                "Configure appearance, editor, and behavior".to_owned(),
                PaletteCommand::OpenSettings,
            ),
        ] {
            let search_text = format!("{label} {detail}");
            all_candidates.push(PaletteCandidate {
                search_text,
                label,
                detail,
                kind: PaletteEntryKind::Command(command),
            });
        }

        let workspace_files = self.workspace.files.get(&self.store);
        for (index, file) in workspace_files.iter().enumerate() {
            let detail = format!(
                "File \u{2022} {} \u{2022} +{} -{}",
                file.status, file.additions, file.deletions
            );
            let search_text = format!("{} {detail}", file.path);
            all_candidates.push(PaletteCandidate {
                search_text,
                label: file.path.clone(),
                detail,
                kind: PaletteEntryKind::File(index),
            });
        }

        let palette_repos = crate::core::frecency::recent_repo_paths(self.frecency.as_ref(), 10);
        for repo in &palette_repos {
            let repo_name = repo
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|n| *n != ".")
                .map(str::to_owned)
                .unwrap_or_else(|| repo.display().to_string());
            let detail = repo.display().to_string();
            let search_text = format!("{repo_name} {detail}");
            all_candidates.push(PaletteCandidate {
                search_text,
                label: repo_name,
                detail,
                kind: PaletteEntryKind::Repo(repo.clone()),
            });
        }

        let repo_branches = self.repository.branches.get(&self.store);
        for branch in &repo_branches {
            let search_text = format!("{} Branch", branch.name);
            all_candidates.push(PaletteCandidate {
                search_text,
                label: branch.name.clone(),
                detail: "Branch".to_owned(),
                kind: PaletteEntryKind::Ref(CompareField::Left, branch.name.clone()),
            });
        }

        let mut entries: Vec<PaletteEntry>;
        if query.is_empty() {
            entries = all_candidates
                .into_iter()
                .map(|c| PaletteEntry {
                    label: c.label,
                    detail: c.detail,
                    kind: c.kind,
                    highlights: Vec::new(),
                    rhs: None,
                    disabled: false,
                })
                .collect();
        } else {
            let haystack: Vec<&str> = all_candidates
                .iter()
                .map(|c| c.search_text.as_str())
                .collect();
            let config = neo_frizbee::Config {
                max_typos: Some(2),
                sort: false,
                ..Default::default()
            };
            let matches = neo_frizbee::match_list_indices(query, &haystack, &config);
            let mut scored: Vec<_> = matches
                .into_iter()
                .map(|m| {
                    let c = &all_candidates[m.index as usize];
                    (
                        m.score,
                        PaletteEntry {
                            label: c.label.clone(),
                            detail: c.detail.clone(),
                            kind: c.kind.clone(),
                            highlights: highlight_ranges_for_visible_match(
                                query, &c.label, &m.indices, &config,
                            ),
                            rhs: None,
                            disabled: false,
                        },
                    )
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.label.cmp(&b.1.label)));
            entries = scored.into_iter().map(|(_, e)| e).collect();
        }
        if let Some(pr) = pr_entry {
            entries.insert(0, pr);
        }
        entries.truncate(18);
        let entry_count = entries.len();
        self.overlays
            .command_palette
            .entries
            .set(&self.store, entries);
        let current_selected = self
            .overlays
            .command_palette
            .selected_index
            .get(&self.store);
        self.overlays.command_palette.selected_index.set(
            &self.store,
            current_selected.min(entry_count.saturating_sub(1)),
        );
        self.overlays.command_palette.list.update(&self.store, |l| {
            l.viewport_height_px = l.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, entry_count);
            l.clamp_scroll(entry_count);
        });
        out_effects
    }

    fn shift_loaded_file(&mut self, delta: isize) -> Vec<Effect> {
        let file_count = self.workspace.files.with(&self.store, |f| f.len());
        if file_count == 0 {
            return Vec::new();
        }
        let current = self
            .workspace
            .selected_file_index
            .get(&self.store)
            .unwrap_or(0);
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current
                .saturating_add(delta as usize)
                .min(file_count.saturating_sub(1))
        };
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare => self.select_compare_file(next, true),
            WorkspaceSource::Status => self.select_status_item(next, true),
            WorkspaceSource::None => Vec::new(),
        }
    }

    fn select_file(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare => self.select_compare_file(index, reveal),
            WorkspaceSource::Status => self.select_status_item(index, reveal),
            WorkspaceSource::None => {
                self.startup.preferred_file_index = Some(index);
                Vec::new()
            }
        }
    }

    fn select_compare_file(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
        let Some(entry) = self
            .workspace
            .files
            .with(&self.store, |files| files.get(index).cloned())
        else {
            self.push_error("Selected file index is out of range.");
            return Vec::new();
        };

        if !self.compare_file_is_large(index) {
            let mut effects =
                vec![SyntaxEffect::EnsureSyntaxPackForPath { path: entry.path }.into()];
            effects.extend(self.select_loaded_compare_file(index, reveal));
            return effects;
        }

        // If we're mid-compare (first file selection post-CompareFinished),
        // flip the phase so the progress panel reports "Preparing first
        // file…". Subsequent selections don't touch compare_progress.
        self.compare_progress.update(&self.store, |slot| {
            if let Some(p) = slot.as_mut() {
                p.phase = ComparePhase::RenderingFirstFile;
            }
        });

        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            self.push_error("Open a repository before selecting a compare file.");
            return Vec::new();
        };
        let deferred_file = self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .and_then(|output| output.carbon.files.get(index))
                .filter(|file| file.is_partial && file.hunks.is_empty())
                .cloned()
        });

        self.workspace
            .selected_file_index
            .set(&self.store, Some(index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(entry.path.clone()));
        self.workspace.selected_status_scope.set(&self.store, None);
        self.workspace.active_file.set(&self.store, None);
        self.workspace.active_file_loading.set(
            &self.store,
            Some(ActiveFileLoading {
                index,
                path: entry.path.clone(),
            }),
        );
        self.editor_clear_document();
        self.file_list.hovered_index.set(&self.store, Some(index));
        if reveal {
            self.reveal_file_list_row(index);
        }

        vec![
            SyntaxEffect::EnsureSyntaxPackForPath {
                path: entry.path.clone(),
            }
            .into(),
            CompareEffect::LoadFile(Task {
                generation: self.workspace.compare_generation.get(&self.store),
                request: CompareFileRequest {
                    repo_path,
                    spec: CompareSpec {
                        mode: self.compare.mode.get(&self.store),
                        left_ref: self.compare.left_ref.get(&self.store),
                        right_ref: self.compare.right_ref.get(&self.store),
                        renderer: self.compare.renderer.get(&self.store),
                        layout: self.compare.layout.get(&self.store),
                    },
                    path: entry.path,
                    index,
                    deferred_file,
                },
            })
            .into(),
        ]
    }

    #[profiling::function]
    fn select_loaded_compare_file(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
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
        self.request_active_file_syntax_effect()
            .into_iter()
            .collect()
    }

    fn reveal_file_list_row(&mut self, index: usize) {
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

    fn select_status_item(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
        let Some(item) = self
            .workspace
            .status_items
            .with(&self.store, |items| items.get(index).cloned())
        else {
            tracing::warn!(
                index,
                "select_status_item: index out of range, returning empty"
            );
            return Vec::new();
        };
        tracing::debug!(
            index,
            path = %item.path,
            scope = ?item.scope,
            status_gen = self.workspace.status_generation.get(&self.store),
            "select_status_item: dispatching LoadStatusDiff"
        );
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            tracing::warn!("select_status_item: no repo_path");
            return Vec::new();
        };

        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Status);
        // Keep the current document visible while the new diff loads — no
        // Loading state, no tear-down. handle_status_diff_finished swaps the
        // ActiveFile atomically when the fresh diff arrives.
        self.workspace
            .selected_file_index
            .set(&self.store, Some(index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(item.path.clone()));
        self.workspace
            .selected_status_scope
            .set(&self.store, Some(item.scope));
        self.file_list.hovered_index.set(&self.store, Some(index));
        if reveal {
            self.reveal_file_list_row(index);
        }

        let generation = self.workspace.status_generation.get(&self.store);
        let renderer = self.compare.renderer.get(&self.store);
        vec![
            SyntaxEffect::EnsureSyntaxPackForPath {
                path: item.path.clone(),
            }
            .into(),
            RepositoryEffect::LoadStatusDiff {
                task: Task {
                    generation,
                    request: StatusDiffRequest {
                        repo_path,
                        item,
                        renderer,
                    },
                },
                index,
            }
            .into(),
        ]
    }

    fn apply_selected_status_operation(&mut self, operation: StatusOperation) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let Some(index) = self.workspace.selected_file_index.get(&self.store) else {
            return Vec::new();
        };
        let Some(item) = self
            .workspace
            .status_items
            .with(&self.store, |items| items.get(index).cloned())
        else {
            return Vec::new();
        };

        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        vec![
            RepositoryEffect::ApplyStatusOperation(StatusOperationRequest {
                repo_path,
                item,
                operation,
            })
            .into(),
        ]
    }

    fn apply_file_status_operation(
        &mut self,
        index: usize,
        operation: StatusOperation,
    ) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let Some(item) = self
            .workspace
            .status_items
            .with(&self.store, |items| items.get(index).cloned())
        else {
            return Vec::new();
        };

        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        vec![
            RepositoryEffect::ApplyStatusOperation(StatusOperationRequest {
                repo_path,
                item,
                operation,
            })
            .into(),
        ]
    }

    fn apply_batch_scope_operation(
        &mut self,
        scopes: &[StatusScope],
        operation: StatusOperation,
    ) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let items: Vec<StatusItem> = self.workspace.status_items.with(&self.store, |items| {
            items
                .iter()
                .filter(|item| scopes.contains(&item.scope))
                .cloned()
                .collect()
        });
        if items.is_empty() {
            return Vec::new();
        }

        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        vec![
            RepositoryEffect::ApplyBatchStatusOperation(BatchStatusOperationRequest {
                repo_path,
                items,
                operation,
            })
            .into(),
        ]
    }

    fn current_hunk_index_from_hover(&self) -> Option<i16> {
        self.editor.hovered_hunk_index.get(&self.store)
    }

    fn apply_hunk_operation(
        &mut self,
        operation: StatusOperation,
        explicit_hunk: Option<i16>,
    ) -> Vec<Effect> {
        tracing::debug!(
            ?operation,
            ?explicit_hunk,
            source = ?self.workspace.source.get(&self.store),
            pending = self.workspace.status_operation_pending.get(&self.store),
            hovered_row = ?self.editor.hovered_row.get(&self.store),
            hovered_hunk_index = ?self.editor.hovered_hunk_index.get(&self.store),
            "apply_hunk_operation: entered"
        );
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            tracing::debug!("apply_hunk_operation: bail: source != Status");
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            tracing::debug!("apply_hunk_operation: bail: status_operation_pending=true");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            tracing::debug!("apply_hunk_operation: bail: no repo_path");
            return Vec::new();
        };
        let Some(scope) = self.workspace.selected_status_scope.get(&self.store) else {
            tracing::debug!("apply_hunk_operation: bail: no selected_status_scope");
            return Vec::new();
        };
        let resolved = explicit_hunk.or_else(|| self.current_hunk_index_from_hover());
        let hunk_index = match resolved {
            Some(idx) if idx >= 0 => idx as usize,
            _ => {
                tracing::debug!(?resolved, "apply_hunk_operation: bail: no hunk_index");
                return Vec::new();
            }
        };

        let patch_text = self.workspace.active_file.with(&self.store, |af| {
            let active = af.as_ref()?;
            patch::format_carbon_hunk_patch(
                &active.carbon_file,
                hunk_index,
                operation != StatusOperation::Stage,
            )
        });
        let Some(patch) = patch_text else {
            tracing::debug!(
                hunk_index,
                "apply_hunk_operation: bail: format_hunk_patch returned None"
            );
            return Vec::new();
        };

        tracing::debug!(
            ?operation,
            hunk_index,
            "apply_hunk_operation: dispatching ApplyPatchOperation"
        );
        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        vec![
            RepositoryEffect::ApplyPatchOperation(PatchOperationRequest {
                repo_path,
                patch,
                scope,
                operation,
            })
            .into(),
        ]
    }

    fn toggle_line_selection(&mut self, row: usize, _extend: bool) {
        let line_opt = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref()
                .and_then(|active| active.render_doc.lines.get(row).copied())
        });
        let Some(line) = line_opt else {
            return;
        };
        let kind = line.row_kind();
        if !matches!(
            kind,
            crate::ui::editor::render_doc::RenderRowKind::Added
                | crate::ui::editor::render_doc::RenderRowKind::Removed
                | crate::ui::editor::render_doc::RenderRowKind::Modified
        ) {
            return;
        }
        if line.hunk_index < 0 {
            return;
        }
        let hunk_id = line.hunk_index as u32;
        self.editor.line_selection.update(&self.store, |ls| {
            if line.old_line_index >= 0 {
                ls.toggle(hunk_id, carbon::DiffSide::Old, line.old_line_index as u32);
            }
            if line.new_line_index >= 0 {
                ls.toggle(hunk_id, carbon::DiffSide::New, line.new_line_index as u32);
            }
            ls.last_toggled_row = Some(row);
        });
    }

    fn toggle_line_selection_range(&mut self, row: usize, anchor: usize) {
        let (start, end) = if row <= anchor {
            (row, anchor)
        } else {
            (anchor, row)
        };
        let lines = self.workspace.active_file.with(&self.store, |af| {
            let Some(active) = af.as_ref() else {
                return Vec::new();
            };
            (start..=end)
                .filter_map(|r| active.render_doc.lines.get(r).copied())
                .collect::<Vec<_>>()
        });
        if lines.is_empty() {
            return;
        }
        self.editor.line_selection.update(&self.store, |ls| {
            for line in &lines {
                let kind = line.row_kind();
                if !matches!(
                    kind,
                    crate::ui::editor::render_doc::RenderRowKind::Added
                        | crate::ui::editor::render_doc::RenderRowKind::Removed
                        | crate::ui::editor::render_doc::RenderRowKind::Modified
                ) {
                    continue;
                }
                if line.hunk_index < 0 {
                    continue;
                }
                let hunk_id = line.hunk_index as u32;
                if line.old_line_index >= 0 {
                    ls.entries
                        .insert(crate::ui::editor::state::LineSelectionKey {
                            hunk_id,
                            side: carbon::DiffSide::Old,
                            source_index: line.old_line_index as u32,
                        });
                }
                if line.new_line_index >= 0 {
                    ls.entries
                        .insert(crate::ui::editor::state::LineSelectionKey {
                            hunk_id,
                            side: carbon::DiffSide::New,
                            source_index: line.new_line_index as u32,
                        });
                }
            }
            ls.last_toggled_row = Some(row);
        });
    }

    fn apply_line_selection_operation(&mut self, operation: StatusOperation) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            return Vec::new();
        }
        if self
            .editor
            .line_selection
            .with(&self.store, |ls| ls.is_empty())
        {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let Some(scope) = self.workspace.selected_status_scope.get(&self.store) else {
            return Vec::new();
        };
        let reverse = operation != StatusOperation::Stage;

        let (hunk_indices, selection_snapshot) =
            self.editor.line_selection.with(&self.store, |ls| {
                let indices: Vec<u32> = ls
                    .entries
                    .iter()
                    .map(|key| key.hunk_id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                (indices, ls.clone())
            });

        let patches = self.workspace.active_file.with(&self.store, |af| {
            let Some(active) = af.as_ref() else {
                return Vec::new();
            };
            let mut patches = Vec::new();
            for hunk_idx in hunk_indices {
                let selected = selection_snapshot
                    .selected_lines_for_hunk(hunk_idx)
                    .into_iter()
                    .map(|key| patch::CarbonLineSelection {
                        side: key.side,
                        source_index: key.source_index,
                    })
                    .collect::<Vec<_>>();
                let patch = patch::format_carbon_lines_patch(
                    &active.carbon_file,
                    carbon::u32_to_usize_saturating(hunk_idx),
                    &selected,
                    reverse,
                );
                if let Some(p) = patch {
                    patches.push(p);
                }
            }
            patches
        });

        self.editor
            .line_selection
            .update(&self.store, |ls| ls.clear());

        if patches.is_empty() {
            return Vec::new();
        }

        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        patches
            .into_iter()
            .map(|p| {
                RepositoryEffect::ApplyPatchOperation(PatchOperationRequest {
                    repo_path: repo_path.clone(),
                    patch: p,
                    scope,
                    operation,
                })
                .into()
            })
            .collect()
    }

    fn scroll_viewport_lines(&mut self, delta_lines: i32) {
        let step_px = 20_i32;
        let delta_px = delta_lines.saturating_mul(step_px);
        self.scroll_viewport_px(delta_px);
    }

    fn scroll_active_overlay_list_px(&mut self, delta_px: i32) {
        match self.overlays_top() {
            Some(
                OverlaySurface::RepoPicker
                | OverlaySurface::RefPicker
                | OverlaySurface::ThemePicker,
            ) => {
                let count = self.overlays.picker.entries.with(&self.store, |e| e.len());
                self.overlays
                    .picker
                    .list
                    .update(&self.store, |l| l.scroll_px(delta_px, count));
            }
            Some(OverlaySurface::CommandPalette) => {
                let count = self
                    .overlays
                    .command_palette
                    .entries
                    .with(&self.store, |e| e.len());
                self.overlays
                    .command_palette
                    .list
                    .update(&self.store, |l| l.scroll_px(delta_px, count));
            }
            _ => {}
        }
    }

    fn scroll_viewport_px(&mut self, delta_px: i32) {
        let current = self.editor.scroll_top_px.get(&self.store);
        let max = self.editor_max_scroll_top_px();
        let next = apply_scroll_delta_px(current, delta_px, max);
        self.editor.scroll_top_px.set(&self.store, next);
    }

    fn scroll_viewport_pages(&mut self, delta_pages: i32) {
        let viewport = self.editor.viewport_height_px.get(&self.store);
        let page_px = ((viewport as f32) * 0.85).round().max(1.0) as i32;
        let delta_px = delta_pages.saturating_mul(page_px);
        let current = self.editor.scroll_top_px.get(&self.store);
        let max = self.editor_max_scroll_top_px();
        let next = apply_scroll_delta_px(current, delta_px, max);
        self.editor.scroll_top_px.set(&self.store, next);
    }

    fn scroll_viewport_half_page(&mut self, direction: i32) {
        let viewport = self.editor.viewport_height_px.get(&self.store);
        let half_px = ((viewport as f32) * 0.5).round().max(1.0) as i32;
        let delta_px = direction.saturating_mul(half_px);
        let current = self.editor.scroll_top_px.get(&self.store);
        let max = self.editor_max_scroll_top_px();
        let next = apply_scroll_delta_px(current, delta_px, max);
        self.editor.scroll_top_px.set(&self.store, next);
    }

    fn request_active_file_syntax_effect(&mut self) -> Option<Effect> {
        let repo_path = self.compare.repo_path.get(&self.store)?;
        let window = self.desired_syntax_window()?;
        let generation = self.active_syntax_generation();
        let mut request = None;

        self.workspace.active_file.update(&self.store, |active| {
            let Some(active) = active.as_mut() else {
                return;
            };
            if active
                .syntax_pending
                .is_some_and(|pending| pending.contains(window))
                || active
                    .syntax_covered
                    .iter()
                    .any(|covered| covered.contains(window))
            {
                return;
            }

            active.syntax_request_id = active.syntax_request_id.saturating_add(1);
            active.syntax_pending = Some(window);
            request = Some(LoadFileSyntaxRequest {
                repo_path,
                file_index: active.index,
                path: active.path.clone(),
                carbon_file: active.carbon_file.clone(),
                left_ref: active.left_ref.clone(),
                right_ref: active.right_ref.clone(),
                window,
                request_id: active.syntax_request_id,
                cache_generation: generation,
            });
        });

        request.map(|request| {
            SyntaxEffect::LoadFileSyntax(Task {
                generation,
                request,
            })
            .into()
        })
    }

    fn active_syntax_generation(&self) -> u64 {
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Status => self.workspace.status_generation.get(&self.store),
            _ => self.workspace.compare_generation.get(&self.store),
        }
    }

    fn desired_syntax_window(&self) -> Option<SyntaxRowWindow> {
        let line_count = self.workspace.active_file.with(&self.store, |active| {
            active.as_ref().map(|active| active.render_doc.lines.len())
        })?;
        if line_count == 0 {
            return None;
        }

        if let (Some(start), Some(end)) = (
            self.editor.visible_row_start.get(&self.store),
            self.editor.visible_row_end.get(&self.store),
        ) && end > start
        {
            return Some(SyntaxRowWindow {
                start: start.saturating_sub(SYNTAX_OVERSCAN_ROWS),
                end: end.saturating_add(SYNTAX_OVERSCAN_ROWS).min(line_count),
            });
        }

        let scroll = self.editor.scroll_top_px.get(&self.store) as usize;
        let viewport = self.editor.viewport_height_px.get(&self.store) as usize;
        let approx_row_height = 20usize;
        let start = scroll / approx_row_height;
        let visible = (viewport / approx_row_height).saturating_add(SYNTAX_INITIAL_ROWS);
        Some(SyntaxRowWindow {
            start: start.saturating_sub(SYNTAX_OVERSCAN_ROWS),
            end: start.saturating_add(visible).min(line_count),
        })
    }

    fn navigate_to_hunk(&mut self, forward: bool) {
        let current = self.editor.scroll_top_px.get(&self.store);
        let target = self.editor.hunk_positions.with(&self.store, |positions| {
            if positions.is_empty() {
                return None;
            }
            if forward {
                positions
                    .iter()
                    .find(|&&y| y > current)
                    .or_else(|| positions.first())
                    .copied()
            } else {
                positions
                    .iter()
                    .rev()
                    .find(|&&y| y < current)
                    .or_else(|| positions.last())
                    .copied()
            }
        });
        if let Some(y) = target {
            self.editor.scroll_top_px.set(&self.store, y);
            self.editor_clamp_scroll();
        }
    }

    fn navigate_to_file(&mut self, forward: bool) {
        let current = self.editor.scroll_top_px.get(&self.store);
        let target = self.editor.file_positions.with(&self.store, |positions| {
            if positions.is_empty() {
                return None;
            }
            if forward {
                positions
                    .iter()
                    .find(|&&y| y > current)
                    .or_else(|| positions.first())
                    .copied()
            } else {
                positions
                    .iter()
                    .rev()
                    .find(|&&y| y < current)
                    .or_else(|| positions.last())
                    .copied()
            }
        });
        if let Some(y) = target {
            self.editor.scroll_top_px.set(&self.store, y);
            self.editor_clamp_scroll();
        }
    }

    fn push_error(&mut self, message: &str) -> u64 {
        self.last_error.set(&self.store, Some(message.to_owned()));
        self.push_toast(ToastKind::Error, message, None, None)
    }

    fn push_info(&mut self, message: &str) -> u64 {
        self.push_toast(ToastKind::Info, message, None, None)
    }

    #[allow(dead_code)]
    fn push_error_with_description(&mut self, message: &str, description: &str) -> u64 {
        self.last_error.set(&self.store, Some(message.to_owned()));
        self.push_toast(
            ToastKind::Error,
            message,
            Some(description.to_owned()),
            None,
        )
    }

    #[allow(dead_code)]
    fn push_info_with_description(&mut self, message: &str, description: &str) -> u64 {
        self.push_toast(ToastKind::Info, message, Some(description.to_owned()), None)
    }

    /// Create an info toast with an externally-driven progress bar (0.0-1.0).
    /// The toast is pinned until `finish_progress_toast` or `fail_progress_toast`
    /// is called — it does not auto-dismiss based on time.
    fn push_progress_toast(&mut self, message: &str) -> u64 {
        self.push_toast(ToastKind::Info, message, None, Some(0.0))
    }

    /// Convert a pinned progress toast into a normal info toast and let it
    /// auto-dismiss. Also updates its message and description.
    fn finish_progress_toast(&mut self, toast_id: u64, message: &str, description: Option<String>) {
        let now = self.clock_ms;
        self.toasts.update(&self.store, |toasts| {
            if let Some(toast) = toasts.iter_mut().find(|t| t.id == toast_id) {
                toast.kind = ToastKind::Info;
                toast.message = message.to_owned();
                toast.description = description;
                toast.created_at_ms = now;
                toast.progress = None;
            }
        });
    }

    /// Convert a pinned progress toast into an error toast.
    fn fail_progress_toast(&mut self, toast_id: u64, message: &str, description: Option<String>) {
        let now = self.clock_ms;
        self.last_error.set(&self.store, Some(message.to_owned()));
        self.toasts.update(&self.store, |toasts| {
            if let Some(toast) = toasts.iter_mut().find(|t| t.id == toast_id) {
                toast.kind = ToastKind::Error;
                toast.message = message.to_owned();
                toast.description = description;
                toast.created_at_ms = now;
                toast.progress = None;
            }
        });
    }

    fn update_toast_progress(&mut self, toast_id: u64, fraction: f32) {
        let clamped = fraction.clamp(0.0, 1.0);
        self.toasts.update(&self.store, |toasts| {
            if let Some(toast) = toasts.iter_mut().find(|t| t.id == toast_id) {
                toast.progress = Some(clamped);
            }
        });
    }

    fn update_toast_message(&mut self, toast_id: u64, message: &str) {
        self.toasts.update(&self.store, |toasts| {
            if let Some(toast) = toasts.iter_mut().find(|t| t.id == toast_id) {
                toast.message = message.to_owned();
            }
        });
    }

    fn start_fetch_remote(&mut self, remote: String) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before fetching.");
            return Vec::new();
        };
        let toast_id = self.push_progress_toast(&format!("Fetching {remote}\u{2026}"));
        vec![
            RepositoryEffect::FetchRemote(FetchRemoteRequest {
                repo_path,
                remote,
                toast_id,
            })
            .into(),
        ]
    }

    fn start_fetch_all_remotes(&mut self) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before fetching.");
            return Vec::new();
        };
        let remotes = match crate::core::vcs::git::GitService::new_with_open(&repo_path)
            .and_then(|git| git.remote_names())
        {
            Ok(names) if !names.is_empty() => names,
            Ok(_) => {
                self.push_error("No remotes are configured for this repository.");
                return Vec::new();
            }
            Err(error) => {
                self.push_error(&error.to_string());
                return Vec::new();
            }
        };
        remotes
            .into_iter()
            .flat_map(|remote| {
                let toast_id = self.push_progress_toast(&format!("Fetching {remote}\u{2026}"));
                std::iter::once(
                    RepositoryEffect::FetchRemote(FetchRemoteRequest {
                        repo_path: repo_path.clone(),
                        remote,
                        toast_id,
                    })
                    .into(),
                )
            })
            .collect()
    }

    fn start_push_current_branch(&mut self, force_with_lease: bool) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before pushing.");
            return Vec::new();
        };
        let git = match crate::core::vcs::git::GitService::new_with_open(&repo_path) {
            Ok(git) => git,
            Err(error) => {
                self.push_error(&error.to_string());
                return Vec::new();
            }
        };
        let branch = match git.head_branch_name() {
            Ok(Some(name)) => name,
            Ok(None) => {
                self.push_error("HEAD is detached; no branch to push.");
                return Vec::new();
            }
            Err(error) => {
                self.push_error(&error.to_string());
                return Vec::new();
            }
        };
        let upstream = git.upstream_for(&branch).ok().flatten();
        let (remote, refspec) = match upstream {
            Some((remote, upstream_branch)) => (
                remote,
                format!("refs/heads/{branch}:refs/heads/{upstream_branch}"),
            ),
            None => {
                // No upstream configured yet — default to `origin/<branch>`.
                let remotes = git.remote_names().unwrap_or_default();
                let remote = if remotes.iter().any(|n| n == "origin") {
                    "origin".to_owned()
                } else if let Some(first) = remotes.first() {
                    first.clone()
                } else {
                    self.push_error("No remotes are configured for this repository.");
                    return Vec::new();
                };
                (remote, format!("refs/heads/{branch}:refs/heads/{branch}"))
            }
        };
        let label = if force_with_lease {
            format!("Force-pushing {branch} to {remote}\u{2026}")
        } else {
            format!("Pushing {branch} to {remote}\u{2026}")
        };
        let toast_id = self.push_progress_toast(&label);
        vec![
            RepositoryEffect::Push(PushRequest {
                repo_path,
                remote,
                refspec,
                force_with_lease,
                toast_id,
            })
            .into(),
        ]
    }

    fn start_pull_current_branch(&mut self) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before pulling.");
            return Vec::new();
        };
        let git = match crate::core::vcs::git::GitService::new_with_open(&repo_path) {
            Ok(git) => git,
            Err(error) => {
                self.push_error(&error.to_string());
                return Vec::new();
            }
        };
        let branch = match git.head_branch_name() {
            Ok(Some(name)) => name,
            Ok(None) => {
                self.push_error("HEAD is detached; no branch to pull into.");
                return Vec::new();
            }
            Err(error) => {
                self.push_error(&error.to_string());
                return Vec::new();
            }
        };
        let (remote, upstream_branch) = match git.upstream_for(&branch) {
            Ok(Some(pair)) => pair,
            Ok(None) => {
                self.push_error(&format!(
                    "No upstream configured for {branch}. Push once to set one."
                ));
                return Vec::new();
            }
            Err(error) => {
                self.push_error(&error.to_string());
                return Vec::new();
            }
        };
        let toast_id = self.push_progress_toast(&format!("Pulling {branch} from {remote}\u{2026}"));
        vec![
            RepositoryEffect::PullFf(PullFfRequest {
                repo_path,
                remote,
                branch: upstream_branch,
                toast_id,
            })
            .into(),
        ]
    }

    fn push_toast(
        &mut self,
        kind: ToastKind,
        message: &str,
        description: Option<String>,
        progress: Option<f32>,
    ) -> u64 {
        use crate::ui::animation::AnimationKey;
        let id = self.next_toast_id;
        self.next_toast_id = self.next_toast_id.saturating_add(1);
        self.animation.set_target(
            AnimationKey::ToastEntrance(id),
            1.0,
            TOAST_ANIM_MS,
            self.clock_ms,
        );
        let now = self.clock_ms;
        self.toasts.update(&self.store, |toasts| {
            toasts.push(Toast {
                id,
                kind,
                message: message.to_owned(),
                description,
                created_at_ms: now,
                hovered: false,
                progress,
            });
            if toasts.len() > MAX_VISIBLE_TOASTS {
                toasts.remove(0);
            }
        });
        id
    }

    fn open_search(&mut self) {
        self.editor.search.open.set(&self.store, true);
        let len = self.editor.search.query.with(&self.store, |q| q.len());
        self.text_edit.cursor.set(&self.store, len);
        self.text_edit.anchor.set(&self.store, 0);
        self.text_edit
            .cursor_moved_at_ms
            .set(&self.store, self.clock_ms);
        self.focus.set(&self.store, Some(FocusTarget::SearchInput));
        self.editor.focused.set(&self.store, false);
        self.recompute_search_matches();
    }

    fn close_search(&mut self) {
        self.editor.search.open.set(&self.store, false);
        self.editor
            .search
            .matches
            .update(&self.store, |matches| matches.clear());
        self.editor.search.active_index.set(&self.store, None);
        self.set_focus(Some(FocusTarget::Editor));
    }

    fn recompute_search_matches(&mut self) {
        use crate::ui::editor::state::MatchSide;

        self.editor
            .search
            .matches
            .update(&self.store, |matches| matches.clear());
        self.editor.search.active_index.set(&self.store, None);

        let query = self
            .editor
            .search
            .query
            .with(&self.store, |q| q.to_ascii_lowercase());
        if query.is_empty() {
            return;
        }

        let new_matches: Vec<SearchMatch> = self.workspace.active_file.with(&self.store, |af| {
            let Some(active_file) = af.as_ref() else {
                return Vec::new();
            };
            let doc = &active_file.render_doc;
            let mut new_matches: Vec<SearchMatch> = Vec::new();
            for (line_idx, line) in doc.lines.iter().enumerate() {
                let line_idx = line_idx as u32;
                if line.left_text.is_valid() {
                    let text = doc.line_text(line.left_text);
                    let lower = text.to_ascii_lowercase();
                    let mut start = 0;
                    while let Some(pos) = lower[start..].find(&query) {
                        let byte_start = (start + pos) as u32;
                        new_matches.push(SearchMatch {
                            line_index: line_idx,
                            byte_start,
                            byte_len: query.len() as u32,
                            side: MatchSide::Left,
                        });
                        start += pos + query.len();
                    }
                }
                if line.right_text.is_valid() {
                    let text = doc.line_text(line.right_text);
                    let lower = text.to_ascii_lowercase();
                    let mut start = 0;
                    while let Some(pos) = lower[start..].find(&query) {
                        let byte_start = (start + pos) as u32;
                        new_matches.push(SearchMatch {
                            line_index: line_idx,
                            byte_start,
                            byte_len: query.len() as u32,
                            side: MatchSide::Right,
                        });
                        start += pos + query.len();
                    }
                }
            }
            new_matches
        });

        let has_matches = !new_matches.is_empty();
        self.editor.search.matches.set(&self.store, new_matches);
        if has_matches {
            self.editor.search.active_index.set(&self.store, Some(0));
        }
    }

    fn search_navigate(&mut self, direction: i32) {
        let count = self.editor.search.matches.with(&self.store, |m| m.len());
        if count == 0 {
            return;
        }

        let current = self
            .editor
            .search
            .active_index
            .get(&self.store)
            .unwrap_or(0);
        let next = if direction > 0 {
            if current + 1 >= count { 0 } else { current + 1 }
        } else {
            if current == 0 { count - 1 } else { current - 1 }
        };
        self.editor.search.active_index.set(&self.store, Some(next));
        self.scroll_to_search_match(next);
    }

    fn scroll_to_search_match(&mut self, match_index: usize) {
        let y_pos = self
            .editor
            .search_match_y_positions
            .with(&self.store, |v| v.get(match_index).copied());
        let target_y = if let Some(y) = y_pos {
            y
        } else {
            let m = self
                .editor
                .search
                .matches
                .with(&self.store, |m| m.get(match_index).copied());
            let Some(m) = m else {
                return;
            };
            self.estimate_line_y(m.line_index)
        };

        let viewport_h = self.editor.viewport_height_px.get(&self.store);
        let centered = target_y.saturating_sub(viewport_h / 3);
        let max = self.editor_max_scroll_top_px();
        self.editor
            .scroll_top_px
            .set(&self.store, centered.min(max));
    }

    fn estimate_line_y(&self, line_index: u32) -> u32 {
        let content_height = self.editor.content_height_px.get(&self.store);
        if content_height == 0 {
            return 0;
        }
        let total_lines = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref()
                .map(|active_file| active_file.render_doc.lines.len() as u32)
                .unwrap_or(0)
        });
        if total_lines == 0 {
            return 0;
        }
        let avg_height = content_height / total_lines;
        line_index.saturating_mul(avg_height)
    }

    // -------- EditorState helpers on AppState --------

    /// Clear document-specific editor state (scroll, content, hunks, etc.)
    pub fn editor_clear_document(&mut self) {
        self.editor.scroll_top_px.set(&self.store, 0);
        self.editor.content_height_px.set(&self.store, 0);
        self.editor.hovered_row.set(&self.store, None);
        self.editor.hovered_hunk_index.set(&self.store, None);
        self.editor.visible_row_start.set(&self.store, None);
        self.editor.visible_row_end.set(&self.store, None);
        self.editor
            .hunk_positions
            .update(&self.store, |v| v.clear());
        self.editor
            .file_positions
            .update(&self.store, |v| v.clear());
        self.editor
            .search_match_y_positions
            .update(&self.store, |v| v.clear());
        self.editor
            .line_selection
            .update(&self.store, |ls| ls.clear());
    }

    pub fn editor_max_scroll_top_px(&self) -> u32 {
        let content = self.editor.content_height_px.get(&self.store);
        let viewport = self.editor.viewport_height_px.get(&self.store);
        content.saturating_sub(viewport.max(1))
    }

    pub fn editor_clamp_scroll(&mut self) {
        let max = self.editor_max_scroll_top_px();
        let cur = self.editor.scroll_top_px.get(&self.store);
        self.editor.scroll_top_px.set(&self.store, cur.min(max));
    }

    pub fn editor_current_hunk_index(&self) -> Option<(usize, usize)> {
        let scroll = self.editor.scroll_top_px.get(&self.store);
        self.editor.hunk_positions.with(&self.store, |positions| {
            if positions.is_empty() {
                return None;
            }
            let idx = positions
                .partition_point(|&y| y <= scroll)
                .saturating_sub(1);
            Some((idx, positions.len()))
        })
    }
}

fn matching_persisted_compare<'a>(
    startup: &'a StartupOptions,
    settings: &'a Settings,
) -> Option<&'a PersistedCompare> {
    settings.last_compare.as_ref().filter(|compare| {
        startup.args.repo.is_some() && compare.repo_path.as_ref() == startup.args.repo.as_ref()
    })
}

fn compare_refs_are_valid(mode: CompareMode, left_ref: &str, right_ref: &str) -> bool {
    match mode {
        CompareMode::SingleCommit => !left_ref.is_empty() || !right_ref.is_empty(),
        CompareMode::TwoDot | CompareMode::ThreeDot => {
            !left_ref.is_empty() && !right_ref.is_empty()
        }
    }
}

/// Short, user-facing label for a ref. Maps the internal WORKDIR sentinel
/// to a friendly "working copy"; empty strings become the conventional
/// "base"/"head" placeholder (handled by the caller since the side matters).
fn compare_ref_display_label(value: &str) -> String {
    if value == crate::core::vcs::git::service::WORKDIR_REF {
        "working copy".to_owned()
    } else if value.is_empty() {
        "\u{2014}".to_owned()
    } else {
        value.to_owned()
    }
}

fn apply_scroll_delta_px(current: u32, delta: i32, max: u32) -> u32 {
    let next = if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as u32)
    };
    next.min(max)
}

fn build_carbon_file_entries(files: &[carbon::FileDiff]) -> Vec<FileListEntry> {
    files.iter().map(carbon_file_entry).collect()
}

fn carbon_file_entry(file: &carbon::FileDiff) -> FileListEntry {
    let (additions, deletions) = carbon_file_stats(file);
    FileListEntry {
        path: file.path().to_owned(),
        status: carbon_status_label(file.status).to_owned(),
        additions,
        deletions,
        is_binary: file.is_binary,
    }
}

fn carbon_file_stats(file: &carbon::FileDiff) -> (i32, i32) {
    if file.additions > 0 || file.deletions > 0 || file.stats_deferred {
        return (
            u32_to_i32_saturating(file.additions),
            u32_to_i32_saturating(file.deletions),
        );
    }
    let mut additions = 0_i32;
    let mut deletions = 0_i32;
    for block in &file.blocks {
        if block.kind == carbon::BlockKind::Change {
            additions = additions.saturating_add(block.new.len.min(i32::MAX as u32) as i32);
            deletions = deletions.saturating_add(block.old.len.min(i32::MAX as u32) as i32);
        }
    }
    (additions, deletions)
}

fn u32_to_i32_saturating(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn carbon_status_label(status: carbon::FileStatus) -> &'static str {
    match status {
        carbon::FileStatus::Added => "A",
        carbon::FileStatus::Deleted => "D",
        carbon::FileStatus::Renamed | carbon::FileStatus::RenamedModified => "R",
        carbon::FileStatus::Binary => "B",
        carbon::FileStatus::ModeChanged | carbon::FileStatus::Modified => "M",
    }
}

fn build_status_file_entries(items: &[StatusItem]) -> Vec<FileListEntry> {
    items.iter().map(FileListEntry::from).collect()
}

fn status_section_count(items: &[StatusItem]) -> usize {
    let mut last_scope = None;
    let mut count = 0;
    for item in items {
        if Some(item.scope) != last_scope {
            count += 1;
            last_scope = Some(item.scope);
        }
    }
    count
}

fn status_section_count_before(items: &[StatusItem], len: usize) -> usize {
    status_section_count(&items[..len.min(items.len())])
}

fn overlay_name(surface: OverlaySurface) -> &'static str {
    match surface {
        OverlaySurface::RepoPicker => "repo-picker",
        OverlaySurface::RefPicker => "ref-picker",
        OverlaySurface::CommandPalette => "command-palette",
        OverlaySurface::GitHubAuthModal => "github-auth-modal",
        OverlaySurface::AccountMenu => "account-menu",
        OverlaySurface::KeyboardShortcuts => "keyboard-shortcuts",
        OverlaySurface::ThemePicker => "theme-picker",
        OverlaySurface::CompareMenu => "compare-menu",
    }
}

pub fn workspace_mode_name(mode: WorkspaceMode) -> &'static str {
    match mode {
        WorkspaceMode::Empty => "empty",
        WorkspaceMode::Loading => "loading",
        WorkspaceMode::Ready => "ready",
    }
}

impl From<&StatusItem> for FileListEntry {
    fn from(value: &StatusItem) -> Self {
        Self {
            path: value.path.clone(),
            status: value.status.clone(),
            additions: 0,
            deletions: 0,
            is_binary: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Grapheme / word boundary helpers
// ---------------------------------------------------------------------------

// Grapheme/word boundary helpers are in text_edit.rs

fn highlight_ranges_from_match_indices(text: &str, indices_rev: &[usize]) -> Vec<(usize, usize)> {
    let len = text.len();
    let mut indices: Vec<usize> = indices_rev
        .iter()
        .copied()
        .filter(|&idx| idx < len && text.is_char_boundary(idx))
        .collect();
    indices.sort_unstable();

    let mut ranges = Vec::new();
    for index in indices {
        let mut end = index + 1;
        while end < len && !text.is_char_boundary(end) {
            end += 1;
        }
        if let Some((_, last_end)) = ranges.last_mut() {
            if index <= *last_end {
                *last_end = (*last_end).max(end);
                continue;
            }
        }
        ranges.push((index, end));
    }
    ranges
}

fn highlight_ranges_for_prefix_match(text: &str, indices_rev: &[usize]) -> Vec<(usize, usize)> {
    let prefix_indices: Vec<usize> = indices_rev
        .iter()
        .copied()
        .filter(|&idx| idx < text.len())
        .collect();
    highlight_ranges_from_match_indices(text, &prefix_indices)
}

fn highlight_ranges_for_visible_match(
    query: &str,
    visible_text: &str,
    search_indices_rev: &[usize],
    config: &neo_frizbee::Config,
) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }

    let visible_only = [visible_text];
    if let Some(m) = neo_frizbee::match_list_indices(query, &visible_only, config)
        .into_iter()
        .next()
    {
        return highlight_ranges_from_match_indices(visible_text, &m.indices);
    }

    highlight_ranges_for_prefix_match(visible_text, search_indices_rev)
}

fn query_looks_like_path(query: &str) -> bool {
    query.starts_with('/')
        || query.starts_with("~/")
        || query.starts_with("./")
        || (query.len() >= 2 && query.as_bytes()[1] == b':')
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") || path == "~" {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_owned()
}

fn split_browse_query(expanded: &str) -> (String, &str) {
    if let Some(pos) = expanded.rfind('/') {
        let dir = if pos == 0 {
            "/".to_owned()
        } else {
            expanded[..pos].to_owned()
        };
        let filter = &expanded[pos + 1..];
        (dir, filter)
    } else if expanded.len() >= 2 && expanded.as_bytes()[1] == b':' {
        if let Some(pos) = expanded.rfind('\\') {
            let dir = expanded[..pos].to_owned();
            let filter = &expanded[pos + 1..];
            (dir, filter)
        } else {
            (expanded.to_owned(), "")
        }
    } else {
        (expanded.to_owned(), "")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use super::{
        ActiveFile, ActiveFileLoading, AppState, AsyncStatus, CarbonStyleOverlays, CompareField,
        FileListEntry, FocusTarget, OverlaySurface, PreparedActiveFile, WorkspaceMode,
        WorkspaceSource, prepare_active_file, refs_for_status_scope,
    };
    use crate::core::compare::{CompareMode, CompareOutput, LayoutMode, RendererKind};
    use crate::core::text::TokenBuffer;
    use crate::core::vcs::git::{StatusItem, StatusScope};
    use crate::effects::{
        AiEffect, CompareEffect, Effect, GitHubEffect, RepositoryEffect, SyntaxEffect,
    };
    use crate::events::{
        AppEvent, CompareEvent, CompareFileFinished, GitHubEvent, RepositoryEvent,
    };
    use crate::platform::persistence::Settings;
    use crate::platform::startup::{Args, StartupOptions};
    use crate::ui::editor::render_doc::{RenderDoc, build_render_doc_from_carbon};

    fn carbon_summary_for_path(index: usize, path: &str) -> carbon::FileDiff {
        carbon::FileDiff {
            id: carbon::FileId(index as u32),
            old_path: Some(path.to_owned()),
            new_path: Some(path.to_owned()),
            is_partial: true,
            ..carbon::FileDiff::default()
        }
    }

    fn carbon_context_file(index: usize, path: &str, text: &str) -> carbon::FileDiff {
        carbon::parse_unified_patch(&format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n {text}\n"
        ))
        .unwrap()
        .files
        .into_iter()
        .next()
        .map(|mut file| {
            file.id = carbon::FileId(index as u32);
            file
        })
        .unwrap()
    }

    fn status_state_with_two_hunks() -> AppState {
        let state = AppState::default();
        let repo_path = PathBuf::from("/repo");
        let path = "src/lib.rs".to_owned();
        let token_buffer = TokenBuffer::default();
        let carbon_file = carbon::parse_unified_patch(
            "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,2 @@
 fn one() {
-    old_first();
 }
@@ -8,3 +7,2 @@
 fn two() {
-    old_second();
 }
",
        )
        .unwrap()
        .files
        .into_iter()
        .next()
        .unwrap();
        let carbon_expansion = carbon::ExpansionState::default();
        let render_doc = build_render_doc_from_carbon(
            &carbon_file,
            0,
            &carbon_expansion,
            &CarbonStyleOverlays::default(),
            &token_buffer,
        );
        let (left_ref, right_ref) = refs_for_status_scope(StatusScope::Unstaged);

        state.compare.repo_path.set(&state.store, Some(repo_path));
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Status);
        state.workspace.status.set(&state.store, AsyncStatus::Ready);
        state
            .workspace
            .status_operation_pending
            .set(&state.store, false);
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state.workspace.files.set(
            &state.store,
            vec![FileListEntry {
                path: path.clone(),
                status: "M".to_owned(),
                additions: 0,
                deletions: 0,
                is_binary: false,
            }],
        );
        state.workspace.status_items.set(
            &state.store,
            vec![StatusItem {
                path: path.clone(),
                scope: StatusScope::Unstaged,
                status: "M".to_owned(),
            }],
        );
        state
            .workspace
            .selected_file_index
            .set(&state.store, Some(0));
        state
            .workspace
            .selected_file_path
            .set(&state.store, Some(path.clone()));
        state
            .workspace
            .selected_status_scope
            .set(&state.store, Some(StatusScope::Unstaged));
        state.workspace.active_file.set(
            &state.store,
            Some(ActiveFile {
                index: 0,
                path,
                carbon_file: carbon_file.clone(),
                carbon_expansion,
                carbon_overlays: CarbonStyleOverlays::default(),
                render_doc,
                base_carbon_file: carbon_file,
                token_buffer,
                left_ref,
                right_ref,
                file_line_count: None,
                old_file_lines: None,
                file_lines: None,
                syntax_request_id: 0,
                syntax_pending: None,
                syntax_covered: Vec::new(),
            }),
        );

        state
    }

    fn loaded_state_with_files(paths: &[&str]) -> AppState {
        let state = AppState::default();
        let carbon_files: Vec<carbon::FileDiff> = paths
            .iter()
            .enumerate()
            .map(|(index, path)| carbon_context_file(index, path, "loaded"))
            .collect();
        let entries: Vec<FileListEntry> = carbon_files
            .iter()
            .map(|file| FileListEntry {
                path: file.path().to_owned(),
                status: "M".to_owned(),
                additions: 0,
                deletions: 0,
                is_binary: false,
            })
            .collect();

        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                carbon: carbon::DiffDocument {
                    files: carbon_files,
                },
                ..CompareOutput::default()
            }),
        );
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.workspace.files.set(&state.store, entries);
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state.file_list.row_height.set(&state.store, 36.0);
        state.file_list.gap.set(&state.store, 4.0);
        state.file_list.viewport_height.set(&state.store, 80.0);
        state
    }

    #[test]
    fn bootstrap_with_no_repo_starts_empty_workspace() {
        let startup = StartupOptions::from_parts(
            Args::parse_from(["diffy"]),
            None,
            "client".to_owned(),
            false,
        );

        let (state, effects) = AppState::bootstrap(startup, Settings::default());
        assert_eq!(state.workspace_mode.get(&state.store), WorkspaceMode::Empty);
        assert_eq!(
            state.focus.get(&state.store),
            Some(FocusTarget::WorkspacePrimaryButton)
        );
        assert!(effects.iter().all(|e| matches!(
            e,
            Effect::GitHub(GitHubEffect::LoadGitHubToken)
                | Effect::Ai(AiEffect::LoadAiKeys)
                | Effect::Syntax(SyntaxEffect::InstallCommonSyntaxPacks)
        )));
    }

    #[test]
    fn bootstrap_with_repo_starts_repo_sync() {
        let startup = StartupOptions::from_parts(
            Args {
                repo: Some("C:\\repo".into()),
                left: Some("main".to_owned()),
                right: None,
                compare_mode: Some(CompareMode::TwoDot),
                layout: Some(LayoutMode::Unified),
                renderer: Some(RendererKind::Builtin),
                file_index: None,
                file_path: None,
                open_pr: None,
            },
            None,
            "client".to_owned(),
            false,
        );

        let (state, effects) = AppState::bootstrap(startup, Settings::default());
        assert_eq!(state.workspace_mode.get(&state.store), WorkspaceMode::Empty);
        assert_eq!(state.active_overlay_name(), None);
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(
                    e,
                    Effect::Repository(RepositoryEffect::SyncRepository { .. })
                        | Effect::Repository(RepositoryEffect::WatchRepository { .. })
                ))
                .count(),
            2
        );
    }

    #[test]
    fn overlay_close_restores_prior_focus() {
        let startup = StartupOptions::from_parts(
            Args::parse_from(["diffy"]),
            None,
            "client".to_owned(),
            false,
        );
        let (mut state, _) = AppState::bootstrap(startup, Settings::default());
        state.apply_action(crate::actions::AppAction::SetFocus(Some(
            FocusTarget::TitleBar,
        )));
        state.apply_action(crate::actions::OverlayAction::OpenCommandPalette);
        assert_eq!(state.overlays_top(), Some(OverlaySurface::CommandPalette));
        state.apply_action(crate::actions::OverlayAction::CloseOverlay);
        assert_eq!(state.focus.get(&state.store), Some(FocusTarget::TitleBar));
    }

    #[test]
    fn pixel_scroll_actions_clamp_file_list_and_viewport() {
        let mut state = AppState::default();

        state.workspace.files.set(
            &state.store,
            vec![
                FileListEntry {
                    path: "a.rs".into(),
                    status: "M".into(),
                    additions: 0,
                    deletions: 0,
                    is_binary: false,
                },
                FileListEntry {
                    path: "b.rs".into(),
                    status: "M".into(),
                    additions: 0,
                    deletions: 0,
                    is_binary: false,
                },
                FileListEntry {
                    path: "c.rs".into(),
                    status: "M".into(),
                    additions: 0,
                    deletions: 0,
                    is_binary: false,
                },
                FileListEntry {
                    path: "d.rs".into(),
                    status: "M".into(),
                    additions: 0,
                    deletions: 0,
                    is_binary: false,
                },
                FileListEntry {
                    path: "e.rs".into(),
                    status: "M".into(),
                    additions: 0,
                    deletions: 0,
                    is_binary: false,
                },
            ],
        );
        state.file_list.row_height.set(&state.store, 36.0);
        state.file_list.gap.set(&state.store, 4.0);
        state.file_list.viewport_height.set(&state.store, 80.0);

        state.apply_action(crate::actions::FileListAction::ScrollFileListPx(50));
        assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 50.0);

        state.apply_action(crate::actions::FileListAction::ScrollFileListPx(500));
        assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 116.0);

        state.apply_action(crate::actions::FileListAction::ScrollFileListPx(-500));
        assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 0.0);

        state.editor.content_height_px.set(&state.store, 600);
        state.editor.viewport_height_px.set(&state.store, 200);

        state.apply_action(crate::actions::EditorAction::ScrollViewportPx(75));
        assert_eq!(state.editor.scroll_top_px.get(&state.store), 75);

        state.apply_action(crate::actions::EditorAction::ScrollViewportPx(500));
        assert_eq!(state.editor.scroll_top_px.get(&state.store), 400);

        state.apply_action(crate::actions::EditorAction::ScrollViewportPx(-500));
        assert_eq!(state.editor.scroll_top_px.get(&state.store), 0);
    }

    #[test]
    fn clicking_a_visible_file_does_not_force_sidebar_reveal() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.file_list.scroll_offset_px.set(&state.store, 10.0);

        state.apply_action(crate::actions::FileListAction::SelectFile(0));

        assert_eq!(
            state.workspace.selected_file_index.get(&state.store),
            Some(0)
        );
        assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 10.0);
    }

    #[test]
    fn keyboard_file_navigation_still_reveals_selection() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs", "d.rs"]);
        state
            .workspace
            .selected_file_index
            .set(&state.store, Some(0));
        state
            .workspace
            .selected_file_path
            .set(&state.store, Some("a.rs".into()));
        state.file_list.scroll_offset_px.set(&state.store, 50.0);

        state.apply_action(crate::actions::FileListAction::SelectNextFile);

        assert_eq!(
            state.workspace.selected_file_index.get(&state.store),
            Some(1)
        );
        assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 40.0);
    }

    #[test]
    fn selecting_a_file_requests_async_syntax_without_mutating_compare_output() {
        let mut state = AppState::default();
        let mut output = CompareOutput::default();
        output.carbon.files = vec![carbon_context_file(
            0,
            "src/lib.rs",
            "fn answer() -> i32 { 42 }",
        )];
        state
            .workspace
            .compare_output
            .set(&state.store, Some(output));
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.workspace.files.set(
            &state.store,
            vec![FileListEntry {
                path: "src/lib.rs".to_owned(),
                status: "M".to_owned(),
                additions: 0,
                deletions: 0,
                is_binary: false,
            }],
        );
        state
            .compare
            .repo_path
            .set(&state.store, Some(PathBuf::from("/tmp/repo")));
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);

        let effects = state.apply_action(crate::actions::FileListAction::SelectFile(0));

        assert!(effects.iter().any(|effect| {
            matches!(
                effect,
                Effect::Syntax(SyntaxEffect::LoadFileSyntax(task))
                    if task.request.path == "src/lib.rs"
                        && task.request.window.start == 0
                        && task.request.window.end > 0
            )
        }));
        state.workspace.compare_output.with(&state.store, |co| {
            let output = co.as_ref().expect("compare output");
            assert_eq!(output.carbon.files[0].path(), "src/lib.rs");
            assert_eq!(output.carbon.files[0].hunks.len(), 1);
        });
    }

    #[test]
    fn prepare_active_file_builds_from_carbon_text() {
        let carbon_file = carbon::parse_unified_patch(
            "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
 fn answer() -> i32 { 42 }
",
        )
        .unwrap()
        .files
        .into_iter()
        .next()
        .unwrap();

        let prepared = prepare_active_file(0, &carbon_file);

        assert_eq!(prepared.carbon_file.path(), "src/lib.rs");
        assert!(prepared.render_doc.lines.iter().any(|render_line| {
            prepared.render_doc.line_text(render_line.left_text) == "fn answer() -> i32 { 42 }"
                || prepared.render_doc.line_text(render_line.right_text)
                    == "fn answer() -> i32 { 42 }"
        }));
    }

    #[test]
    fn small_compare_file_selection_stays_synchronous() {
        let mut state = AppState::default();
        let mut output = CompareOutput::default();
        let mut carbon_file = carbon_context_file(0, "src/lib.rs", "fn answer() -> i32 { 42 }");
        carbon_file.additions = 10;
        carbon_file.deletions = 5;
        output.carbon.files = vec![carbon_file];

        state
            .workspace
            .compare_output
            .set(&state.store, Some(output));
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.workspace.files.set(
            &state.store,
            vec![FileListEntry {
                path: "src/lib.rs".to_owned(),
                status: "M".to_owned(),
                additions: 10,
                deletions: 5,
                is_binary: false,
            }],
        );
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state
            .compare
            .repo_path
            .set(&state.store, Some(PathBuf::from("/repo")));

        let effects = state.apply_action(crate::actions::FileListAction::SelectFile(0));

        assert!(effects.iter().any(|effect| {
            matches!(effect, Effect::Syntax(SyntaxEffect::EnsureSyntaxPackForPath { path }) if path == "src/lib.rs")
        }));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Compare(CompareEffect::LoadFile(_))))
        );
        assert!(
            state
                .workspace
                .active_file_loading
                .get(&state.store)
                .is_none()
        );
        assert_eq!(
            state
                .workspace
                .active_file
                .get(&state.store)
                .as_ref()
                .map(|file| file.path.as_str()),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn selecting_large_compare_file_dispatches_async_load() {
        let mut state = loaded_state_with_files(&["src/big.rs"]);
        state.workspace.files.set(
            &state.store,
            vec![FileListEntry {
                path: "src/big.rs".to_owned(),
                status: "M".to_owned(),
                additions: 1_500,
                deletions: 0,
                is_binary: false,
            }],
        );
        state
            .compare
            .repo_path
            .set(&state.store, Some(PathBuf::from("/repo")));
        state.compare.left_ref.set(&state.store, "v5.5".to_owned());
        state.compare.right_ref.set(&state.store, "v5.6".to_owned());
        state
            .compare
            .renderer
            .set(&state.store, RendererKind::Builtin);
        state.compare.layout.set(&state.store, LayoutMode::Unified);
        state.compare.mode.set(&state.store, CompareMode::TwoDot);

        let effects = state.apply_action(crate::actions::FileListAction::SelectFile(0));

        assert!(matches!(
            effects.as_slice(),
            [
                Effect::Syntax(SyntaxEffect::EnsureSyntaxPackForPath { path }),
                Effect::Compare(CompareEffect::LoadFile(task))
            ]
                if path == "src/big.rs"
                    && task.request.index == 0
                    && task.request.path == "src/big.rs"
        ));
        assert_eq!(
            state.workspace.active_file_loading.get(&state.store),
            Some(ActiveFileLoading {
                index: 0,
                path: "src/big.rs".to_owned(),
            })
        );
        assert!(state.workspace.active_file.get(&state.store).is_none());
    }

    #[test]
    fn selecting_deferred_compare_file_dispatches_async_load() {
        let mut state = loaded_state_with_files(&["src/kernel.c"]);
        state
            .workspace
            .compare_output
            .update(&state.store, |output| {
                let file = &mut output.as_mut().expect("compare output").carbon.files[0];
                file.is_partial = true;
                file.hunks.clear();
                file.blocks.clear();
            });
        state
            .compare
            .repo_path
            .set(&state.store, Some(PathBuf::from("/repo")));

        let effects = state.apply_action(crate::actions::FileListAction::SelectFile(0));

        assert!(effects.iter().any(|effect| {
            matches!(
                effect,
                Effect::Compare(CompareEffect::LoadFile(task))
                    if task.request.index == 0
                        && task.request.path == "src/kernel.c"
                        && task.request.deferred_file.as_ref().is_some_and(|file| file.is_partial && file.hunks.is_empty())
            )
        }));
        assert_eq!(
            state.workspace.active_file_loading.get(&state.store),
            Some(ActiveFileLoading {
                index: 0,
                path: "src/kernel.c".to_owned(),
            })
        );
        assert!(state.workspace.active_file.get(&state.store).is_none());
    }

    #[test]
    fn compare_file_finished_ignores_stale_path() {
        let mut state = loaded_state_with_files(&["src/lib.rs"]);
        state.workspace.compare_generation.set(&state.store, 7);
        state
            .workspace
            .selected_file_index
            .set(&state.store, Some(0));
        state
            .workspace
            .selected_file_path
            .set(&state.store, Some("src/lib.rs".to_owned()));
        state.workspace.active_file_loading.set(
            &state.store,
            Some(ActiveFileLoading {
                index: 0,
                path: "src/lib.rs".to_owned(),
            }),
        );

        state.apply_event(AppEvent::from(CompareEvent::CompareFileFinished(
            CompareFileFinished {
                generation: 7,
                index: 0,
                path: "src/other.rs".to_owned(),
                prepared: PreparedActiveFile {
                    carbon_file: carbon::FileDiff::default(),
                    carbon_expansion: carbon::ExpansionState::default(),
                    carbon_overlays: CarbonStyleOverlays::default(),
                    render_doc: RenderDoc::default(),
                    token_buffer: TokenBuffer::default(),
                },
            },
        )));

        assert!(state.workspace.active_file.get(&state.store).is_none());
        assert_eq!(
            state.workspace.active_file_loading.get(&state.store),
            Some(ActiveFileLoading {
                index: 0,
                path: "src/lib.rs".to_owned(),
            })
        );
    }

    #[test]
    fn overlay_list_pixel_scroll_action_clamps_active_overlay() {
        let mut state = AppState::default();
        state.overlays.stack.update(&state.store, |stack| {
            stack.push(super::OverlayEntry {
                surface: OverlaySurface::RepoPicker,
                focus_return: None,
            });
        });
        let picker_entries: Vec<super::PickerEntry> = (0..12)
            .map(|index| super::PickerEntry {
                label: format!("repo-{index}"),
                detail: format!("C:\\repo-{index}"),
                value: format!("C:\\repo-{index}"),
                highlights: Vec::new(),
                icon: None,
                section_header: false,
            })
            .collect();
        state
            .overlays
            .picker
            .entries
            .set(&state.store, picker_entries);
        state
            .overlays
            .picker
            .list
            .update(&state.store, |l| l.viewport_height_px = 120);

        state.apply_action(crate::actions::OverlayAction::ScrollActiveOverlayListPx(50));
        assert_eq!(
            state
                .overlays
                .picker
                .list
                .with(&state.store, |l| l.scroll_top_px),
            50
        );

        state.apply_action(crate::actions::OverlayAction::ScrollActiveOverlayListPx(
            1_000,
        ));
        assert_eq!(
            state
                .overlays
                .picker
                .list
                .with(&state.store, |l| l.scroll_top_px),
            312
        );

        state.apply_action(crate::actions::OverlayAction::ScrollActiveOverlayListPx(
            -1_000,
        ));
        assert_eq!(
            state
                .overlays
                .picker
                .list
                .with(&state.store, |l| l.scroll_top_px),
            0
        );
    }

    #[test]
    fn stage_hunk_at_stages_the_given_index() {
        let mut state = status_state_with_two_hunks();

        let effects = state.apply_action(crate::actions::RepositoryAction::StageHunkAt(1));

        let [Effect::Repository(RepositoryEffect::ApplyPatchOperation(request))] =
            effects.as_slice()
        else {
            panic!("expected one patch effect, got {:?}", effects);
        };
        assert!(request.patch.contains("old_second();"));
        assert!(!request.patch.contains("old_first();"));
    }

    #[test]
    fn stage_hunk_reads_the_hovered_hunk_index() {
        let mut state = status_state_with_two_hunks();
        state.editor.hovered_hunk_index.set(&state.store, Some(1));

        let effects = state.apply_action(crate::actions::RepositoryAction::StageHunk);

        let [Effect::Repository(RepositoryEffect::ApplyPatchOperation(request))] =
            effects.as_slice()
        else {
            panic!("expected one patch effect");
        };
        assert!(request.patch.contains("old_second();"));
    }

    #[test]
    fn status_operation_failure_clears_the_pending_flag() {
        let mut state = status_state_with_two_hunks();
        let _ = state.apply_action(crate::actions::RepositoryAction::StageHunkAt(0));
        assert!(state.workspace.status_operation_pending.get(&state.store));

        let _ = state.apply_event(AppEvent::from(RepositoryEvent::StatusOperationFailed {
            path: PathBuf::from("/repo"),
            message: "patch failed".to_owned(),
        }));

        assert!(!state.workspace.status_operation_pending.get(&state.store));
    }

    #[test]
    fn ref_picker_rebuilds_matches_while_typing_and_keeps_raw_git_revisions_selectable() {
        let mut state = AppState::default();
        state.repository.branches.set(
            &state.store,
            vec![crate::core::vcs::git::BranchInfo {
                name: "main".to_owned(),
                is_remote: false,
                is_head: true,
                upstream: None,
                ahead_behind: None,
            }],
        );

        state.open_ref_picker(CompareField::Left);
        state.apply_action(crate::actions::TextEditAction::InsertText("mai".to_owned()));

        let branch_highlights = state.overlays.picker.entries.with(&state.store, |entries| {
            entries
                .iter()
                .find(|entry| entry.value == "main")
                .expect("main branch entry")
                .highlights
                .clone()
        });
        assert_eq!(branch_highlights, vec![(0, 3)]);

        let mut state = AppState::default();
        state.open_ref_picker(CompareField::Left);
        state.apply_action(crate::actions::TextEditAction::InsertText(
            "HEAD~2".to_owned(),
        ));

        let (typed_value, typed_highlights) =
            state.overlays.picker.entries.with(&state.store, |entries| {
                let typed_entry = entries.first().expect("typed ref entry");
                (typed_entry.value.clone(), typed_entry.highlights.clone())
            });
        assert_eq!(typed_value, "HEAD~2");
        assert_eq!(typed_highlights, vec![(0, "HEAD~2".len())]);

        state.apply_action(crate::actions::OverlayAction::ConfirmOverlaySelection);
        assert_eq!(state.compare.left_ref.get(&state.store), "HEAD~2");
    }

    #[test]
    fn command_palette_uses_actual_match_indices_for_highlighting() {
        let mut state = AppState::default();
        state
            .overlays
            .command_palette
            .query
            .set(&state.store, "them".to_owned());

        state.rebuild_command_palette();

        let highlights = state
            .overlays
            .command_palette
            .entries
            .with(&state.store, |entries| {
                entries
                    .iter()
                    .find(|entry| entry.label == "Change Theme")
                    .expect("Change Theme entry")
                    .highlights
                    .clone()
            });
        assert_eq!(highlights, vec![(7, 11)]);
    }

    #[test]
    fn sidebar_width_action_clamps_and_stores_manual_preference() {
        let mut state = AppState::default();

        state.apply_action(crate::actions::SettingsAction::SetSidebarWidthPx(40));
        assert_eq!(state.settings.sidebar_width_px, Some(179));

        state.apply_action(crate::actions::SettingsAction::SetSidebarWidthPx(420));
        assert_eq!(state.settings.sidebar_width_px, Some(420));
    }

    #[test]
    fn ui_scale_actions_step_and_persist_within_bounds() {
        let mut state = AppState::default();

        let effects = state.apply_action(crate::actions::SettingsAction::IncreaseUiScale);
        assert_eq!(state.settings.ui_scale_pct, 110);
        assert_eq!(effects.len(), 1);

        for _ in 0..20 {
            state.apply_action(crate::actions::SettingsAction::IncreaseUiScale);
        }
        assert_eq!(state.settings.ui_scale_pct, 180);

        for _ in 0..20 {
            state.apply_action(crate::actions::SettingsAction::DecreaseUiScale);
        }
        assert_eq!(state.settings.ui_scale_pct, 70);
    }

    #[test]
    fn avatar_url_sized_appends_or_replaces_s_param() {
        use super::avatar_url_sized;
        assert_eq!(
            avatar_url_sized("https://avatars.githubusercontent.com/u/1?v=4", 128),
            Some("https://avatars.githubusercontent.com/u/1?v=4&s=128".to_owned())
        );
        assert_eq!(
            avatar_url_sized("https://avatars.githubusercontent.com/u/1", 64),
            Some("https://avatars.githubusercontent.com/u/1?s=64".to_owned())
        );
        assert_eq!(
            avatar_url_sized("https://avatars.githubusercontent.com/u/1?s=40&v=4", 128),
            Some("https://avatars.githubusercontent.com/u/1?v=4&s=128".to_owned())
        );
        assert_eq!(avatar_url_sized("", 128), None);
    }

    #[test]
    fn command_palette_detects_pr_url_and_emits_peek_effect() {
        let mut state = AppState::default();
        state.overlays.command_palette.query.set(
            &state.store,
            "https://github.com/foo/bar/pull/42".to_owned(),
        );

        let effects = state.rebuild_command_palette();

        // A peek effect was fired for the parsed key.
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::GitHub(GitHubEffect::PeekPullRequest {
                owner, repo, number, ..
            }) if owner == "foo" && repo == "bar" && *number == 42
        )));

        // Palette has the synthesized PR entry as the top row with key intact.
        let top = state
            .overlays
            .command_palette
            .entries
            .with(&state.store, |e| e.first().cloned())
            .expect("palette has at least one entry");
        assert!(matches!(
            top.kind,
            super::PaletteEntryKind::PullRequest((ref o, ref r, n))
            if o == "foo" && r == "bar" && n == 42
        ));

        // Cache entry is initialized to Loading.
        let cached = state.github.pull_request.cache.with(&state.store, |c| {
            c.get(&("foo".to_owned(), "bar".to_owned(), 42)).cloned()
        });
        let cached = cached.expect("cache entry");
        assert!(matches!(cached.meta, super::PrPeekMeta::Loading));
    }

    #[test]
    fn pr_peeked_event_transitions_cache_meta_to_ready() {
        use crate::core::vcs::github::PullRequestInfo;
        use crate::events::AppEvent;

        let mut state = AppState::default();
        state
            .overlays
            .command_palette
            .query
            .set(&state.store, "https://github.com/foo/bar/pull/7".to_owned());
        let _ = state.rebuild_command_palette();

        let info = PullRequestInfo {
            title: "Fix thing".to_owned(),
            state: "open".to_owned(),
            author_login: "alice".to_owned(),
            number: 7,
            additions: 12,
            deletions: 3,
            changed_files: 1,
            base_branch: "main".to_owned(),
            head_branch: "fix".to_owned(),
            base_sha: "a".to_owned(),
            head_sha: "b".to_owned(),
            base_repo_url: String::new(),
            head_repo_url: String::new(),
        };
        state.apply_event(AppEvent::from(GitHubEvent::PullRequestPeeked {
            owner: "foo".to_owned(),
            repo: "bar".to_owned(),
            number: 7,
            info: info.clone(),
        }));

        let meta = state.github.pull_request.cache.with(&state.store, |c| {
            c.get(&("foo".to_owned(), "bar".to_owned(), 7))
                .map(|e| e.meta.clone())
        });
        assert!(matches!(meta, Some(super::PrPeekMeta::Ready(_))));
    }

    // -----------------------------------------------------------------
    // Compare progress — end-to-end through the event lifecycle
    // -----------------------------------------------------------------

    use super::{ComparePhase, CompareProgress, LoadingSubject};
    use crate::core::compare::CompareSpec;
    use crate::events::{CompareFinished, RepositorySyncReason};

    fn compare_ready_state() -> AppState {
        let state = AppState::default();
        state
            .compare
            .repo_path
            .set(&state.store, Some(PathBuf::from("/repo")));
        state.compare.left_ref.set(&state.store, "v5.0".to_owned());
        state.compare.right_ref.set(&state.store, "v5.1".to_owned());
        state.compare.mode.set(&state.store, CompareMode::TwoDot);
        state
    }

    #[test]
    fn kickoff_compare_seeds_progress_with_labels_and_started_at() {
        let mut state = compare_ready_state();
        state.clock_ms = 1_000;
        let _ = state.kickoff_compare();

        let progress = state
            .compare_progress
            .with(&state.store, |p| p.clone())
            .expect("progress should be populated");
        match &progress.subject {
            LoadingSubject::Compare {
                left_label,
                right_label,
            } => {
                assert_eq!(left_label, "v5.0");
                assert_eq!(right_label, "v5.1");
            }
            other => panic!("expected Compare subject, got {other:?}"),
        }
        assert_eq!(progress.started_at_ms, 1_000);
        assert_eq!(progress.phase, ComparePhase::OpeningRepo);
        assert_eq!(progress.file_count_total, None);
        assert_eq!(
            state.workspace_mode.get(&state.store),
            WorkspaceMode::Loading,
            "viewport should flip to loading so the panel actually renders"
        );
    }

    #[test]
    fn compare_progress_update_applies_only_when_generation_matches() {
        let mut state = compare_ready_state();
        let _ = state.kickoff_compare();
        let generation = state.workspace.compare_generation.get(&state.store);

        // Stale reporter — must be ignored.
        state.apply_event(AppEvent::from(CompareEvent::CompareProgressUpdate {
            generation: generation.wrapping_sub(1),
            phase: ComparePhase::EnumeratingChanges,
        }));
        assert_eq!(
            state
                .compare_progress
                .with(&state.store, |p| p.as_ref().unwrap().phase),
            ComparePhase::OpeningRepo,
            "stale generation must not advance the phase"
        );

        // Fresh reporter — applies.
        state.apply_event(AppEvent::from(CompareEvent::CompareProgressUpdate {
            generation,
            phase: ComparePhase::EnumeratingChanges,
        }));
        assert_eq!(
            state
                .compare_progress
                .with(&state.store, |p| p.as_ref().unwrap().phase),
            ComparePhase::EnumeratingChanges,
        );
    }

    #[test]
    fn loading_files_phase_updates_counts_on_struct() {
        let mut state = compare_ready_state();
        let _ = state.kickoff_compare();
        let generation = state.workspace.compare_generation.get(&state.store);

        state.apply_event(AppEvent::from(CompareEvent::CompareProgressUpdate {
            generation,
            phase: ComparePhase::LoadingFiles {
                files_seen: 142,
                files_total: 3_891,
            },
        }));

        let progress = state
            .compare_progress
            .with(&state.store, |p| p.clone())
            .expect("progress exists");
        assert_eq!(progress.files_loaded, 142);
        assert_eq!(progress.file_count_total, Some(3_891));
        assert!(matches!(progress.phase, ComparePhase::LoadingFiles { .. }));
    }

    #[test]
    fn kickoff_with_prior_state_delays_reveal_by_500ms() {
        let mut state = compare_ready_state();
        // Simulate a previously loaded compare (files present).
        state.workspace.files.set(
            &state.store,
            vec![FileListEntry {
                path: "old.rs".into(),
                status: "M".into(),
                additions: 0,
                deletions: 0,
                is_binary: false,
            }],
        );
        state.clock_ms = 10_000;

        let _ = state.kickoff_compare();
        let progress = state
            .compare_progress
            .with(&state.store, |p| p.clone())
            .expect("progress populated");
        assert_eq!(progress.started_at_ms, 10_000);
        assert_eq!(
            progress.reveal_at_ms, 10_500,
            "re-compare with prior state delays reveal by COMPARE_REVEAL_DELAY_MS"
        );
        // Workspace mode stays as-is so the old diff remains visible during
        // the grace period.
        assert_ne!(
            state.workspace_mode.get(&state.store),
            WorkspaceMode::Loading
        );
        // Prior files are preserved so fast compares don't cause a flash.
        assert_eq!(state.workspace.files.with(&state.store, |f| f.len()), 1);
    }

    #[test]
    fn open_repository_seeds_repo_subject_progress() {
        let mut state = AppState::default();
        state.clock_ms = 500;

        let effects = state.open_repository(PathBuf::from("/tmp/linux"));

        let progress = state
            .compare_progress
            .with(&state.store, |p| p.clone())
            .expect("progress seeded for repo open");
        match progress.subject {
            LoadingSubject::RepoOpen { ref name } => {
                assert_eq!(name, "linux");
            }
            other => panic!("expected RepoOpen subject, got {other:?}"),
        }
        assert_eq!(progress.phase, ComparePhase::OpeningRepo);
        assert_eq!(
            progress.reveal_at_ms,
            500 + super::COMPARE_REVEAL_DELAY_MS,
            "every repo open delays reveal so sub-threshold opens don't flash"
        );
        // Reporter generation is threaded through the SyncRepository effect
        // so the worker's phase events stamp the matching generation.
        let sync_gen = effects.iter().find_map(|eff| match eff {
            Effect::Repository(RepositoryEffect::SyncRepository {
                reporter_generation,
                ..
            }) => *reporter_generation,
            _ => None,
        });
        assert_eq!(sync_gen, Some(progress.generation));
    }

    #[test]
    fn open_repository_with_prior_diff_delays_reveal() {
        let mut state = AppState::default();
        state.workspace.files.set(
            &state.store,
            vec![FileListEntry {
                path: "old.rs".into(),
                status: "M".into(),
                additions: 0,
                deletions: 0,
                is_binary: false,
            }],
        );
        state.clock_ms = 10_000;

        let _ = state.open_repository(PathBuf::from("/tmp/other"));

        let progress = state
            .compare_progress
            .with(&state.store, |p| p.clone())
            .expect("progress seeded");
        assert_eq!(
            progress.reveal_at_ms, 10_500,
            "re-open with prior diff delays reveal by COMPARE_REVEAL_DELAY_MS"
        );
    }

    #[test]
    fn repository_snapshot_ready_clears_repo_open_progress() {
        use super::StatusItem;
        let mut state = AppState::default();
        let path = PathBuf::from("/tmp/linux");
        let _ = state.open_repository(path.clone());
        assert!(state.compare_progress.with(&state.store, |p| p.is_some()));

        state.apply_event(AppEvent::from(RepositoryEvent::RepositorySnapshotReady(
            crate::events::RepositorySnapshot {
                path,
                reason: RepositorySyncReason::Open,
                change_kind: None,
                branches: Vec::new(),
                tags: Vec::new(),
                commits: Vec::new(),
                status_items: Vec::<StatusItem>::new(),
            },
        )));

        assert!(
            state.compare_progress.with(&state.store, |p| p.is_none()),
            "snapshot-ready must tear down the repo-open progress panel"
        );
    }

    #[test]
    fn kickoff_without_prior_state_also_delays_reveal() {
        let mut state = compare_ready_state();
        state.clock_ms = 5_000;

        let _ = state.kickoff_compare();
        let progress = state
            .compare_progress
            .with(&state.store, |p| p.clone())
            .expect("progress populated");
        assert_eq!(progress.started_at_ms, 5_000);
        assert_eq!(
            progress.reveal_at_ms,
            5_000 + super::COMPARE_REVEAL_DELAY_MS,
            "every compare delays reveal so fast ops skip the loading flash"
        );
        // With no prior state to preserve, workspace_mode flips to Loading
        // up front so the editor/ready-hint stops rendering in the background.
        assert_eq!(
            state.workspace_mode.get(&state.store),
            WorkspaceMode::Loading
        );
    }

    #[test]
    fn cancel_compare_bumps_generation_and_drops_stale_result() {
        let mut state = compare_ready_state();
        let _ = state.kickoff_compare();
        let generation = state.workspace.compare_generation.get(&state.store);

        let _ = state.cancel_compare();

        assert!(
            state.compare_progress.with(&state.store, |p| p.is_none()),
            "progress should be cleared after cancel"
        );
        let new_gen = state.workspace.compare_generation.get(&state.store);
        assert!(new_gen > generation, "generation should be bumped");
        assert_eq!(
            state.workspace_mode.get(&state.store),
            WorkspaceMode::Empty,
            "fresh-state cancel should revert the Loading flip"
        );

        // A stale CompareFinished arriving after cancel must be silently dropped.
        state.apply_event(AppEvent::from(CompareEvent::CompareFinished(
            CompareFinished {
                generation,
                spec: CompareSpec {
                    mode: CompareMode::TwoDot,
                    left_ref: "v5.0".to_owned(),
                    right_ref: "v5.1".to_owned(),
                    renderer: RendererKind::Builtin,
                    layout: LayoutMode::Unified,
                },
                resolved_left: "deadbeef".to_owned(),
                resolved_right: "cafefeed".to_owned(),
                output: CompareOutput::default(),
                range_commits: Vec::new(),
            },
        )));
        assert_eq!(
            state.workspace_mode.get(&state.store),
            WorkspaceMode::Empty,
            "stale finished result must not promote workspace to Ready",
        );
        assert!(
            state.compare_progress.with(&state.store, |p| p.is_none()),
            "stale finished result must not re-seed progress",
        );
    }

    #[test]
    fn cancel_compare_preserves_previous_diff_on_recompare() {
        let mut state = compare_ready_state();
        // Prior state: an existing file in the workspace.
        state.workspace.files.set(
            &state.store,
            vec![FileListEntry {
                path: "old.rs".into(),
                status: "M".into(),
                additions: 0,
                deletions: 0,
                is_binary: false,
            }],
        );
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);

        let _ = state.kickoff_compare();
        let _ = state.cancel_compare();

        assert!(
            state.compare_progress.with(&state.store, |p| p.is_none()),
            "progress cleared on cancel"
        );
        assert_eq!(
            state.workspace_mode.get(&state.store),
            WorkspaceMode::Ready,
            "previous workspace state is preserved on cancel — no blanking"
        );
        assert_eq!(
            state.workspace.files.with(&state.store, |f| f.len()),
            1,
            "prior file list must not be wiped by cancel"
        );
    }

    #[test]
    fn compare_finished_advances_phase_and_records_file_count() {
        let mut state = compare_ready_state();
        let _ = state.kickoff_compare();
        let generation = state.workspace.compare_generation.get(&state.store);

        // Simulate a successful compare with 3 files.
        let files = ["a.rs", "b.rs", "c.rs"];
        let output = CompareOutput {
            carbon: carbon::DiffDocument {
                files: files
                    .iter()
                    .enumerate()
                    .map(|(index, path)| carbon_summary_for_path(index, path))
                    .collect(),
            },
            ..CompareOutput::default()
        };

        state.apply_event(AppEvent::from(CompareEvent::CompareFinished(
            CompareFinished {
                generation,
                spec: CompareSpec {
                    mode: CompareMode::TwoDot,
                    left_ref: "v5.0".to_owned(),
                    right_ref: "v5.1".to_owned(),
                    renderer: RendererKind::Builtin,
                    layout: LayoutMode::Unified,
                },
                resolved_left: "deadbeef".to_owned(),
                resolved_right: "cafefeed".to_owned(),
                output,
                range_commits: Vec::new(),
            },
        )));

        // Small files load synchronously, so progress is already cleared by the
        // time handle_compare_finished returns. We at least know the workspace
        // is Ready and files are populated.
        assert_eq!(state.workspace_mode.get(&state.store), WorkspaceMode::Ready,);
        assert_eq!(state.workspace.files.with(&state.store, |f| f.len()), 3,);
    }

    #[test]
    fn compare_failed_clears_progress_and_marks_workspace_empty() {
        let mut state = compare_ready_state();
        let _ = state.kickoff_compare();
        let generation = state.workspace.compare_generation.get(&state.store);

        state.apply_event(AppEvent::from(CompareEvent::CompareFailed {
            generation,
            message: "boom".to_owned(),
        }));

        assert_eq!(state.workspace_mode.get(&state.store), WorkspaceMode::Empty,);
        assert!(
            state.compare_progress.with(&state.store, |p| p.is_none()),
            "progress panel must tear down on compare failure",
        );
    }

    #[test]
    fn compare_progress_label_does_not_panic_for_all_phases() {
        // Non-empty labels matter for the title-bar fallback. Cheap to
        // check exhaustively.
        let phases = [
            ComparePhase::OpeningRepo,
            ComparePhase::ResolvingRefs,
            ComparePhase::EnumeratingChanges,
            ComparePhase::LoadingFiles {
                files_seen: 142,
                files_total: 3_891,
            },
            ComparePhase::FetchingHistory,
            ComparePhase::PopulatingList,
            ComparePhase::RenderingFirstFile,
        ];
        for phase in phases {
            let label = phase.label();
            assert!(!label.is_empty());
        }
        // LoadingFiles label should interpolate counts.
        assert!(
            ComparePhase::LoadingFiles {
                files_seen: 142,
                files_total: 3_891,
            }
            .label()
            .contains("142"),
            "file counts must appear in the label"
        );

        let _ = CompareProgress {
            generation: 0,
            phase: ComparePhase::default(),
            subject: LoadingSubject::Compare {
                left_label: String::new(),
                right_label: String::new(),
            },
            started_at_ms: 0,
            reveal_at_ms: 0,
            file_count_total: None,
            files_loaded: 0,
        };
    }
}
