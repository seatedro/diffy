mod text_edit;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use halogen::Store;
use halogen::reactive::{Signal, SignalStore};

use crate::actions::Action;
use crate::core::compare::{CompareMode, CompareOutput, CompareSpec, LayoutMode, RendererKind};
use crate::core::diff::FileDiff;
use crate::core::frecency::FrecencyStore;
use crate::core::syntax::DiffSyntaxAnnotator;
use crate::core::text::TextBuffer;
use crate::core::vcs::git::patch;
use crate::core::vcs::git::{
    BranchInfo, CommitInfo, StatusItem, StatusOperation, StatusScope, TagInfo,
};
use crate::core::vcs::github::{DeviceFlowState, PullRequestInfo};
use crate::editor::Editor;
use crate::effects::{
    BatchStatusOperationRequest, CommitRequest, CompareRequest, Effect, PatchOperationRequest,
    StatusDiffRequest, StatusOperationRequest,
};
use crate::events::{
    AppEvent, CompareFinished, RepositoryChangeKind, RepositorySnapshot, RepositorySyncReason,
    StatusDiffFinished,
};
use crate::platform::persistence::{PersistedCompare, Settings};
use crate::platform::startup::StartupOptions;
use crate::ui::design::{Sp, Sz};
use crate::ui::editor::render_doc::{RenderDoc, build_render_doc};
use crate::ui::editor::state::{EditorState, EditorStateStore, SearchMatch};
use crate::ui::icons::lucide;
use crate::ui::theme::ThemeMode;

const MAX_VISIBLE_TOASTS: usize = 5;
const TOAST_LIFETIME_MS: u64 = 5_000;
const TOAST_ANIM_MS: u64 = 150;
const CURSOR_BLINK_INTERVAL_MS: u64 = 530;

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
    PullRequestInput,
    PullRequestConfirm,
    AuthPrimaryAction,
    SidebarSearch,
    SearchInput,
    CommitEditor,
}

impl FocusTarget {
    pub fn is_text_field(self) -> bool {
        matches!(
            self,
            Self::PickerInput
                | Self::CommandPaletteInput
                | Self::PullRequestInput
                | Self::SidebarSearch
                | Self::SearchInput
                | Self::CommitEditor
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

#[derive(Debug, Clone)]
pub struct ActiveFile {
    pub index: usize,
    pub path: String,
    pub file: FileDiff,
    pub render_doc: RenderDoc,
    pub text_buffer: TextBuffer,
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
    pub compare_generation: u64,
    pub status_generation: u64,
    pub files: Vec<FileListEntry>,
    pub status_items: Vec<StatusItem>,
    pub selected_file_index: Option<usize>,
    pub selected_file_path: Option<String>,
    pub selected_status_scope: Option<StatusScope>,
    pub compare_output: Option<CompareOutput>,
    pub active_file: Option<ActiveFile>,
    pub raw_diff_len: usize,
    pub used_fallback: bool,
    pub fallback_message: String,
    pub sidebar_auto_width: Option<SidebarWidthCache>,
    pub range_commits: Vec<CommitInfo>,
    pub pre_drill_compare: Option<(String, String, CompareMode)>,
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
        self.file_list
            .viewed_files
            .set(&self.store, d.viewed_files);
    }

    pub fn sidebar_row_count(&self) -> usize {
        if self.workspace.source.get(&self.store) == WorkspaceSource::Status
            && self
                .file_list
                .filter
                .with(&self.store, |s| s.is_empty())
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
            || !self
                .file_list
                .filter
                .with(&self.store, |s| s.is_empty())
        {
            return index;
        }
        index
            + self
                .workspace
                .status_items
                .with(&self.store, |s| status_section_count_before(s, index + 1))
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
    OpenPullRequestModal,
    OpenGitHubAuthModal,
    FocusFileList,
    FocusViewport,
    ToggleWrap,
    ToggleThemeMode,
    ChangeTheme,
    SetLayout(LayoutMode),
    SetTheme(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEntryKind {
    Command(PaletteCommand),
    File(usize),
    Repo(PathBuf),
    Ref(CompareField, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    pub label: String,
    pub detail: String,
    pub kind: PaletteEntryKind,
    pub highlights: Vec<(usize, usize)>,
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct CommandPaletteState {
    pub query: String,
    pub entries: Vec<PaletteEntry>,
    pub selected_index: usize,
    pub list: OverlayListState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct PullRequestState {
    pub status: AsyncStatus,
    pub url_input: String,
    pub info: Option<PullRequestInfo>,
    pub candidate_left_ref: Option<String>,
    pub candidate_right_ref: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct GitHubAuthState {
    pub status: AsyncStatus,
    pub device_flow: Option<DeviceFlowState>,
    pub token_present: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct GitHubState {
    pub client_id: String,
    #[store(flatten)]
    pub auth: GitHubAuthState,
    #[store(flatten)]
    pub pull_request: PullRequestState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlaySurface {
    RepoPicker,
    RefPicker(CompareField),
    CommandPalette,
    PullRequestModal,
    GitHubAuthModal,
    KeyboardShortcuts,
    ThemePicker,
    CompareMenu,
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
        self.overlays
            .command_palette
            .list
            .set(&self.store, d.list);
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
    pub created_at_ms: u64,
    pub hovered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartupState {
    pub auto_compare_pending: bool,
    pub pending_pr_url: Option<String>,
    pub preferred_file_index: Option<usize>,
    pub preferred_file_path: Option<String>,
    pub hidden_window: bool,
    pub exit_after: Option<Duration>,
    pub dump_state_json: Option<PathBuf>,
    pub dump_files_json: Option<PathBuf>,
    pub dump_errors_json: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct DebugState {
    pub last_scene_primitive_count: usize,
    pub last_frame_time_us: u64,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub workspace_mode: Signal<WorkspaceMode>,
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
    pub animation: crate::ui::animation::AnimationState,
    pub commit_editor: Editor,
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
}

impl Default for AppState {
    fn default() -> Self {
        let store = Rc::new(SignalStore::default());
        let sidebar_visible = store.create(true);
        let focus = store.create(None::<FocusTarget>);
        let workspace_mode = store.create(WorkspaceMode::default());
        let last_error = store.create(None::<String>);
        let theme_preview_original = store.create(None::<String>);
        let toasts = store.create(Vec::<Toast>::new());
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
            animation: crate::ui::animation::AnimationState::default(),
            commit_editor: Editor::default(),
            sidebar_visible,
            debug,
            store,
            clock_ms: 0,
            next_toast_id: 1,
            frecency: None,
            theme_names: Vec::new(),
            theme_variants: Vec::new(),
            theme_preview_original,
        }
    }
}

impl AppState {
    pub fn bootstrap(startup: StartupOptions, mut settings: Settings) -> (Self, Vec<Effect>) {
        if startup.github_token.is_some() {
            settings.github_token = startup.github_token.clone();
        }

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
        let workspace_mode = store.create(if repo_path.is_some() && auto_compare_pending {
            WorkspaceMode::Loading
        } else {
            WorkspaceMode::Empty
        });
        let last_error = store.create(None::<String>);
        let theme_preview_original = store.create(None::<String>);
        let toasts = store.create(Vec::<Toast>::new());
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
        let github = GitHubStateStore::new(
            &store,
            GitHubState {
                client_id: startup.github_client_id.clone(),
                auth: GitHubAuthState {
                    token_present: settings.github_token.is_some(),
                    ..GitHubAuthState::default()
                },
                pull_request: PullRequestState {
                    url_input: startup.args.open_pr.clone().unwrap_or_default(),
                    ..PullRequestState::default()
                },
            },
        );
        let mut state = Self {
            workspace_mode,
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
                hidden_window: startup.hidden_window(),
                exit_after: startup.exit_after(),
                dump_state_json: startup.args.dump_state_json.clone(),
                dump_files_json: startup.args.dump_files_json.clone(),
                dump_errors_json: startup.args.dump_errors_json.clone(),
            },
            last_error,
            toasts,
            animation: crate::ui::animation::AnimationState::default(),
            commit_editor: Editor::default(),
            sidebar_visible,
            debug,
            store,
            clock_ms: 0,
            next_toast_id: 1,
            frecency: crate::core::frecency::open_default_store(),
            theme_names: Vec::new(),
            theme_variants: Vec::new(),
            theme_preview_original,
        };
        state.sync_settings_snapshot();

        let mut effects = Vec::new();
        if let Some(path) = repo_path {
            state.repository.status.set(&state.store, AsyncStatus::Loading);
            effects.push(Effect::SyncRepository {
                path: path.clone(),
                reason: RepositorySyncReason::Open,
            });
            effects.push(Effect::WatchRepository { path: Some(path) });
        }
        (state, effects)
    }

    pub fn apply_action(&mut self, action: Action) -> Vec<Effect> {
        use Action::*;
        match action {
            // Text editing
            InsertText(_)
            | Backspace
            | BackspaceWord
            | BackspaceLine
            | DeleteForward
            | DeleteForwardWord
            | CursorLeft
            | CursorRight
            | CursorUp
            | CursorDown
            | CursorWordLeft
            | CursorWordRight
            | CursorHome
            | CursorEnd
            | CursorSoftHome
            | CursorSoftEnd
            | SelectLeft
            | SelectRight
            | SelectUp
            | SelectDown
            | SelectWordLeft
            | SelectWordRight
            | SelectHome
            | SelectEnd
            | SelectSoftHome
            | SelectSoftEnd
            | SelectAll
            | Copy
            | Cut
            | Paste(_)
            | SetTextCursor(_)
            | ExtendTextSelection(_) => self.apply_text_edit_action(action),

            // Overlay management
            OpenRepoPicker
            | OpenThemePicker
            | OpenRefPicker(_)
            | OpenCommandPalette
            | OpenPullRequestModal
            | OpenGitHubAuthModal
            | CloseOverlay
            | MoveOverlaySelection(_)
            | ConfirmOverlaySelection
            | TabCompletePickerDir
            | SelectOverlayEntry(_)
            | HoverOverlayEntry(_)
            | ScrollActiveOverlayListPx(_)
            | ShowKeyboardShortcuts => self.apply_overlay_action(action),

            // Compare & repository
            Bootstrap
            | OpenRepositoryDialog
            | OpenRepository(_)
            | SetLeftRef(_)
            | SetRightRef(_)
            | SetCompareMode(_)
            | CycleCompareMode
            | OpenCompareMenu
            | ApplyComparePreset(_)
            | SetLayoutMode(_)
            | SetRenderer(_)
            | StartCompare
            | StageSelectedFile
            | UnstageSelectedFile
            | DiscardSelectedFile
            | StageFile(_)
            | UnstageFile(_)
            | StageAllFiles
            | UnstageAllFiles
            | StageHunk
            | UnstageHunk
            | DiscardHunk
            | ToggleLineSelection(_)
            | ToggleLineSelectionRange(_, _)
            | StageSelectedLines
            | UnstageSelectedLines
            | DiscardSelectedLines
            | ClearLineSelection
            | ShowWorkingTree
            | SubmitCommit => self.apply_compare_action(action),

            // File list & sidebar
            SelectFile(_)
            | SelectFilePath(_)
            | SelectNextFile
            | SelectPreviousFile
            | ScrollFileList(_)
            | ScrollFileListPx(_)
            | ScrollFileListToPx(_)
            | HoverFile(_)
            | ToggleFolder(_)
            | ToggleFileViewed(_)
            | SetSidebarFilter(_)
            | ClearSidebarFilter
            | ToggleSidebarMode
            | ToggleSidebar
            | SetSidebarTab(_)
            | ScrollCommitListPx(_)
            | ExpandAllFolders
            | CollapseAllFolders => self.apply_file_list_action(action),

            SelectSidebarCommit(_) | ClearSidebarCommit => self.apply_compare_action(action),

            // Viewport & editor navigation
            ScrollViewportLines(_)
            | ScrollViewportPx(_)
            | ScrollViewportPages(_)
            | ScrollViewportTo(_)
            | ScrollViewportHalfPage(_)
            | HoverViewportRow(_)
            | FocusViewport
            | GoToNextHunk
            | GoToPreviousHunk
            | GoToNextFile
            | GoToPreviousFile
            | OpenSearch
            | CloseSearch
            | SearchNext
            | SearchPrevious => self.apply_navigation_action(action),

            // Settings & UI
            ToggleWrap | SetWrapColumn(_) | SetSidebarWidthPx(_) | IncreaseUiScale
            | DecreaseUiScale | ToggleThemeMode | SetThemeName(_) => {
                self.apply_settings_action(action)
            }

            // GitHub
            SubmitPullRequest
            | UsePullRequestCompare
            | StartGitHubDeviceFlow
            | OpenDeviceFlowBrowser => self.apply_github_action(action),

            // Focus & misc
            SetFocus(target) => {
                self.set_focus(target);
                Vec::new()
            }
            DismissToast(index) => {
                self.toasts.update(&self.store, |toasts| {
                    if index < toasts.len() {
                        toasts.remove(index);
                    }
                });
                Vec::new()
            }
            HoverToast(index) => {
                let mut was_any_hovered = false;
                let mut is_any_hovered = false;
                self.toasts.update(&self.store, |toasts| {
                    was_any_hovered = toasts.iter().any(|t| t.hovered);
                    let hovered_id = index.and_then(|i| toasts.get(i)).map(|t| t.id);
                    for toast in toasts.iter_mut() {
                        toast.hovered = Some(toast.id) == hovered_id;
                    }
                    is_any_hovered = toasts.iter().any(|t| t.hovered);
                });
                if was_any_hovered != is_any_hovered {
                    use crate::ui::animation::AnimationKey;
                    let target = if is_any_hovered { 1.0 } else { 0.0 };
                    self.animation.set_target(
                        AnimationKey::ToastStackFan,
                        target,
                        150,
                        self.clock_ms,
                    );
                }
                Vec::new()
            }
            EditorClick(x, y) => {
                self.commit_editor.click(x, y);
                Vec::new()
            }
            EditorDrag(x, y) => {
                self.commit_editor.drag(x, y);
                Vec::new()
            }
            EditorScrollPx(delta) => {
                self.commit_editor.scroll(delta as f32);
                Vec::new()
            }
            Noop => Vec::new(),
        }
    }

    fn apply_text_edit_action(&mut self, action: Action) -> Vec<Effect> {
        if self.focus.get(&self.store) == Some(FocusTarget::CommitEditor) {
            return self.apply_commit_editor_action(action);
        }
        match action {
            Action::InsertText(value) => self.insert_text(value),
            Action::Backspace => self.backspace(),
            Action::DeleteForward => self.delete_forward(),
            Action::CursorLeft => {
                self.cursor_left(false);
                Vec::new()
            }
            Action::CursorRight => {
                self.cursor_right(false);
                Vec::new()
            }
            Action::CursorWordLeft => {
                self.cursor_word_left(false);
                Vec::new()
            }
            Action::CursorWordRight => {
                self.cursor_word_right(false);
                Vec::new()
            }
            Action::CursorHome => {
                self.cursor_home(false);
                Vec::new()
            }
            Action::CursorEnd => {
                self.cursor_end(false);
                Vec::new()
            }
            Action::SelectLeft => {
                self.cursor_left(true);
                Vec::new()
            }
            Action::SelectRight => {
                self.cursor_right(true);
                Vec::new()
            }
            Action::SelectWordLeft => {
                self.cursor_word_left(true);
                Vec::new()
            }
            Action::SelectWordRight => {
                self.cursor_word_right(true);
                Vec::new()
            }
            Action::SelectHome => {
                self.cursor_home(true);
                Vec::new()
            }
            Action::SelectEnd => {
                self.cursor_end(true);
                Vec::new()
            }
            Action::SelectAll => {
                self.select_all();
                Vec::new()
            }
            Action::Copy => {
                let (effects, copied) = self.copy_selection();
                if let Some(value) = copied {
                    let truncated = if value.len() > 32 {
                        format!("{}…", &value[..32])
                    } else {
                        value
                    };
                    self.push_info(&format!("Copied {truncated}"));
                }
                effects
            }
            Action::Cut => self.cut_selection(),
            Action::Paste(value) => self.paste(value),
            Action::SetTextCursor(offset) => {
                self.move_cursor(offset, false);
                Vec::new()
            }
            Action::ExtendTextSelection(offset) => {
                self.move_cursor(offset, true);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn apply_commit_editor_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::InsertText(value) => self.commit_editor.insert_text(&value),
            Action::Backspace => self.commit_editor.delete_backward(),
            Action::BackspaceWord => self.commit_editor.delete_backward_word(),
            Action::BackspaceLine => self.commit_editor.delete_backward_line(),
            Action::DeleteForward => self.commit_editor.delete_forward(),
            Action::DeleteForwardWord => self.commit_editor.delete_forward_word(),
            Action::CursorLeft => self.commit_editor.move_left(false),
            Action::CursorRight => self.commit_editor.move_right(false),
            Action::CursorUp => self.commit_editor.move_up(false),
            Action::CursorDown => self.commit_editor.move_down(false),
            Action::CursorWordLeft => self.commit_editor.move_word_left(false),
            Action::CursorWordRight => self.commit_editor.move_word_right(false),
            Action::CursorHome => self.commit_editor.move_home(false),
            Action::CursorEnd => self.commit_editor.move_end(false),
            Action::CursorSoftHome => self.commit_editor.move_soft_home(false),
            Action::CursorSoftEnd => self.commit_editor.move_soft_end(false),
            Action::SelectLeft => self.commit_editor.move_left(true),
            Action::SelectRight => self.commit_editor.move_right(true),
            Action::SelectUp => self.commit_editor.move_up(true),
            Action::SelectDown => self.commit_editor.move_down(true),
            Action::SelectWordLeft => self.commit_editor.move_word_left(true),
            Action::SelectWordRight => self.commit_editor.move_word_right(true),
            Action::SelectHome => self.commit_editor.move_home(true),
            Action::SelectEnd => self.commit_editor.move_end(true),
            Action::SelectSoftHome => self.commit_editor.move_soft_home(true),
            Action::SelectSoftEnd => self.commit_editor.move_soft_end(true),
            Action::SelectAll => self.commit_editor.select_all(),
            Action::Copy => {
                if let Some(text) = self.commit_editor.selected_text() {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                }
            }
            Action::Cut => {
                if let Some(text) = self.commit_editor.selected_text() {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                    self.commit_editor.delete_backward();
                }
            }
            Action::Paste(value) => self.commit_editor.insert_text(&value),
            _ => {}
        }
        Vec::new()
    }

    fn apply_overlay_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::OpenRepoPicker => {
                self.open_repo_picker();
                Vec::new()
            }
            Action::OpenThemePicker => {
                self.open_theme_picker();
                Vec::new()
            }
            Action::OpenRefPicker(field) => self.open_ref_picker(field),
            Action::OpenCommandPalette => {
                self.open_command_palette();
                Vec::new()
            }
            Action::OpenPullRequestModal => {
                self.push_overlay(
                    OverlaySurface::PullRequestModal,
                    Some(FocusTarget::PullRequestInput),
                );
                Vec::new()
            }
            Action::OpenGitHubAuthModal => {
                self.push_overlay(
                    OverlaySurface::GitHubAuthModal,
                    Some(FocusTarget::AuthPrimaryAction),
                );
                Vec::new()
            }
            Action::CloseOverlay => {
                self.pop_overlay();
                Vec::new()
            }
            Action::MoveOverlaySelection(delta) => {
                self.move_overlay_selection(delta);
                Vec::new()
            }
            Action::ConfirmOverlaySelection => self.confirm_overlay_selection(),
            Action::TabCompletePickerDir => {
                self.tab_complete_picker_dir();
                Vec::new()
            }
            Action::SelectOverlayEntry(index) => {
                self.select_overlay_entry(index);
                self.confirm_overlay_selection()
            }
            Action::HoverOverlayEntry(Some(index)) => {
                self.overlays
                    .picker
                    .hovered_index
                    .set(&self.store, Some(index));
                self.select_overlay_entry(index);
                Vec::new()
            }
            Action::HoverOverlayEntry(None) => {
                self.overlays.picker.hovered_index.set(&self.store, None);
                Vec::new()
            }
            Action::ScrollActiveOverlayListPx(delta_px) => {
                self.scroll_active_overlay_list_px(delta_px);
                Vec::new()
            }
            Action::ShowKeyboardShortcuts => {
                if self.overlays_top() == Some(OverlaySurface::KeyboardShortcuts) {
                    self.pop_overlay();
                } else {
                    self.push_overlay(OverlaySurface::KeyboardShortcuts, None);
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn apply_compare_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::Bootstrap => Vec::new(),
            Action::OpenRepositoryDialog => vec![Effect::OpenRepositoryDialog],
            Action::OpenRepository(path) => self.open_repository(path),
            Action::SetLeftRef(value) => {
                let mut effects = self.update_compare_field(CompareField::Left, value);
                effects.extend(self.persist_settings_effect());
                effects
            }
            Action::SetRightRef(value) => {
                let mut effects = self.update_compare_field(CompareField::Right, value);
                effects.extend(self.persist_settings_effect());
                effects
            }
            Action::SetCompareMode(mode) => {
                self.compare.mode.set(&self.store, mode);
                if self.overlays_top() == Some(OverlaySurface::CompareMenu) {
                    self.pop_overlay();
                }
                self.persist_settings_effect()
            }
            Action::CycleCompareMode => {
                let next = match self.compare.mode.get(&self.store) {
                    CompareMode::SingleCommit => CompareMode::TwoDot,
                    CompareMode::TwoDot => CompareMode::ThreeDot,
                    CompareMode::ThreeDot => CompareMode::SingleCommit,
                };
                self.compare.mode.set(&self.store, next);
                self.persist_settings_effect()
            }
            Action::OpenCompareMenu => {
                self.push_overlay(OverlaySurface::CompareMenu, None);
                Vec::new()
            }
            Action::ApplyComparePreset(preset) => self.apply_compare_preset(&preset),
            Action::SetLayoutMode(layout) => {
                self.compare.layout.set(&self.store, layout);
                self.editor.layout.set(&self.store, layout);
                self.rebuild_command_palette();
                self.persist_settings_effect()
            }
            Action::SetRenderer(renderer) => {
                self.compare.renderer.set(&self.store, renderer);
                self.persist_settings_effect()
            }
            Action::StartCompare => self.kickoff_compare(),
            Action::ShowWorkingTree => self.show_working_tree(),
            Action::StageSelectedFile => {
                self.apply_selected_status_operation(StatusOperation::Stage)
            }
            Action::UnstageSelectedFile => {
                self.apply_selected_status_operation(StatusOperation::Unstage)
            }
            Action::DiscardSelectedFile => {
                self.apply_selected_status_operation(StatusOperation::Discard)
            }
            Action::StageFile(index) => {
                self.apply_file_status_operation(index, StatusOperation::Stage)
            }
            Action::UnstageFile(index) => {
                self.apply_file_status_operation(index, StatusOperation::Unstage)
            }
            Action::StageAllFiles => self.apply_batch_scope_operation(
                &[StatusScope::Unstaged, StatusScope::Untracked],
                StatusOperation::Stage,
            ),
            Action::UnstageAllFiles => {
                self.apply_batch_scope_operation(&[StatusScope::Staged], StatusOperation::Unstage)
            }
            Action::StageHunk => self.apply_hunk_operation(StatusOperation::Stage),
            Action::UnstageHunk => self.apply_hunk_operation(StatusOperation::Unstage),
            Action::DiscardHunk => self.apply_hunk_operation(StatusOperation::Discard),
            Action::ToggleLineSelection(row) => {
                self.toggle_line_selection(row, false);
                let entries_len = self
                    .editor
                    .line_selection
                    .with(&self.store, |ls| ls.entries.len());
                tracing::info!(row, entries = entries_len, "ToggleLineSelection");
                Vec::new()
            }
            Action::ToggleLineSelectionRange(row, anchor) => {
                self.toggle_line_selection_range(row, anchor);
                Vec::new()
            }
            Action::StageSelectedLines => {
                self.apply_line_selection_operation(StatusOperation::Stage)
            }
            Action::UnstageSelectedLines => {
                self.apply_line_selection_operation(StatusOperation::Unstage)
            }
            Action::DiscardSelectedLines => {
                self.apply_line_selection_operation(StatusOperation::Discard)
            }
            Action::ClearLineSelection => {
                self.editor
                    .line_selection
                    .update(&self.store, |ls| ls.clear());
                Vec::new()
            }
            Action::SubmitCommit => self.submit_commit(),
            Action::SelectSidebarCommit(oid) => self.drill_into_commit(&oid),
            Action::ClearSidebarCommit => self.restore_pre_drill_compare(),
            _ => Vec::new(),
        }
    }

    fn submit_commit(&mut self) -> Vec<Effect> {
        let message = self.commit_editor.text().trim().to_owned();
        if message.is_empty() {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let has_staged = self
            .workspace
            .status_items
            .with(&self.store, |items| items.iter().any(|item| item.scope == StatusScope::Staged));
        if !has_staged {
            return Vec::new();
        }
        vec![Effect::CreateCommit(CommitRequest { repo_path, message })]
    }

    fn drill_into_commit(&mut self, oid: &str) -> Vec<Effect> {
        if self
            .workspace
            .pre_drill_compare
            .with(&self.store, |p| p.is_none())
        {
            let left = self.compare.left_ref.get(&self.store);
            let right = self.compare.right_ref.get(&self.store);
            let mode = self.compare.mode.get(&self.store);
            self.workspace
                .pre_drill_compare
                .set(&self.store, Some((left, right, mode)));
        }
        self.compare
            .left_ref
            .set(&self.store, oid[..7.min(oid.len())].to_owned());
        self.compare.right_ref.set(&self.store, String::new());
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        self.compare.mode.set(&self.store, CompareMode::SingleCommit);
        self.kickoff_compare()
    }

    fn restore_pre_drill_compare(&mut self) -> Vec<Effect> {
        let mut taken: Option<(String, String, CompareMode)> = None;
        self.workspace
            .pre_drill_compare
            .update(&self.store, |p| taken = p.take());
        let Some((left, right, mode)) = taken else {
            return Vec::new();
        };
        self.compare.left_ref.set(&self.store, left);
        self.compare.right_ref.set(&self.store, right);
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        self.compare.mode.set(&self.store, mode);
        self.kickoff_compare()
    }

    fn apply_file_list_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::SelectFile(index) => self.select_file(index, false),
            Action::SelectFilePath(path) => {
                let idx = self
                    .workspace
                    .files
                    .with(&self.store, |files| {
                        files.iter().position(|file| file.path == path)
                    });
                if let Some(index) = idx {
                    return self.select_file(index, true);
                } else {
                    self.startup.preferred_file_path = Some(path);
                }
                Vec::new()
            }
            Action::SelectNextFile => {
                self.shift_loaded_file(1);
                Vec::new()
            }
            Action::SelectPreviousFile => {
                self.shift_loaded_file(-1);
                Vec::new()
            }
            Action::ScrollFileList(delta) => {
                self.file_list_scroll_rows(delta, self.sidebar_row_count());
                Vec::new()
            }
            Action::ScrollFileListPx(delta_px) => {
                self.file_list_scroll_px(delta_px as f32, self.sidebar_row_count());
                Vec::new()
            }
            Action::ScrollFileListToPx(px) => {
                self.file_list
                    .scroll_offset_px
                    .set(&self.store, px as f32);
                self.file_list_clamp_scroll(self.sidebar_row_count());
                Vec::new()
            }
            Action::HoverFile(index) => {
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
            Action::ToggleFolder(path) => {
                self.file_list.expanded_folders.update(&self.store, |set| {
                    if set.contains(&path) {
                        set.remove(&path);
                    } else {
                        set.insert(path);
                    }
                });
                Vec::new()
            }
            Action::ToggleFileViewed(index) => {
                self.file_list.viewed_files.update(&self.store, |set| {
                    if set.contains(&index) {
                        set.remove(&index);
                    } else {
                        set.insert(index);
                    }
                });
                Vec::new()
            }
            Action::SetSidebarFilter(query) => {
                self.file_list.filter.set(&self.store, query);
                if self.file_list.tab.get(&self.store) == SidebarTab::Commits {
                    self.file_list
                        .commits_scroll_offset_px
                        .set(&self.store, 0.0);
                } else {
                    self.file_list.scroll_offset_px.set(&self.store, 0.0);
                }
                Vec::new()
            }
            Action::ClearSidebarFilter => {
                self.file_list
                    .filter
                    .update(&self.store, |s| s.clear());
                if self.file_list.tab.get(&self.store) == SidebarTab::Commits {
                    self.file_list
                        .commits_scroll_offset_px
                        .set(&self.store, 0.0);
                } else {
                    self.file_list.scroll_offset_px.set(&self.store, 0.0);
                }
                Vec::new()
            }
            Action::ToggleSidebar => {
                self.store.update(self.sidebar_visible, |v| *v = !*v);
                Vec::new()
            }
            Action::ToggleSidebarMode => {
                let next = match self.file_list.mode.get(&self.store) {
                    SidebarMode::FlatList => SidebarMode::TreeView,
                    SidebarMode::TreeView => SidebarMode::FlatList,
                };
                self.file_list.mode.set(&self.store, next);
                self.file_list.scroll_offset_px.set(&self.store, 0.0);
                Vec::new()
            }
            Action::ExpandAllFolders => {
                let paths = self
                    .workspace
                    .files
                    .with(&self.store, |files| {
                        files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
                    });
                self.file_list.expanded_folders.update(&self.store, |set| {
                    for path in &paths {
                        let parts: Vec<&str> = path.split('/').collect();
                        for depth in 0..parts.len().saturating_sub(1) {
                            let folder_path = parts[..=depth].join("/");
                            set.insert(folder_path);
                        }
                    }
                });
                Vec::new()
            }
            Action::CollapseAllFolders => {
                self.file_list
                    .expanded_folders
                    .update(&self.store, |s| s.clear());
                Vec::new()
            }
            Action::SetSidebarTab(tab) => {
                self.file_list.tab.set(&self.store, tab);
                self.file_list
                    .filter
                    .update(&self.store, |s| s.clear());
                Vec::new()
            }
            Action::ScrollCommitListPx(delta) => {
                let stride = self.file_list_row_stride();
                let commit_count = self.workspace.range_commits.with(&self.store, |c| c.len());
                let total = self.file_list_total_content_height(commit_count);
                let max_scroll =
                    (total - self.file_list.viewport_height.get(&self.store)).max(0.0);
                let cur = self.file_list.commits_scroll_offset_px.get(&self.store);
                self.file_list.commits_scroll_offset_px.set(
                    &self.store,
                    (cur + delta as f32 * stride).clamp(0.0, max_scroll),
                );
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn apply_navigation_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::ScrollViewportLines(delta) => {
                self.scroll_viewport_lines(delta);
                Vec::new()
            }
            Action::ScrollViewportPx(delta_px) => {
                self.scroll_viewport_px(delta_px);
                Vec::new()
            }
            Action::ScrollViewportPages(delta) => {
                self.scroll_viewport_pages(delta);
                Vec::new()
            }
            Action::ScrollViewportTo(px) => {
                self.editor.scroll_top_px.set(&self.store, px);
                self.editor_clamp_scroll();
                Vec::new()
            }
            Action::ScrollViewportHalfPage(dir) => {
                self.scroll_viewport_half_page(dir);
                Vec::new()
            }
            Action::HoverViewportRow(row) => {
                self.editor.hovered_row.set(&self.store, row);
                Vec::new()
            }
            Action::FocusViewport => {
                self.set_focus(Some(FocusTarget::Editor));
                Vec::new()
            }
            Action::GoToNextHunk => {
                self.navigate_to_hunk(true);
                Vec::new()
            }
            Action::GoToPreviousHunk => {
                self.navigate_to_hunk(false);
                Vec::new()
            }
            Action::GoToNextFile => {
                self.navigate_to_file(true);
                Vec::new()
            }
            Action::GoToPreviousFile => {
                self.navigate_to_file(false);
                Vec::new()
            }
            Action::OpenSearch => {
                self.open_search();
                Vec::new()
            }
            Action::CloseSearch => {
                self.close_search();
                Vec::new()
            }
            Action::SearchNext => {
                self.search_navigate(1);
                Vec::new()
            }
            Action::SearchPrevious => {
                self.search_navigate(-1);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn apply_settings_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::ToggleWrap => {
                let current = self.editor.wrap_enabled.get(&self.store);
                self.editor.wrap_enabled.set(&self.store, !current);
                self.persist_settings_effect()
            }
            Action::SetWrapColumn(column) => {
                self.editor.wrap_column.set(&self.store, column);
                self.persist_settings_effect()
            }
            Action::SetSidebarWidthPx(width) => {
                self.settings.sidebar_width_px = Some(self.clamp_sidebar_width_px(width));
                Vec::new()
            }
            Action::IncreaseUiScale => self.adjust_ui_scale(UI_SCALE_STEP_PCT as i16),
            Action::DecreaseUiScale => self.adjust_ui_scale(-(UI_SCALE_STEP_PCT as i16)),
            Action::ToggleThemeMode => {
                self.settings.theme_mode = match self.settings.theme_mode {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
                self.persist_settings_effect()
            }
            Action::SetThemeName(name) => {
                self.settings.theme_name = name;
                self.persist_settings_effect()
            }
            _ => Vec::new(),
        }
    }

    fn apply_github_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::SubmitPullRequest => self.submit_pull_request(),
            Action::UsePullRequestCompare => self.use_pull_request_compare(),
            Action::StartGitHubDeviceFlow => {
                self.github.auth.status.set(&self.store, AsyncStatus::Loading);
                vec![Effect::StartDeviceFlow {
                    client_id: self.github.client_id.get(&self.store),
                }]
            }
            Action::OpenDeviceFlowBrowser => {
                let verification_uri = self
                    .github
                    .auth
                    .device_flow
                    .with(&self.store, |opt| {
                        opt.as_ref().map(|df| df.verification_uri.clone())
                    });
                if let Some(url) = verification_uri {
                    vec![Effect::OpenBrowser { url }]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    pub fn apply_event(&mut self, event: AppEvent) -> Vec<Effect> {
        match event {
            AppEvent::RepositoryDialogClosed { path } => {
                path.map_or_else(Vec::new, |path| self.open_repository(path))
            }
            AppEvent::RepositorySnapshotReady(payload) => self.handle_repository_snapshot(payload),
            AppEvent::RepositorySnapshotFailed {
                path,
                reason,
                message,
            } => {
                if self
                    .compare
                    .repo_path
                    .with(&self.store, |p| p.as_ref() == Some(&path))
                {
                    if reason == RepositorySyncReason::Open {
                        self.repository.status.set(&self.store, AsyncStatus::Failed);
                        self.workspace_mode.set(&self.store, WorkspaceMode::Empty);
                        self.push_error(&message);
                    } else {
                        self.last_error.set(&self.store, Some(message));
                    }
                }
                Vec::new()
            }
            AppEvent::CompareFinished(payload) => self.handle_compare_finished(payload),
            AppEvent::CompareFailed {
                generation,
                message,
            } => {
                if generation == self.workspace.compare_generation.get(&self.store) {
                    self.workspace.status.set(&self.store, AsyncStatus::Failed);
                    self.workspace_mode.set(&self.store, WorkspaceMode::Empty);
                    self.push_error(&message);
                }
                Vec::new()
            }
            AppEvent::StatusDiffFinished(payload) => self.handle_status_diff_finished(payload),
            AppEvent::StatusDiffFailed {
                generation,
                index: _,
                message,
            } => {
                if generation == self.workspace.status_generation.get(&self.store) {
                    self.workspace.status.set(&self.store, AsyncStatus::Failed);
                    self.push_error(&message);
                }
                Vec::new()
            }
            AppEvent::StatusOperationFailed { path, message } => {
                if self
                    .compare
                    .repo_path
                    .with(&self.store, |p| p.as_ref() == Some(&path))
                {
                    self.push_error(&message);
                }
                Vec::new()
            }
            AppEvent::CommitCreated { path } => {
                if self
                    .compare
                    .repo_path
                    .with(&self.store, |p| p.as_ref() == Some(&path))
                {
                    self.commit_editor.request_clear();
                    self.push_info("Commit created.");
                }
                Vec::new()
            }
            AppEvent::CommitFailed { path, message } => {
                if self
                    .compare
                    .repo_path
                    .with(&self.store, |p| p.as_ref() == Some(&path))
                {
                    self.push_error(&message);
                }
                Vec::new()
            }
            AppEvent::PullRequestLoaded {
                url,
                info,
                left_ref,
                right_ref,
            } => {
                self.github
                    .pull_request
                    .status
                    .set(&self.store, AsyncStatus::Ready);
                self.github.pull_request.url_input.set(&self.store, url);
                self.github.pull_request.info.set(&self.store, Some(info));
                self.github
                    .pull_request
                    .candidate_left_ref
                    .set(&self.store, Some(left_ref));
                self.github
                    .pull_request
                    .candidate_right_ref
                    .set(&self.store, Some(right_ref));
                Vec::new()
            }
            AppEvent::PullRequestLoadFailed { message, .. } => {
                self.github
                    .pull_request
                    .status
                    .set(&self.store, AsyncStatus::Failed);
                self.push_error(&message);
                Vec::new()
            }
            AppEvent::DeviceFlowStarted(device_flow) => {
                self.github
                    .auth
                    .status
                    .set(&self.store, AsyncStatus::Loading);
                self.github
                    .auth
                    .device_flow
                    .set(&self.store, Some(device_flow.clone()));
                vec![
                    Effect::OpenBrowser {
                        url: device_flow.verification_uri.clone(),
                    },
                    Effect::PollDeviceFlow {
                        client_id: self.github.client_id.get(&self.store),
                        device_code: device_flow.device_code,
                        interval_seconds: device_flow.interval,
                    },
                ]
            }
            AppEvent::DeviceFlowStartFailed { message } => {
                self.github
                    .auth
                    .status
                    .set(&self.store, AsyncStatus::Failed);
                self.push_error(&message);
                Vec::new()
            }
            AppEvent::DeviceFlowCompleted { token } => {
                self.github
                    .auth
                    .status
                    .set(&self.store, AsyncStatus::Ready);
                self.github.auth.device_flow.set(&self.store, None);
                self.github.auth.token_present.set(&self.store, true);
                self.settings.github_token = Some(token);
                self.push_info("GitHub authentication completed.");
                if self.overlays_top() == Some(OverlaySurface::GitHubAuthModal) {
                    self.pop_overlay();
                }
                self.persist_settings_effect()
            }
            AppEvent::DeviceFlowFailed { message } => {
                self.github
                    .auth
                    .status
                    .set(&self.store, AsyncStatus::Failed);
                self.push_error(&message);
                Vec::new()
            }
            AppEvent::RefResolved {
                query,
                generation,
                short_oid,
                summary,
            } => {
                if generation == self.overlays.picker.ref_resolve_generation.get(&self.store) {
                    self.overlays.picker.entries.update(&self.store, |entries| {
                        if let Some(entry) = entries
                            .iter_mut()
                            .find(|e| e.value == query && e.detail == "Resolving\u{2026}")
                        {
                            entry.detail = format!("{short_oid} \u{2014} {summary}");
                        }
                    });
                }
                Vec::new()
            }
            AppEvent::RefResolveFailed { generation } => {
                if generation == self.overlays.picker.ref_resolve_generation.get(&self.store) {
                    self.overlays.picker.entries.update(&self.store, |entries| {
                        if let Some(entry) =
                            entries.iter_mut().find(|e| e.detail == "Resolving\u{2026}")
                        {
                            entry.detail = "Use typed ref".to_owned();
                        }
                    });
                }
                Vec::new()
            }
            AppEvent::SettingsSaved => Vec::new(),
            AppEvent::SettingsSaveFailed { message } => {
                self.push_error(&message);
                Vec::new()
            }
            AppEvent::BrowserOpenFailed { message } => {
                self.push_error(&message);
                Vec::new()
            }
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
        let selected_path = self
            .workspace
            .selected_file_path
            .get(&self.store);
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
                toast.hovered || now_ms.saturating_sub(toast.created_at_ms) < TOAST_LIFETIME_MS
            });
        });
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
                .filter(|toast| !toast.hovered)
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
        self.repository.status.set(&self.store, AsyncStatus::Loading);
        self.workspace_clear_compare();
        self.reset_file_list();
        self.editor_clear_document();
        self.editor.focused.set(&self.store, false);
        self.last_error.set(&self.store, None);
        self.github.pull_request.info.set(&self.store, None);
        self.github
            .pull_request
            .candidate_left_ref
            .set(&self.store, None);
        self.github
            .pull_request
            .candidate_right_ref
            .set(&self.store, None);
        self.clear_overlays();
        self.focus.set(&self.store, Some(FocusTarget::TitleBar));
        self.sync_settings_snapshot();
        vec![
            Effect::SaveSettings(self.settings.clone()),
            Effect::SyncRepository {
                path: path.clone(),
                reason: RepositorySyncReason::Open,
            },
            Effect::WatchRepository { path: Some(path) },
        ]
    }

    /// Clear the workspace back to a blank "no compare loaded" state. Replaces
    /// the former `WorkspaceState::clear_compare(&mut self)` method.
    fn workspace_clear_compare(&mut self) {
        self.workspace.source.set(&self.store, WorkspaceSource::None);
        self.workspace.status.set(&self.store, AsyncStatus::Idle);
        self.workspace.status_generation.set(&self.store, 0);
        self.workspace.files.set(&self.store, Vec::new());
        self.workspace.status_items.set(&self.store, Vec::new());
        self.workspace.selected_file_index.set(&self.store, None);
        self.workspace.selected_file_path.set(&self.store, None);
        self.workspace.selected_status_scope.set(&self.store, None);
        self.workspace.compare_output.set(&self.store, None);
        self.workspace.active_file.set(&self.store, None);
        self.workspace.raw_diff_len.set(&self.store, 0);
        self.workspace.used_fallback.set(&self.store, false);
        self.workspace
            .fallback_message
            .set(&self.store, String::new());
        self.workspace.sidebar_auto_width.set(&self.store, None);
        self.workspace.range_commits.set(&self.store, Vec::new());
        self.workspace.pre_drill_compare.set(&self.store, None);
    }

    fn handle_repository_snapshot(&mut self, payload: RepositorySnapshot) -> Vec<Effect> {
        if self
            .compare
            .repo_path
            .with(&self.store, |p| p.as_ref() != Some(&payload.path))
        {
            return Vec::new();
        }

        self.repository.status.set(&self.store, AsyncStatus::Ready);
        self.repository.branches.set(&self.store, payload.branches);
        self.repository.tags.set(&self.store, payload.tags);
        self.repository.commits.set(&self.store, payload.commits);
        self.workspace
            .status_items
            .set(&self.store, payload.status_items);

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
                    effects.push(Effect::LoadPullRequest {
                        url,
                        repo_path: payload.path,
                        github_token: self.settings.github_token.clone(),
                    });
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

    fn handle_compare_finished(&mut self, payload: CompareFinished) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

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
        self.workspace
            .files
            .set(&self.store, build_file_entries(&payload.output.files));
        self.workspace
            .compare_output
            .set(&self.store, Some(payload.output));
        self.workspace.sidebar_auto_width.set(&self.store, None);
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

        let (file_count, index_for_path) =
            self.workspace.files.with(&self.store, |files| {
                let idx = preferred_path
                    .as_deref()
                    .and_then(|path| files.iter().position(|file| file.path == path));
                (files.len(), idx)
            });

        if let Some(index) = index_for_path
            .or(preferred_index.filter(|index| *index < file_count))
            .or_else(|| (file_count > 0).then_some(0))
        {
            let _ = self.select_file(index, true);
        } else {
            self.workspace.selected_file_index.set(&self.store, None);
            self.workspace.selected_file_path.set(&self.store, None);
            self.workspace.selected_status_scope.set(&self.store, None);
            self.workspace.active_file.set(&self.store, None);
            self.editor_clear_document();
        }

        let (used_fallback, fallback_message) = (
            self.workspace.used_fallback.get(&self.store),
            self.workspace.fallback_message.get(&self.store),
        );
        if used_fallback && !fallback_message.is_empty() {
            self.push_info(&fallback_message);
        }
        Vec::new()
    }

    fn handle_status_diff_finished(&mut self, payload: StatusDiffFinished) -> Vec<Effect> {
        if payload.generation != self.workspace.status_generation.get(&self.store) {
            return Vec::new();
        }
        let matches = self.workspace.status_items.with(&self.store, |items| {
            match items.get(payload.index) {
                Some(current) => {
                    current.path == payload.item.path && current.scope == payload.item.scope
                }
                None => false,
            }
        });
        if !matches {
            return Vec::new();
        }

        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Status);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace_mode.set(&self.store, WorkspaceMode::Ready);
        let mut output = payload.output;
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

        let Some(mut file) = output.files.into_iter().next() else {
            self.workspace.active_file.set(&self.store, None);
            self.editor_clear_document();
            return Vec::new();
        };

        DiffSyntaxAnnotator::new().annotate(
            &mut file,
            &mut output.text_buffer,
            &mut output.token_buffer,
        );
        file.syntax_annotated = true;
        let render_doc = build_render_doc(
            &file,
            payload.index,
            &output.text_buffer,
            &output.token_buffer,
        );

        self.workspace
            .selected_file_index
            .set(&self.store, Some(payload.index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(payload.item.path.clone()));
        self.workspace
            .selected_status_scope
            .set(&self.store, Some(payload.item.scope));
        self.workspace.active_file.set(
            &self.store,
            Some(ActiveFile {
                index: payload.index,
                path: payload.item.path.clone(),
                file,
                render_doc,
                text_buffer: output.text_buffer,
            }),
        );
        self.editor_clear_document();
        self.editor
            .line_selection
            .update(&self.store, |ls| ls.clear());
        if self.editor.search.open.get(&self.store) {
            self.recompute_search_matches();
        }
        Vec::new()
    }

    fn activate_status_view(&mut self, reset_scroll: bool) -> Vec<Effect> {
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Status);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace_mode.set(&self.store, WorkspaceMode::Ready);
        self.workspace.compare_output.set(&self.store, None);
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
        let selected_index = self
            .workspace
            .status_items
            .with(&self.store, |items| {
                if let Some((path, scope)) = current_path.clone().zip(current_scope) {
                    if let Some(idx) = items
                        .iter()
                        .position(|item| item.path == path && item.scope == scope)
                    {
                        return Some(idx);
                    }
                }
                if let Some(path) = current_path.as_deref() {
                    if let Some(idx) = items.iter().position(|item| item.path == path) {
                        return Some(idx);
                    }
                }
                (!items.is_empty()).then_some(0)
            });

        match selected_index {
            Some(index) => self.select_status_item(index, false),
            None => {
                self.workspace.selected_file_index.set(&self.store, None);
                self.workspace.selected_file_path.set(&self.store, None);
                self.workspace.selected_status_scope.set(&self.store, None);
                self.workspace.active_file.set(&self.store, None);
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

        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Compare);
        let next_gen = self
            .workspace
            .compare_generation
            .get(&self.store)
            .saturating_add(1);
        self.workspace
            .compare_generation
            .set(&self.store, next_gen);
        self.clear_overlays();
        self.sync_settings_snapshot();

        let renderer = self.compare.renderer.get(&self.store);
        let layout = self.compare.layout.get(&self.store);
        vec![
            Effect::SaveSettings(self.settings.clone()),
            Effect::RunCompare {
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
                    github_token: self.settings.github_token.clone(),
                },
            },
        ]
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

    fn persist_settings_effect(&mut self) -> Vec<Effect> {
        self.sync_settings_snapshot();
        vec![Effect::SaveSettings(self.settings.clone())]
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
            FocusTarget::PullRequestInput => Some(
                self.github
                    .pull_request
                    .url_input
                    .with(&self.store, |s| f(s)),
            ),
            FocusTarget::SidebarSearch => Some(self.file_list.filter.with(&self.store, |s| f(s))),
            FocusTarget::SearchInput => {
                Some(self.editor.search.query.with(&self.store, |s| f(s)))
            }
            FocusTarget::CommitEditor => None,
            _ => None,
        }
    }

    pub(super) fn with_focused_text<R>(&self, f: impl FnOnce(&str) -> R) -> Option<R> {
        let target = self.focus.get(&self.store)?;
        self.with_text_for_focus(target, f)
    }

    pub(super) fn update_focused_text<R>(
        &mut self,
        f: impl FnOnce(&mut String) -> R,
    ) -> Option<R> {
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
            Some(FocusTarget::PullRequestInput) => {
                let mut out = None;
                self.github
                    .pull_request
                    .url_input
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
            _ => None,
        }
    }

    /// Returns true if the current focus target is a text editing field.
    pub fn is_text_focused(&self) -> bool {
        self.focus
            .get(&self.store)
            .is_some_and(|target| target.is_text_field())
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
        let effects = if matches!(self.overlays_top(), Some(OverlaySurface::RefPicker(active)) if active == field)
        {
            self.rebuild_ref_picker(field)
        } else {
            Vec::new()
        };
        self.rebuild_command_palette();
        effects
    }

    fn auto_select_compare_mode(&mut self) {
        let left = self.compare.left_ref.get(&self.store);
        let right = self.compare.right_ref.get(&self.store);
        if left.is_empty() || right.is_empty() {
            return;
        }
        if left == right && right != crate::core::vcs::git::service::WORKDIR_REF {
            self.compare.mode.set(&self.store, CompareMode::SingleCommit);
            return;
        }
        let is_trunk = |r: &str| matches!(r, "main" | "master" | "develop" | "development");
        if is_trunk(&left) != is_trunk(&right) {
            self.compare.mode.set(&self.store, CompareMode::ThreeDot);
        }
    }

    fn submit_pull_request(&mut self) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            self.push_error("Open a repository before loading a pull request.");
            return Vec::new();
        };
        let url = self
            .github
            .pull_request
            .url_input
            .with(&self.store, |s| s.trim().to_owned());
        if url.is_empty() {
            self.push_error("Paste a GitHub pull request URL first.");
            return Vec::new();
        }
        self.github
            .pull_request
            .status
            .set(&self.store, AsyncStatus::Loading);
        vec![Effect::LoadPullRequest {
            url,
            repo_path,
            github_token: self.settings.github_token.clone(),
        }]
    }

    fn use_pull_request_compare(&mut self) -> Vec<Effect> {
        let Some(left) = self.github.pull_request.candidate_left_ref.get(&self.store) else {
            self.push_error("Load a pull request before using its compare.");
            return Vec::new();
        };
        let Some(right) = self.github.pull_request.candidate_right_ref.get(&self.store) else {
            self.push_error("Load a pull request before using its compare.");
            return Vec::new();
        };
        // Picker won't be open here so resolve effects are empty, but drain them.
        let _ = self.update_compare_field(CompareField::Left, left);
        let _ = self.update_compare_field(CompareField::Right, right);
        self.compare.mode.set(&self.store, CompareMode::ThreeDot);
        self.clear_overlays();
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
        let selected = entries
            .iter()
            .position(|e| !e.section_header)
            .unwrap_or(0);
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
            let selected = entries
                .iter()
                .position(|e| !e.section_header)
                .unwrap_or(0);
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
        self.push_overlay(
            OverlaySurface::RefPicker(field),
            Some(FocusTarget::PickerInput),
        );
        effects
    }

    fn open_command_palette(&mut self) {
        let scale = self.ui_scale_factor();
        self.overlays.command_palette.list.update(&self.store, |l| {
            l.row_height_px = (Sz::ROW * scale).round() as u32;
            l.gap_px = (Sp::XS * scale).round() as u32;
            l.scroll_top_px = 0;
        });
        self.rebuild_command_palette();
        self.push_overlay(
            OverlaySurface::CommandPalette,
            Some(FocusTarget::CommandPaletteInput),
        );
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
            OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_) => {
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
                let (idx, len, value) =
                    self.overlays.picker.entries.with(&self.store, |entries| {
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
            Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_)) => {
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
            Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_)) => {
                let (clamped, len, is_header) =
                    self.overlays.picker.entries.with(&self.store, |entries| {
                        let len = entries.len();
                        let clamped = index.min(len.saturating_sub(1));
                        let is_header = entries
                            .get(clamped)
                            .map_or(false, |e| e.section_header);
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
                let value = self
                    .overlays
                    .picker
                    .entries
                    .with(&self.store, |entries| {
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
            Some(OverlaySurface::RefPicker(field)) => self.confirm_ref_picker(field),
            Some(OverlaySurface::CommandPalette) => self.confirm_command_palette(),
            Some(OverlaySurface::PullRequestModal) => self.submit_pull_request(),
            Some(OverlaySurface::GitHubAuthModal) => {
                if self
                    .github
                    .auth
                    .device_flow
                    .with(&self.store, |opt| opt.is_some())
                {
                    self.apply_action(Action::OpenDeviceFlowBrowser)
                } else {
                    self.apply_action(Action::StartGitHubDeviceFlow)
                }
            }
            Some(OverlaySurface::KeyboardShortcuts | OverlaySurface::CompareMenu) => Vec::new(),
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
        if let Some(rest) = entry.value.strip_prefix("@preset:") {
            return self.apply_compare_preset(rest);
        }
        if let Some(ref store) = self.frecency {
            store.record_access(&format!("ref:{}", entry.value));
        }
        let _ = self.update_compare_field(field, entry.value);
        self.pop_overlay();
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
        self.clear_overlays();
        match entry.kind {
            PaletteEntryKind::Command(command) => match command {
                PaletteCommand::OpenRepoPicker => {
                    self.open_repo_picker();
                    Vec::new()
                }
                PaletteCommand::OpenPullRequestModal => {
                    self.push_overlay(
                        OverlaySurface::PullRequestModal,
                        Some(FocusTarget::PullRequestInput),
                    );
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
                PaletteCommand::ToggleWrap => self.apply_action(Action::ToggleWrap),
                PaletteCommand::ToggleThemeMode => self.apply_action(Action::ToggleThemeMode),
                PaletteCommand::SetLayout(layout) => {
                    self.apply_action(Action::SetLayoutMode(layout))
                }
                PaletteCommand::ChangeTheme => self.apply_action(Action::OpenThemePicker),
                PaletteCommand::SetTheme(name) => self.apply_action(Action::SetThemeName(name)),
            },
            PaletteEntryKind::File(index) => self.select_file(index, true),
            PaletteEntryKind::Repo(path) => self.open_repository(path),
            PaletteEntryKind::Ref(field, value) => {
                let _ = self.update_compare_field(field, value);
                self.persist_settings_effect()
            }
        }
    }

    fn rebuild_repo_picker(&mut self) {
        let query = self
            .overlays
            .picker
            .query
            .with(&self.store, |q| q.clone());
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
                    let first_selectable = entries
                        .iter()
                        .position(|e| !e.section_header)
                        .unwrap_or(0);
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
        self.overlays
            .picker
            .selected_index
            .set(&self.store, current_selected.min(entry_count.saturating_sub(1)));
        self.overlays.picker.list.update(&self.store, |l| {
            l.viewport_height_px = l.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, entry_count);
            l.clamp_scroll(entry_count);
        });

        if needs_resolve {
            if let Some(repo_path) = self.compare.repo_path.get(&self.store) {
                let new_gen = self
                    .overlays
                    .picker
                    .ref_resolve_generation
                    .get(&self.store)
                    + 1;
                self.overlays
                    .picker
                    .ref_resolve_generation
                    .set(&self.store, new_gen);
                return vec![Effect::ResolveRef {
                    repo_path,
                    query: query.to_owned(),
                    generation: new_gen,
                }];
            }
        }
        Vec::new()
    }

    fn rebuild_command_palette(&mut self) {
        let query_owned = self
            .overlays
            .command_palette
            .query
            .with(&self.store, |q| q.trim().to_owned());
        let query = query_owned.as_str();

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
                "Open Pull Request".to_owned(),
                "Load PR metadata".to_owned(),
                PaletteCommand::OpenPullRequestModal,
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
                        },
                    )
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.label.cmp(&b.1.label)));
            entries = scored.into_iter().map(|(_, e)| e).collect();
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
        self.overlays
            .command_palette
            .selected_index
            .set(&self.store, current_selected.min(entry_count.saturating_sub(1)));
        self.overlays.command_palette.list.update(&self.store, |l| {
            l.viewport_height_px = l.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, entry_count);
            l.clamp_scroll(entry_count);
        });
    }

    fn shift_loaded_file(&mut self, delta: isize) {
        let file_count = self.workspace.files.with(&self.store, |f| f.len());
        if file_count == 0 {
            return;
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
            WorkspaceSource::Compare => {
                self.select_loaded_compare_file(next, true);
            }
            WorkspaceSource::Status => {
                let _ = self.select_status_item(next, true);
            }
            WorkspaceSource::None => {}
        }
    }

    fn select_file(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare => {
                self.select_loaded_compare_file(index, reveal);
                Vec::new()
            }
            WorkspaceSource::Status => self.select_status_item(index, reveal),
            WorkspaceSource::None => {
                self.startup.preferred_file_index = Some(index);
                Vec::new()
            }
        }
    }

    fn select_loaded_compare_file(&mut self, index: usize, reveal: bool) {
        let mut out: Option<(FileDiff, CompareOutput)> = None;
        let mut oob = false;
        self.workspace
            .compare_output
            .update(&self.store, |maybe_output| {
                let Some(output) = maybe_output.as_mut() else {
                    return;
                };
                let Some(file) = output.files.get_mut(index) else {
                    oob = true;
                    return;
                };
                if !file.syntax_annotated {
                    DiffSyntaxAnnotator::new().annotate(
                        file,
                        &mut output.text_buffer,
                        &mut output.token_buffer,
                    );
                    file.syntax_annotated = true;
                }
                out = Some((file.clone(), output.clone()));
            });

        if out.is_none() {
            if oob {
                self.push_error("Selected file index is out of range.");
                return;
            }
            self.startup.preferred_file_index = Some(index);
            return;
        }
        let (file, output) = out.unwrap();

        self.workspace
            .selected_file_index
            .set(&self.store, Some(index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(file.path.clone()));
        self.workspace.active_file.set(
            &self.store,
            Some(ActiveFile {
                index,
                path: file.path.clone(),
                file: file.clone(),
                render_doc: build_render_doc(
                    &file,
                    index,
                    &output.text_buffer,
                    &output.token_buffer,
                ),
                text_buffer: output.text_buffer.clone(),
            }),
        );
        self.editor_clear_document();
        self.editor
            .line_selection
            .update(&self.store, |ls| ls.clear());
        if self.editor.search.open.get(&self.store) {
            self.recompute_search_matches();
        }
        self.file_list.hovered_index.set(&self.store, Some(index));
        if reveal {
            self.reveal_file_list_row(index);
        }
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
            return Vec::new();
        };
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };

        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Status);
        self.workspace.status.set(&self.store, AsyncStatus::Loading);
        self.workspace
            .selected_file_index
            .set(&self.store, Some(index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(item.path.clone()));
        self.workspace
            .selected_status_scope
            .set(&self.store, Some(item.scope));
        self.workspace.active_file.set(&self.store, None);
        self.editor_clear_document();
        self.file_list.hovered_index.set(&self.store, Some(index));
        if reveal {
            self.reveal_file_list_row(index);
        }

        let generation = self.workspace.status_generation.get(&self.store);
        let renderer = self.compare.renderer.get(&self.store);
        vec![Effect::LoadStatusDiff {
            generation,
            index,
            request: StatusDiffRequest {
                repo_path,
                item,
                renderer,
            },
        }]
    }

    fn apply_selected_status_operation(&mut self, operation: StatusOperation) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
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

        vec![Effect::ApplyStatusOperation(StatusOperationRequest {
            repo_path,
            item,
            operation,
        })]
    }

    fn apply_file_status_operation(
        &mut self,
        index: usize,
        operation: StatusOperation,
    ) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
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

        vec![Effect::ApplyStatusOperation(StatusOperationRequest {
            repo_path,
            item,
            operation,
        })]
    }

    fn apply_batch_scope_operation(
        &mut self,
        scopes: &[StatusScope],
        operation: StatusOperation,
    ) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
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

        vec![Effect::ApplyBatchStatusOperation(
            BatchStatusOperationRequest {
                repo_path,
                items,
                operation,
            },
        )]
    }

    fn current_hunk_index_from_hover(&self) -> Option<i16> {
        let hovered = self.editor.hovered_row.get(&self.store)?;
        self.workspace.active_file.with(&self.store, |af| {
            let active = af.as_ref()?;
            let line = active.render_doc.lines.get(hovered)?;
            if line.hunk_index < 0 {
                return None;
            }
            Some(line.hunk_index)
        })
    }

    fn apply_hunk_operation(&mut self, operation: StatusOperation) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let Some(scope) = self.workspace.selected_status_scope.get(&self.store) else {
            return Vec::new();
        };
        let hunk_index = match self.current_hunk_index_from_hover() {
            Some(idx) => idx as usize,
            None => return Vec::new(),
        };

        let patch_text = self.workspace.active_file.with(&self.store, |af| {
            let active = af.as_ref()?;
            if operation == StatusOperation::Stage {
                patch::format_hunk_patch(&active.file, hunk_index, &active.text_buffer)
            } else {
                patch::format_reverse_hunk_patch(&active.file, hunk_index, &active.text_buffer)
            }
        });
        let Some(patch) = patch_text else {
            return Vec::new();
        };

        vec![Effect::ApplyPatchOperation(PatchOperationRequest {
            repo_path,
            patch,
            scope,
            operation,
        })]
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
        self.editor.line_selection.update(&self.store, |ls| {
            if line.old_line_index >= 0 {
                ls.toggle(line.hunk_index, line.old_line_index);
            }
            if line.new_line_index >= 0 {
                ls.toggle(line.hunk_index, line.new_line_index);
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
                if line.old_line_index >= 0 {
                    ls.entries.insert((line.hunk_index, line.old_line_index));
                }
                if line.new_line_index >= 0 {
                    ls.entries.insert((line.hunk_index, line.new_line_index));
                }
            }
            ls.last_toggled_row = Some(row);
        });
    }

    fn apply_line_selection_operation(&mut self, operation: StatusOperation) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
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
                let indices: Vec<i16> = ls
                    .entries
                    .iter()
                    .map(|(h, _)| *h)
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
                let selected = selection_snapshot.selected_lines_for_hunk(hunk_idx);
                if let Some(p) = patch::format_lines_patch(
                    &active.file,
                    hunk_idx as usize,
                    &selected,
                    &active.text_buffer,
                    reverse,
                ) {
                    patches.push(p);
                }
            }
            patches
        });

        self.editor
            .line_selection
            .update(&self.store, |ls| ls.clear());

        patches
            .into_iter()
            .map(|p| {
                Effect::ApplyPatchOperation(PatchOperationRequest {
                    repo_path: repo_path.clone(),
                    patch: p,
                    scope,
                    operation,
                })
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
                | OverlaySurface::RefPicker(_)
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

    fn push_error(&mut self, message: &str) {
        self.last_error.set(&self.store, Some(message.to_owned()));
        self.push_toast(ToastKind::Error, message);
    }

    fn push_info(&mut self, message: &str) {
        self.push_toast(ToastKind::Info, message);
    }

    fn push_toast(&mut self, kind: ToastKind, message: &str) {
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
                created_at_ms: now,
                hovered: false,
            });
            if toasts.len() > MAX_VISIBLE_TOASTS {
                toasts.remove(0);
            }
        });
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

        let new_matches: Vec<SearchMatch> =
            self.workspace.active_file.with(&self.store, |af| {
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
        self.editor.scroll_top_px.set(&self.store, centered.min(max));
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

fn apply_scroll_delta_px(current: u32, delta: i32, max: u32) -> u32 {
    let next = if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as u32)
    };
    next.min(max)
}

fn build_file_entries(files: &[FileDiff]) -> Vec<FileListEntry> {
    files.iter().map(FileListEntry::from).collect()
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
        OverlaySurface::RefPicker(CompareField::Left) => "left-ref-picker",
        OverlaySurface::RefPicker(CompareField::Right) => "right-ref-picker",
        OverlaySurface::CommandPalette => "command-palette",
        OverlaySurface::PullRequestModal => "pull-request-modal",
        OverlaySurface::GitHubAuthModal => "github-auth-modal",
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

impl From<&FileDiff> for FileListEntry {
    fn from(value: &FileDiff) -> Self {
        Self {
            path: value.path.clone(),
            status: value.status.clone(),
            additions: value.additions,
            deletions: value.deletions,
            is_binary: value.is_binary,
        }
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
    use clap::Parser;

    use super::{
        AppState, CompareField, FileListEntry, FocusTarget, OverlaySurface, WorkspaceMode,
        WorkspaceSource,
    };
    use crate::actions::Action;
    use crate::core::compare::{CompareMode, CompareOutput, LayoutMode, RendererKind};
    use crate::core::diff::{DiffLine, FileDiff, Hunk, LineKind};
    use crate::platform::persistence::Settings;
    use crate::platform::startup::{Args, StartupOptions};

    fn loaded_state_with_files(paths: &[&str]) -> AppState {
        let mut state = AppState::default();
        let files: Vec<FileDiff> = paths
            .iter()
            .map(|path| FileDiff {
                path: (*path).to_owned(),
                status: "M".to_owned(),
                ..FileDiff::default()
            })
            .collect();

        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                files: files.clone(),
                ..CompareOutput::default()
            }),
        );
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.workspace.files.set(
            &state.store,
            files.iter().map(FileListEntry::from).collect(),
        );
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
        assert!(effects.is_empty());
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
                exit_after_ms: None,
                hidden_window: false,
                dump_state_json: None,
                dump_files_json: None,
                dump_errors_json: None,
            },
            None,
            "client".to_owned(),
            false,
        );

        let (state, effects) = AppState::bootstrap(startup, Settings::default());
        assert_eq!(state.workspace_mode.get(&state.store), WorkspaceMode::Empty);
        assert_eq!(state.active_overlay_name(), None);
        assert_eq!(effects.len(), 2);
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
        state.apply_action(Action::SetFocus(Some(FocusTarget::TitleBar)));
        state.apply_action(Action::OpenCommandPalette);
        assert_eq!(state.overlays_top(), Some(OverlaySurface::CommandPalette));
        state.apply_action(Action::CloseOverlay);
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

        state.apply_action(Action::ScrollFileListPx(50));
        assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 50.0);

        state.apply_action(Action::ScrollFileListPx(500));
        assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 116.0);

        state.apply_action(Action::ScrollFileListPx(-500));
        assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 0.0);

        state.editor.content_height_px.set(&state.store, 600);
        state.editor.viewport_height_px.set(&state.store, 200);

        state.apply_action(Action::ScrollViewportPx(75));
        assert_eq!(state.editor.scroll_top_px.get(&state.store), 75);

        state.apply_action(Action::ScrollViewportPx(500));
        assert_eq!(state.editor.scroll_top_px.get(&state.store), 400);

        state.apply_action(Action::ScrollViewportPx(-500));
        assert_eq!(state.editor.scroll_top_px.get(&state.store), 0);
    }

    #[test]
    fn clicking_a_visible_file_does_not_force_sidebar_reveal() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.file_list.scroll_offset_px.set(&state.store, 10.0);

        state.apply_action(Action::SelectFile(0));

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

        state.apply_action(Action::SelectNextFile);

        assert_eq!(
            state.workspace.selected_file_index.get(&state.store),
            Some(1)
        );
        assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 40.0);
    }

    #[test]
    fn selecting_a_file_lazily_annotates_syntax_once() {
        let mut state = AppState::default();
        let mut output = CompareOutput::default();
        let text_range = output.text_buffer.append("fn answer() -> i32 { 42 }");
        output.files = vec![FileDiff {
            path: "src/lib.rs".to_owned(),
            status: "M".to_owned(),
            hunks: vec![Hunk {
                header: "@@ -1 +1 @@".to_owned(),
                lines: vec![DiffLine {
                    kind: LineKind::Context,
                    old_line_number: Some(1),
                    new_line_number: Some(1),
                    text_range,
                    ..DiffLine::default()
                }],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        }];
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
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);

        state.apply_action(Action::SelectFile(0));

        let previous_tokens = state.workspace.compare_output.with(&state.store, |co| {
            let output = co.as_ref().expect("compare output");
            assert!(output.files[0].syntax_annotated);
            assert!(
                !output
                    .token_buffer
                    .view(output.files[0].hunks[0].lines[0].syntax_tokens)
                    .is_empty()
            );
            output.files[0].hunks[0].lines[0].syntax_tokens
        });
        state.apply_action(Action::SelectFile(0));
        state.workspace.compare_output.with(&state.store, |co| {
            let output = co.as_ref().expect("compare output");
            assert_eq!(
                output.files[0].hunks[0].lines[0].syntax_tokens,
                previous_tokens
            );
        });
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

        state.apply_action(Action::ScrollActiveOverlayListPx(50));
        assert_eq!(
            state.overlays.picker.list.with(&state.store, |l| l.scroll_top_px),
            50
        );

        state.apply_action(Action::ScrollActiveOverlayListPx(1_000));
        assert_eq!(
            state.overlays.picker.list.with(&state.store, |l| l.scroll_top_px),
            312
        );

        state.apply_action(Action::ScrollActiveOverlayListPx(-1_000));
        assert_eq!(
            state.overlays.picker.list.with(&state.store, |l| l.scroll_top_px),
            0
        );
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
            }],
        );

        state.open_ref_picker(CompareField::Left);
        state.apply_action(Action::InsertText("mai".to_owned()));

        let branch_highlights =
            state.overlays.picker.entries.with(&state.store, |entries| {
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
        state.apply_action(Action::InsertText("HEAD~2".to_owned()));

        let (typed_value, typed_highlights) =
            state.overlays.picker.entries.with(&state.store, |entries| {
                let typed_entry = entries.first().expect("typed ref entry");
                (typed_entry.value.clone(), typed_entry.highlights.clone())
            });
        assert_eq!(typed_value, "HEAD~2");
        assert_eq!(typed_highlights, vec![(0, "HEAD~2".len())]);

        state.apply_action(Action::ConfirmOverlaySelection);
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

        state.apply_action(Action::SetSidebarWidthPx(40));
        assert_eq!(state.settings.sidebar_width_px, Some(179));

        state.apply_action(Action::SetSidebarWidthPx(420));
        assert_eq!(state.settings.sidebar_width_px, Some(420));
    }

    #[test]
    fn ui_scale_actions_step_and_persist_within_bounds() {
        let mut state = AppState::default();

        let effects = state.apply_action(Action::IncreaseUiScale);
        assert_eq!(state.settings.ui_scale_pct, 110);
        assert_eq!(effects.len(), 1);

        for _ in 0..20 {
            state.apply_action(Action::IncreaseUiScale);
        }
        assert_eq!(state.settings.ui_scale_pct, 180);

        for _ in 0..20 {
            state.apply_action(Action::DecreaseUiScale);
        }
        assert_eq!(state.settings.ui_scale_pct, 70);
    }
}
