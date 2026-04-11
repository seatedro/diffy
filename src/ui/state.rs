use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use unicode_segmentation::UnicodeSegmentation;

use crate::core::compare::{CompareMode, CompareOutput, CompareSpec, LayoutMode, RendererKind};
use crate::core::diff::FileDiff;
use crate::core::frecency::FrecencyStore;
use crate::core::syntax::DiffSyntaxAnnotator;
use crate::core::vcs::git::{BranchInfo, CommitInfo, TagInfo};
use crate::core::vcs::github::{DeviceFlowState, PullRequestInfo};
use crate::platform::persistence::{PersistedCompare, Settings};
use crate::platform::startup::StartupOptions;
use crate::actions::Action;
use crate::ui::design::{Sp, Sz};
use crate::ui::editor::render_doc::{RenderDoc, build_render_doc};
use crate::ui::editor::state::EditorState;
use crate::effects::{CompareRequest, Effect};
use crate::events::{AppEvent, CompareFinished, RepositoryLoaded};
use crate::ui::icons::lucide;
use crate::ui::theme::ThemeMode;

const MAX_VISIBLE_TOASTS: usize = 8;
const TOAST_LIFETIME_MS: u64 = 10_000;
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
    CompareRepoButton,
    CompareLeftRef,
    CompareRightRef,
    CompareStartButton,
    PickerInput,
    PickerList,
    CommandPaletteInput,
    CommandPaletteList,
    PullRequestInput,
    PullRequestConfirm,
    AuthPrimaryAction,
    SidebarSearch,
    SearchInput,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusState {
    pub current: Option<FocusTarget>,
}

/// Cursor/selection state for the currently focused text field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextEditState {
    /// Byte offset of the caret.
    pub cursor: usize,
    /// Byte offset of the selection anchor.  Equal to `cursor` when nothing is selected.
    pub anchor: usize,
    /// Timestamp (clock_ms) when the cursor last moved — used to reset blink phase.
    pub cursor_moved_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompareSheetState {
    pub validation_message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidebarWidthCache {
    pub compare_generation: u64,
    pub ui_scale_pct: u16,
    pub intrinsic_width_px: f32,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceState {
    pub status: AsyncStatus,
    pub compare_generation: u64,
    pub files: Vec<FileListEntry>,
    pub selected_file_index: Option<usize>,
    pub selected_file_path: Option<String>,
    pub compare_output: Option<CompareOutput>,
    pub active_file: Option<ActiveFile>,
    pub raw_diff_len: usize,
    pub used_fallback: bool,
    pub fallback_message: String,
    pub sidebar_auto_width: Option<SidebarWidthCache>,
}

impl WorkspaceState {
    fn clear_compare(&mut self) {
        self.status = AsyncStatus::Idle;
        self.files.clear();
        self.selected_file_index = None;
        self.selected_file_path = None;
        self.compare_output = None;
        self.active_file = None;
        self.raw_diff_len = 0;
        self.used_fallback = false;
        self.fallback_message.clear();
        self.sidebar_auto_width = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarMode {
    #[default]
    FlatList,
    TreeView,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileListState {
    pub scroll_offset_px: f32,
    pub hovered_index: Option<usize>,
    pub row_height: f32,
    pub gap: f32,
    pub viewport_height: f32,
    pub filter: String,
    pub mode: SidebarMode,
    pub expanded_folders: HashSet<String>,
    pub viewed_files: HashSet<usize>,
}

impl Default for FileListState {
    fn default() -> Self {
        Self {
            scroll_offset_px: 0.0,
            hovered_index: None,
            row_height: 36.0,
            gap: 4.0,
            viewport_height: 0.0,
            filter: String::new(),
            mode: SidebarMode::FlatList,
            expanded_folders: HashSet::new(),
            viewed_files: HashSet::new(),
        }
    }
}

impl FileListState {
    pub fn row_stride(&self) -> f32 {
        self.row_height + self.gap
    }

    pub fn total_content_height(&self, file_count: usize) -> f32 {
        if file_count == 0 {
            return 0.0;
        }
        file_count as f32 * self.row_stride() - self.gap
    }

    pub fn max_scroll_px(&self, file_count: usize) -> f32 {
        (self.total_content_height(file_count) - self.viewport_height).max(0.0)
    }

    pub fn clamp_scroll(&mut self, file_count: usize) {
        let max = self.max_scroll_px(file_count);
        self.scroll_offset_px = self.scroll_offset_px.clamp(0.0, max);
    }

    /// Scroll by a number of rows (positive = down).
    pub fn scroll_rows(&mut self, delta: i32, file_count: usize) {
        let px_delta = delta as f32 * self.row_stride();
        self.scroll_offset_px += px_delta;
        self.clamp_scroll(file_count);
    }

    /// Scroll by a raw pixel delta (positive = down).
    pub fn scroll_px(&mut self, delta_px: f32, file_count: usize) {
        self.scroll_offset_px += delta_px;
        self.clamp_scroll(file_count);
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerState {
    pub kind: PickerKind,
    pub query: String,
    pub entries: Vec<PickerEntry>,
    pub selected_index: usize,
    pub list: OverlayListState,
    pub browse_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteCommand {
    OpenCompareSheet,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandPaletteState {
    pub query: String,
    pub entries: Vec<PaletteEntry>,
    pub selected_index: usize,
    pub list: OverlayListState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestState {
    pub status: AsyncStatus,
    pub url_input: String,
    pub info: Option<PullRequestInfo>,
    pub candidate_left_ref: Option<String>,
    pub candidate_right_ref: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitHubAuthState {
    pub status: AsyncStatus,
    pub device_flow: Option<DeviceFlowState>,
    pub token_present: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitHubState {
    pub client_id: String,
    pub auth: GitHubAuthState,
    pub pull_request: PullRequestState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlaySurface {
    CompareSheet,
    RepoPicker,
    RefPicker(CompareField),
    CommandPalette,
    PullRequestModal,
    GitHubAuthModal,
    KeyboardShortcuts,
    ThemePicker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayEntry {
    pub surface: OverlaySurface,
    pub focus_return: Option<FocusTarget>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayStackState {
    pub stack: Vec<OverlayEntry>,
    pub compare_sheet: CompareSheetState,
    pub picker: PickerState,
    pub command_palette: CommandPaletteState,
}

impl OverlayStackState {
    pub fn top(&self) -> Option<OverlaySurface> {
        self.stack.last().map(|entry| entry.surface)
    }

    pub fn active_name(&self) -> Option<&'static str> {
        self.top().map(overlay_name)
    }

    pub fn clear(&mut self) {
        self.stack.clear();
        self.picker = PickerState::default();
        self.command_palette = CommandPaletteState::default();
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DebugState {
    pub last_scene_primitive_count: usize,
    pub last_frame_time_us: u64,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub workspace_mode: WorkspaceMode,
    pub compare: CompareState,
    pub repository: RepositoryState,
    pub workspace: WorkspaceState,
    pub file_list: FileListState,
    pub overlays: OverlayStackState,
    pub focus: FocusState,
    pub text_edit: TextEditState,
    pub editor: EditorState,
    pub github: GitHubState,
    pub settings: Settings,
    pub startup: StartupState,
    pub last_error: Option<String>,
    pub toasts: Vec<Toast>,
    pub animation: crate::ui::animation::AnimationState,
    pub sidebar_visible: bool,
    pub debug: DebugState,
    pub clock_ms: u64,
    pub next_toast_id: u64,
    pub frecency: Option<FrecencyStore>,
    pub theme_names: Vec<String>,
    pub theme_variants: Vec<crate::core::themes::ThemeVariant>,
    pub theme_preview_original: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            workspace_mode: WorkspaceMode::default(),
            compare: CompareState::default(),
            repository: RepositoryState::default(),
            workspace: WorkspaceState::default(),
            file_list: FileListState::default(),
            overlays: OverlayStackState::default(),
            focus: FocusState::default(),
            text_edit: TextEditState::default(),
            editor: EditorState::default(),
            github: GitHubState::default(),
            settings: Settings::default(),
            startup: StartupState::default(),
            last_error: None,
            toasts: Vec::new(),
            animation: crate::ui::animation::AnimationState::default(),
            sidebar_visible: true,
            debug: DebugState::default(),
            clock_ms: 0,
            next_toast_id: 1,
            frecency: None,
            theme_names: Vec::new(),
            theme_variants: Vec::new(),
            theme_preview_original: None,
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

        let mut state = Self {
            workspace_mode: if repo_path.is_some() && auto_compare_pending {
                WorkspaceMode::Loading
            } else {
                WorkspaceMode::Empty
            },
            compare: CompareState {
                repo_path: repo_path.clone(),
                left_ref,
                right_ref,
                mode,
                layout,
                renderer,
                resolved_left: None,
                resolved_right: None,
            },
            repository: RepositoryState::default(),
            workspace: WorkspaceState::default(),
            file_list: FileListState::default(),
            overlays: OverlayStackState::default(),
            focus: FocusState {
                current: if repo_path.is_some() {
                    Some(FocusTarget::CompareLeftRef)
                } else {
                    Some(FocusTarget::WorkspacePrimaryButton)
                },
            },
            text_edit: TextEditState::default(),
            editor: EditorState {
                layout,
                wrap_enabled: settings.viewport.wrap_enabled,
                wrap_column: settings.viewport.wrap_column,
                ..EditorState::default()
            },
            github: GitHubState {
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
            last_error: None,
            toasts: Vec::new(),
            animation: crate::ui::animation::AnimationState::default(),
            sidebar_visible: true,
            debug: DebugState::default(),
            clock_ms: 0,
            next_toast_id: 1,
            frecency: crate::core::frecency::open_default_store(),
            theme_names: Vec::new(),
            theme_variants: Vec::new(),
            theme_preview_original: None,
        };
        state.sync_settings_snapshot();

        if repo_path.is_some() && !auto_compare_pending {
            state.open_compare_sheet();
        }

        let mut effects = Vec::new();
        if let Some(path) = repo_path {
            state.repository.status = AsyncStatus::Loading;
            effects.push(Effect::LoadRepository { path });
        }
        (state, effects)
    }

    pub fn apply_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::Bootstrap => Vec::new(),
            Action::OpenRepositoryDialog => vec![Effect::OpenRepositoryDialog],
            Action::OpenRepository(path) => self.open_repository(path),
            Action::OpenCompareSheet => {
                self.open_compare_sheet();
                Vec::new()
            }
            Action::OpenRepoPicker => {
                self.open_repo_picker();
                Vec::new()
            }
            Action::OpenThemePicker => {
                self.open_theme_picker();
                Vec::new()
            }
            Action::OpenRefPicker(field) => {
                self.open_ref_picker(field);
                Vec::new()
            }
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
            Action::SetLeftRef(value) => {
                self.update_compare_field(CompareField::Left, value);
                self.persist_settings_effect()
            }
            Action::SetRightRef(value) => {
                self.update_compare_field(CompareField::Right, value);
                self.persist_settings_effect()
            }
            Action::SetCompareMode(mode) => {
                self.compare.mode = mode;
                self.overlays.compare_sheet.validation_message = None;
                self.persist_settings_effect()
            }
            Action::CycleCompareMode => {
                self.compare.mode = match self.compare.mode {
                    CompareMode::SingleCommit => CompareMode::TwoDot,
                    CompareMode::TwoDot => CompareMode::ThreeDot,
                    CompareMode::ThreeDot => CompareMode::SingleCommit,
                };
                self.overlays.compare_sheet.validation_message = None;
                self.persist_settings_effect()
            }
            Action::SetLayoutMode(layout) => {
                self.compare.layout = layout;
                self.editor.layout = layout;
                self.rebuild_command_palette();
                self.persist_settings_effect()
            }
            Action::SetRenderer(renderer) => {
                self.compare.renderer = renderer;
                self.persist_settings_effect()
            }
            Action::SetFocus(target) => {
                self.set_focus(target);
                Vec::new()
            }
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
            Action::Copy => self.copy_selection(),
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
            Action::StartCompare => self.kickoff_compare(),
            Action::SelectFile(index) => {
                self.select_loaded_file(index, false);
                Vec::new()
            }
            Action::SelectFilePath(path) => {
                if let Some(index) = self
                    .workspace
                    .files
                    .iter()
                    .position(|file| file.path == path)
                {
                    self.select_loaded_file(index, true);
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
                self.file_list
                    .scroll_rows(delta, self.workspace.files.len());
                Vec::new()
            }
            Action::ScrollFileListPx(delta_px) => {
                self.file_list
                    .scroll_px(delta_px as f32, self.workspace.files.len());
                Vec::new()
            }
            Action::ScrollFileListToPx(px) => {
                self.file_list.scroll_offset_px = px as f32;
                self.file_list.clamp_scroll(self.workspace.files.len());
                Vec::new()
            }
            Action::ScrollActiveOverlayListPx(delta_px) => {
                self.scroll_active_overlay_list_px(delta_px);
                Vec::new()
            }
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
            Action::ScrollViewportTo(scroll_top_px) => {
                self.editor.scroll_top_px = scroll_top_px;
                self.editor.clamp_scroll();
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
            Action::HoverViewportRow(row) => {
                self.editor.hovered_row = row;
                Vec::new()
            }
            Action::FocusViewport => {
                self.set_focus(Some(FocusTarget::Editor));
                Vec::new()
            }
            Action::HoverFile(index) => {
                use crate::ui::animation::AnimationKey;
                if let Some(prev) = self.file_list.hovered_index {
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
                self.file_list.hovered_index = index;
                Vec::new()
            }
            Action::SubmitPullRequest => self.submit_pull_request(),
            Action::UsePullRequestCompare => self.use_pull_request_compare(),
            Action::StartGitHubDeviceFlow => {
                self.github.auth.status = AsyncStatus::Loading;
                vec![Effect::StartDeviceFlow {
                    client_id: self.github.client_id.clone(),
                }]
            }
            Action::OpenDeviceFlowBrowser => {
                if let Some(device_flow) = self.github.auth.device_flow.as_ref() {
                    vec![Effect::OpenBrowser {
                        url: device_flow.verification_uri.clone(),
                    }]
                } else {
                    Vec::new()
                }
            }
            Action::DismissToast(index) => {
                if index < self.toasts.len() {
                    self.toasts.remove(index);
                }
                Vec::new()
            }
            Action::HoverToast(index) => {
                let hovered_id = index.and_then(|i| self.toasts.get(i)).map(|toast| toast.id);
                for toast in &mut self.toasts {
                    toast.hovered = Some(toast.id) == hovered_id;
                }
                Vec::new()
            }
            Action::ToggleWrap => {
                self.editor.wrap_enabled = !self.editor.wrap_enabled;
                self.persist_settings_effect()
            }
            Action::SetWrapColumn(column) => {
                self.editor.wrap_column = column;
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
            Action::ToggleFolder(path) => {
                if self.file_list.expanded_folders.contains(&path) {
                    self.file_list.expanded_folders.remove(&path);
                } else {
                    self.file_list.expanded_folders.insert(path);
                }
                Vec::new()
            }
            Action::ToggleFileViewed(index) => {
                if self.file_list.viewed_files.contains(&index) {
                    self.file_list.viewed_files.remove(&index);
                } else {
                    self.file_list.viewed_files.insert(index);
                }
                Vec::new()
            }
            Action::SetSidebarFilter(query) => {
                self.file_list.filter = query;
                self.file_list.scroll_offset_px = 0.0;
                Vec::new()
            }
            Action::ClearSidebarFilter => {
                self.file_list.filter.clear();
                self.file_list.scroll_offset_px = 0.0;
                Vec::new()
            }
            Action::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                Vec::new()
            }
            Action::ToggleSidebarMode => {
                self.file_list.mode = match self.file_list.mode {
                    crate::ui::state::SidebarMode::FlatList => {
                        crate::ui::state::SidebarMode::TreeView
                    }
                    crate::ui::state::SidebarMode::TreeView => {
                        crate::ui::state::SidebarMode::FlatList
                    }
                };
                self.file_list.scroll_offset_px = 0.0;
                Vec::new()
            }
            Action::ExpandAllFolders => {
                for file in &self.workspace.files {
                    let parts: Vec<&str> = file.path.split('/').collect();
                    for depth in 0..parts.len().saturating_sub(1) {
                        let folder_path = parts[..=depth].join("/");
                        self.file_list.expanded_folders.insert(folder_path);
                    }
                }
                Vec::new()
            }
            Action::CollapseAllFolders => {
                self.file_list.expanded_folders.clear();
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
            Action::ShowKeyboardShortcuts => {
                if self.overlays.top() == Some(OverlaySurface::KeyboardShortcuts) {
                    self.pop_overlay();
                } else {
                    self.push_overlay(OverlaySurface::KeyboardShortcuts, None);
                }
                Vec::new()
            }
            Action::ScrollViewportHalfPage(direction) => {
                self.scroll_viewport_half_page(direction);
                Vec::new()
            }
            Action::Noop => Vec::new(),
        }
    }

    pub fn apply_event(&mut self, event: AppEvent) -> Vec<Effect> {
        match event {
            AppEvent::RepositoryDialogClosed { path } => {
                path.map_or_else(Vec::new, |path| self.open_repository(path))
            }
            AppEvent::RepositoryLoaded(payload) => self.handle_repository_loaded(payload),
            AppEvent::RepositoryLoadFailed { path, message } => {
                if self.compare.repo_path.as_ref() == Some(&path) {
                    self.repository.status = AsyncStatus::Failed;
                    self.workspace_mode = WorkspaceMode::Empty;
                    self.push_error(&message);
                    self.open_compare_sheet();
                }
                Vec::new()
            }
            AppEvent::CompareFinished(payload) => self.handle_compare_finished(payload),
            AppEvent::CompareFailed {
                generation,
                message,
            } => {
                if generation == self.workspace.compare_generation {
                    self.workspace.status = AsyncStatus::Failed;
                    self.workspace_mode = WorkspaceMode::Empty;
                    self.overlays.compare_sheet.validation_message = Some(message.clone());
                    self.push_error(&message);
                    self.open_compare_sheet();
                }
                Vec::new()
            }
            AppEvent::PullRequestLoaded {
                url,
                info,
                left_ref,
                right_ref,
            } => {
                self.github.pull_request.status = AsyncStatus::Ready;
                self.github.pull_request.url_input = url;
                self.github.pull_request.info = Some(info);
                self.github.pull_request.candidate_left_ref = Some(left_ref);
                self.github.pull_request.candidate_right_ref = Some(right_ref);
                Vec::new()
            }
            AppEvent::PullRequestLoadFailed { message, .. } => {
                self.github.pull_request.status = AsyncStatus::Failed;
                self.push_error(&message);
                Vec::new()
            }
            AppEvent::DeviceFlowStarted(device_flow) => {
                self.github.auth.status = AsyncStatus::Loading;
                self.github.auth.device_flow = Some(device_flow.clone());
                vec![
                    Effect::OpenBrowser {
                        url: device_flow.verification_uri.clone(),
                    },
                    Effect::PollDeviceFlow {
                        client_id: self.github.client_id.clone(),
                        device_code: device_flow.device_code,
                        interval_seconds: device_flow.interval,
                    },
                ]
            }
            AppEvent::DeviceFlowStartFailed { message } => {
                self.github.auth.status = AsyncStatus::Failed;
                self.push_error(&message);
                Vec::new()
            }
            AppEvent::DeviceFlowCompleted { token } => {
                self.github.auth.status = AsyncStatus::Ready;
                self.github.auth.device_flow = None;
                self.github.auth.token_present = true;
                self.settings.github_token = Some(token);
                self.push_info("GitHub authentication completed.");
                if self.overlays.top() == Some(OverlaySurface::GitHubAuthModal) {
                    self.pop_overlay();
                }
                self.persist_settings_effect()
            }
            AppEvent::DeviceFlowFailed { message } => {
                self.github.auth.status = AsyncStatus::Failed;
                self.push_error(&message);
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
        let workspace_mode = workspace_mode_name(self.workspace_mode);
        let repo = self
            .compare
            .repo_path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .unwrap_or("native");
        if let Some(path) = self.workspace.selected_file_path.as_deref() {
            format!("diffy native - {repo} [{workspace_mode}] {path}")
        } else {
            format!("diffy native - {repo} [{workspace_mode}]")
        }
    }

    pub fn update_time(&mut self, now_ms: u64) {
        self.clock_ms = now_ms;
        self.animation.tick(now_ms);
        self.toasts.retain(|toast| {
            toast.hovered || now_ms.saturating_sub(toast.created_at_ms) < TOAST_LIFETIME_MS
        });
    }

    pub fn cursor_blink_epoch(&self) -> Option<u64> {
        self.is_text_focused().then(|| {
            self.clock_ms
                .saturating_sub(self.text_edit.cursor_moved_at_ms)
                / CURSOR_BLINK_INTERVAL_MS
        })
    }

    pub fn next_cursor_blink_at_ms(&self) -> Option<u64> {
        self.is_text_focused().then(|| {
            let elapsed = self
                .clock_ms
                .saturating_sub(self.text_edit.cursor_moved_at_ms);
            let next_epoch = elapsed / CURSOR_BLINK_INTERVAL_MS + 1;
            self.text_edit
                .cursor_moved_at_ms
                .saturating_add(next_epoch.saturating_mul(CURSOR_BLINK_INTERVAL_MS))
        })
    }

    pub fn next_toast_expiry_at_ms(&self) -> Option<u64> {
        self.toasts
            .iter()
            .filter(|toast| !toast.hovered)
            .map(|toast| toast.created_at_ms.saturating_add(TOAST_LIFETIME_MS))
            .min()
    }

    pub fn active_overlay_name(&self) -> Option<&'static str> {
        self.overlays.active_name()
    }

    fn open_repository(&mut self, path: PathBuf) -> Vec<Effect> {
        self.workspace_mode = WorkspaceMode::Loading;
        self.compare.repo_path = Some(path.clone());
        self.compare.resolved_left = None;
        self.compare.resolved_right = None;
        self.overlays.compare_sheet.validation_message = None;
        self.repository.status = AsyncStatus::Loading;
        self.workspace.clear_compare();
        self.file_list = FileListState::default();
        self.editor.clear_document();
        self.editor.focused = false;
        self.last_error = None;
        self.github.pull_request.info = None;
        self.github.pull_request.candidate_left_ref = None;
        self.github.pull_request.candidate_right_ref = None;
        self.overlays.clear();
        self.focus.current = Some(FocusTarget::CompareLeftRef);
        self.sync_settings_snapshot();
        vec![
            Effect::SaveSettings(self.settings.clone()),
            Effect::LoadRepository { path },
        ]
    }

    fn handle_repository_loaded(&mut self, payload: RepositoryLoaded) -> Vec<Effect> {
        if self.compare.repo_path.as_ref() != Some(&payload.path) {
            return Vec::new();
        }

        self.repository.status = AsyncStatus::Ready;
        self.repository.branches = payload.branches;
        self.repository.tags = payload.tags;
        self.repository.commits = payload.commits;
        if let Some(ref store) = self.frecency {
            store.record_access(&format!("repo:{}", payload.path.display()));
        }

        let mut effects = self.persist_settings_effect();
        if let Some(url) = self.startup.pending_pr_url.clone() {
            self.startup.pending_pr_url = None;
            self.github.pull_request.status = AsyncStatus::Loading;
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
            self.compare.left_ref = persisted.left_ref.clone();
            self.compare.right_ref = persisted.right_ref.clone();
            self.compare.mode = persisted.mode;
            effects.extend(self.kickoff_compare());
        } else {
            self.compare.left_ref = "HEAD".to_owned();
            self.compare.right_ref = crate::core::vcs::git::service::WORKDIR_REF.to_owned();
            self.compare.mode = CompareMode::TwoDot;
            effects.extend(self.kickoff_compare());
        }
        effects
    }

    fn handle_compare_finished(&mut self, payload: CompareFinished) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation {
            return Vec::new();
        }

        self.workspace.status = AsyncStatus::Ready;
        self.workspace_mode = WorkspaceMode::Ready;
        self.overlays.compare_sheet.validation_message = None;
        self.compare.layout = payload.spec.layout;
        self.compare.renderer = payload.spec.renderer;
        self.compare.resolved_left = Some(payload.resolved_left);
        self.compare.resolved_right = Some(payload.resolved_right);
        self.workspace.raw_diff_len = payload.output.raw_diff.len();
        self.workspace.used_fallback = payload.output.used_fallback;
        self.workspace.fallback_message = payload.output.fallback_message.clone();
        self.workspace.files = build_file_entries(&payload.output.files);
        self.workspace.compare_output = Some(payload.output);
        self.workspace.sidebar_auto_width = None;
        self.file_list.scroll_offset_px = 0.0;
        self.set_focus(Some(FocusTarget::FileList));
        self.editor.clear_document();
        self.overlays.clear();

        let preferred_index = self
            .startup
            .preferred_file_index
            .or(self.workspace.selected_file_index);
        let preferred_path = self
            .startup
            .preferred_file_path
            .clone()
            .or_else(|| self.workspace.selected_file_path.clone());

        if let Some(index) = preferred_path
            .as_deref()
            .and_then(|path| {
                self.workspace
                    .files
                    .iter()
                    .position(|file| file.path == path)
            })
            .or(preferred_index.filter(|index| *index < self.workspace.files.len()))
            .or_else(|| (!self.workspace.files.is_empty()).then_some(0))
        {
            self.select_loaded_file(index, true);
        } else {
            self.workspace.selected_file_index = None;
            self.workspace.selected_file_path = None;
            self.workspace.active_file = None;
            self.editor.clear_document();
        }

        if self.workspace.used_fallback && !self.workspace.fallback_message.is_empty() {
            self.push_info(&self.workspace.fallback_message.clone());
        }
        Vec::new()
    }

    fn kickoff_compare(&mut self) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.clone() else {
            self.overlays.compare_sheet.validation_message =
                Some("Open a repository before starting a compare.".to_owned());
            self.push_error("Open a repository before starting a compare.");
            self.open_compare_sheet();
            return Vec::new();
        };

        if !compare_refs_are_valid(
            self.compare.mode,
            &self.compare.left_ref,
            &self.compare.right_ref,
        ) {
            self.overlays.compare_sheet.validation_message =
                Some("Provide the required refs for the selected compare mode.".to_owned());
            self.push_error("Provide the required refs for the selected compare mode.");
            self.open_compare_sheet();
            return Vec::new();
        }

        self.workspace_mode = WorkspaceMode::Loading;
        self.workspace.status = AsyncStatus::Loading;
        self.overlays.compare_sheet.validation_message = None;
        self.workspace.compare_generation = self.workspace.compare_generation.saturating_add(1);
        self.overlays.clear();
        self.sync_settings_snapshot();

        vec![
            Effect::SaveSettings(self.settings.clone()),
            Effect::RunCompare {
                generation: self.workspace.compare_generation,
                request: CompareRequest {
                    repo_path,
                    spec: CompareSpec {
                        mode: self.compare.mode,
                        left_ref: self.compare.left_ref.clone(),
                        right_ref: self.compare.right_ref.clone(),
                        renderer: self.compare.renderer,
                        layout: self.compare.layout,
                    },
                    github_token: self.settings.github_token.clone(),
                },
            },
        ]
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
        self.settings.viewport.wrap_enabled = self.editor.wrap_enabled;
        self.settings.viewport.wrap_column = self.editor.wrap_column;
        self.settings.viewport.layout = self.compare.layout;
        self.settings.last_compare = Some(PersistedCompare {
            repo_path: self.compare.repo_path.clone(),
            left_ref: self.compare.left_ref.clone(),
            right_ref: self.compare.right_ref.clone(),
            mode: self.compare.mode,
            layout: self.compare.layout,
            renderer: self.compare.renderer,
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
        if target != self.focus.current {
            // Reset cursor to end of the new field
            let len = target
                .and_then(|t| self.text_for_focus(t).map(|s| s.len()))
                .unwrap_or(0);
            self.text_edit = TextEditState {
                cursor: len,
                anchor: len,
                cursor_moved_at_ms: self.clock_ms,
            };
        }
        self.focus.current = target;
        self.editor.focused = target == Some(FocusTarget::Editor);
    }

    /// Returns a reference to the text string for the given focus target, if it's a text field.
    fn text_for_focus(&self, target: FocusTarget) -> Option<&str> {
        match target {
            FocusTarget::CompareLeftRef => Some(&self.compare.left_ref),
            FocusTarget::CompareRightRef => Some(&self.compare.right_ref),
            FocusTarget::PickerInput => match self.overlays.picker.kind {
                PickerKind::Repository | PickerKind::Theme => Some(&self.overlays.picker.query),
                PickerKind::LeftRef => Some(&self.compare.left_ref),
                PickerKind::RightRef => Some(&self.compare.right_ref),
            },
            FocusTarget::CommandPaletteInput => Some(&self.overlays.command_palette.query),
            FocusTarget::PullRequestInput => Some(&self.github.pull_request.url_input),
            FocusTarget::SidebarSearch => Some(&self.file_list.filter),
            FocusTarget::SearchInput => Some(&self.editor.search.query),
            _ => None,
        }
    }

    fn focused_text(&self) -> Option<&str> {
        self.focus.current.and_then(|t| self.text_for_focus(t))
    }

    fn focused_text_mut(&mut self) -> Option<&mut String> {
        match self.focus.current {
            Some(FocusTarget::CompareLeftRef) => Some(&mut self.compare.left_ref),
            Some(FocusTarget::CompareRightRef) => Some(&mut self.compare.right_ref),
            Some(FocusTarget::PickerInput) => match self.overlays.picker.kind {
                PickerKind::Repository | PickerKind::Theme => Some(&mut self.overlays.picker.query),
                PickerKind::LeftRef => Some(&mut self.compare.left_ref),
                PickerKind::RightRef => Some(&mut self.compare.right_ref),
            },
            Some(FocusTarget::CommandPaletteInput) => {
                Some(&mut self.overlays.command_palette.query)
            }
            Some(FocusTarget::PullRequestInput) => Some(&mut self.github.pull_request.url_input),
            Some(FocusTarget::SidebarSearch) => Some(&mut self.file_list.filter),
            Some(FocusTarget::SearchInput) => Some(&mut self.editor.search.query),
            _ => None,
        }
    }

    /// Returns true if the current focus target is a text editing field.
    pub fn is_text_focused(&self) -> bool {
        self.focused_text().is_some()
    }

    fn touch_cursor(&mut self) {
        self.text_edit.cursor_moved_at_ms = self.clock_ms;
    }

    fn clamp_cursor(&mut self) {
        let Some(text) = self.focused_text() else {
            return;
        };
        let len = text.len();
        let mut cursor = self.text_edit.cursor.min(len);
        while cursor > 0 && !text.is_char_boundary(cursor) {
            cursor -= 1;
        }
        let mut anchor = self.text_edit.anchor.min(len);
        while anchor > 0 && !text.is_char_boundary(anchor) {
            anchor -= 1;
        }
        self.text_edit.cursor = cursor;
        self.text_edit.anchor = anchor;
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        let (c, a) = (self.text_edit.cursor, self.text_edit.anchor);
        if c == a {
            None
        } else {
            Some((c.min(a), c.max(a)))
        }
    }

    /// Delete the current selection and collapse cursor. Returns true if something was deleted.
    fn delete_selection(&mut self) -> bool {
        self.clamp_cursor();
        if let Some((start, end)) = self.selection_range() {
            if let Some(text) = self.focused_text_mut() {
                text.drain(start..end);
            }
            self.text_edit.cursor = start;
            self.text_edit.anchor = start;
            true
        } else {
            false
        }
    }

    /// Called after text mutation to sync compare fields and rebuild pickers.
    fn after_text_mutation(&mut self) {
        match self.focus.current {
            Some(FocusTarget::CompareLeftRef) => {
                self.compare.resolved_left = None;
            }
            Some(FocusTarget::CompareRightRef) => {
                self.compare.resolved_right = None;
            }
            Some(FocusTarget::PickerInput) => match self.overlays.picker.kind {
                PickerKind::Repository => self.rebuild_repo_picker(),
                PickerKind::LeftRef => {
                    self.compare.resolved_left = None;
                    self.rebuild_ref_picker(CompareField::Left);
                }
                PickerKind::RightRef => {
                    self.compare.resolved_right = None;
                    self.rebuild_ref_picker(CompareField::Right);
                }
                PickerKind::Theme => self.rebuild_theme_picker(),
            },
            Some(FocusTarget::CommandPaletteInput) => self.rebuild_command_palette(),
            Some(FocusTarget::SearchInput) => self.recompute_search_matches(),
            _ => {}
        }
    }

    /// Should we persist settings after editing the current field?
    fn needs_persist(&self) -> bool {
        matches!(
            self.focus.current,
            Some(FocusTarget::CompareLeftRef | FocusTarget::CompareRightRef)
        ) || matches!(
            self.focus.current,
            Some(FocusTarget::PickerInput)
                if matches!(self.overlays.picker.kind, PickerKind::LeftRef | PickerKind::RightRef)
        )
    }

    fn text_edit_effects(&mut self) -> Vec<Effect> {
        self.after_text_mutation();
        if self.needs_persist() {
            self.persist_settings_effect()
        } else {
            Vec::new()
        }
    }

    fn insert_text(&mut self, value: String) -> Vec<Effect> {
        if self.focused_text().is_none() {
            return Vec::new();
        }
        self.delete_selection();
        self.clamp_cursor();
        let cursor = self.text_edit.cursor;
        if let Some(text) = self.focused_text_mut() {
            text.insert_str(cursor, &value);
        }
        self.text_edit.cursor += value.len();
        self.text_edit.anchor = self.text_edit.cursor;
        self.touch_cursor();
        self.text_edit_effects()
    }

    fn backspace(&mut self) -> Vec<Effect> {
        if self.focused_text().is_none() {
            return Vec::new();
        }
        if self.delete_selection() {
            self.touch_cursor();
            return self.text_edit_effects();
        }
        let cursor = self.text_edit.cursor;
        if cursor == 0 {
            return Vec::new();
        }
        let prev = self
            .focused_text()
            .map(|t| prev_grapheme_boundary(t, cursor))
            .unwrap_or(0);
        if let Some(text) = self.focused_text_mut() {
            text.drain(prev..cursor);
        }
        self.text_edit.cursor = prev;
        self.text_edit.anchor = prev;
        self.touch_cursor();
        self.text_edit_effects()
    }

    fn delete_forward(&mut self) -> Vec<Effect> {
        if self.focused_text().is_none() {
            return Vec::new();
        }
        if self.delete_selection() {
            self.touch_cursor();
            return self.text_edit_effects();
        }
        let cursor = self.text_edit.cursor;
        let len = self.focused_text().map_or(0, |s| s.len());
        if cursor >= len {
            return Vec::new();
        }
        let next = self
            .focused_text()
            .map(|t| next_grapheme_boundary(t, cursor))
            .unwrap_or(cursor);
        if let Some(text) = self.focused_text_mut() {
            text.drain(cursor..next);
        }
        self.touch_cursor();
        self.text_edit_effects()
    }

    fn move_cursor(&mut self, offset: usize, extend_selection: bool) {
        self.text_edit.cursor = offset;
        if !extend_selection {
            self.text_edit.anchor = offset;
        }
        self.touch_cursor();
    }

    fn cursor_left(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let start = self.text_edit.cursor.min(self.text_edit.anchor);
            self.move_cursor(start, false);
            return;
        }
        let cursor = self.text_edit.cursor;
        if cursor == 0 {
            return;
        }
        let prev = self
            .focused_text()
            .map(|t| prev_grapheme_boundary(t, cursor))
            .unwrap_or(0);
        self.move_cursor(prev, extend);
    }

    fn cursor_right(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let end = self.text_edit.cursor.max(self.text_edit.anchor);
            self.move_cursor(end, false);
            return;
        }
        let cursor = self.text_edit.cursor;
        let len = self.focused_text().map_or(0, |s| s.len());
        if cursor >= len {
            return;
        }
        let next = self
            .focused_text()
            .map(|t| next_grapheme_boundary(t, cursor))
            .unwrap_or(cursor);
        self.move_cursor(next, extend);
    }

    fn cursor_word_left(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let start = self.text_edit.cursor.min(self.text_edit.anchor);
            self.move_cursor(start, false);
            return;
        }
        let cursor = self.text_edit.cursor;
        let pos = self
            .focused_text()
            .map(|t| prev_word_boundary(t, cursor))
            .unwrap_or(0);
        self.move_cursor(pos, extend);
    }

    fn cursor_word_right(&mut self, extend: bool) {
        if !extend && self.selection_range().is_some() {
            let end = self.text_edit.cursor.max(self.text_edit.anchor);
            self.move_cursor(end, false);
            return;
        }
        let cursor = self.text_edit.cursor;
        let len = self.focused_text().map_or(0, |s| s.len());
        let pos = self
            .focused_text()
            .map(|t| next_word_boundary(t, cursor))
            .unwrap_or(len);
        self.move_cursor(pos, extend);
    }

    fn cursor_home(&mut self, extend: bool) {
        self.move_cursor(0, extend);
    }

    fn cursor_end(&mut self, extend: bool) {
        let len = self.focused_text().map_or(0, |s| s.len());
        self.move_cursor(len, extend);
    }

    fn select_all(&mut self) {
        let len = self.focused_text().map_or(0, |s| s.len());
        self.text_edit.anchor = 0;
        self.text_edit.cursor = len;
        self.touch_cursor();
    }

    fn copy_selection(&self) -> Vec<Effect> {
        if let Some((start, end)) = self.selection_range() {
            if let Some(text) = self.focused_text() {
                let selected = text[start..end].to_string();
                return vec![Effect::SetClipboard(selected)];
            }
        }
        Vec::new()
    }

    fn cut_selection(&mut self) -> Vec<Effect> {
        let mut effects = self.copy_selection();
        if self.delete_selection() {
            self.touch_cursor();
            effects.extend(self.text_edit_effects());
        }
        effects
    }

    fn paste(&mut self, value: String) -> Vec<Effect> {
        self.insert_text(value)
    }

    fn update_compare_field(&mut self, field: CompareField, value: String) {
        match field {
            CompareField::Left => {
                self.compare.left_ref = value;
                self.compare.resolved_left = None;
            }
            CompareField::Right => {
                self.compare.right_ref = value;
                self.compare.resolved_right = None;
            }
        }
        if matches!(self.overlays.top(), Some(OverlaySurface::RefPicker(active)) if active == field)
        {
            self.rebuild_ref_picker(field);
        }
        self.rebuild_command_palette();
    }

    fn submit_pull_request(&mut self) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.clone() else {
            self.push_error("Open a repository before loading a pull request.");
            return Vec::new();
        };
        let url = self.github.pull_request.url_input.trim().to_owned();
        if url.is_empty() {
            self.push_error("Paste a GitHub pull request URL first.");
            return Vec::new();
        }
        self.github.pull_request.status = AsyncStatus::Loading;
        vec![Effect::LoadPullRequest {
            url,
            repo_path,
            github_token: self.settings.github_token.clone(),
        }]
    }

    fn use_pull_request_compare(&mut self) -> Vec<Effect> {
        let Some(left) = self.github.pull_request.candidate_left_ref.clone() else {
            self.push_error("Load a pull request before using its compare.");
            return Vec::new();
        };
        let Some(right) = self.github.pull_request.candidate_right_ref.clone() else {
            self.push_error("Load a pull request before using its compare.");
            return Vec::new();
        };
        self.update_compare_field(CompareField::Left, left);
        self.update_compare_field(CompareField::Right, right);
        self.compare.mode = CompareMode::ThreeDot;
        self.overlays.clear();
        self.kickoff_compare()
    }

    fn open_compare_sheet(&mut self) {
        self.push_overlay(
            OverlaySurface::CompareSheet,
            Some(FocusTarget::CompareLeftRef),
        );
    }

    fn open_repo_picker(&mut self) {
        let scale = self.ui_scale_factor();
        self.overlays.picker.kind = PickerKind::Repository;
        self.overlays.picker.list.row_height_px = (Sz::ROW * scale).round() as u32;
        self.overlays.picker.list.gap_px = (Sp::XS * scale).round() as u32;
        self.overlays.picker.browse_path = None;
        self.overlays.picker.selected_index = 0;
        self.overlays.picker.list.scroll_top_px = 0;

        let has_recents = crate::core::frecency::recent_repo_paths(self.frecency.as_ref(), 1)
            .first()
            .is_some();

        if has_recents {
            self.overlays.picker.query = String::new();
        } else {
            let home = dirs::home_dir()
                .map(|p| format!("{}/", p.display()))
                .unwrap_or_else(|| "/".to_owned());
            self.overlays.picker.query = home.clone();
            self.text_edit.cursor = home.len();
            self.text_edit.anchor = home.len();
            self.text_edit.cursor_moved_at_ms = self.clock_ms;
        }

        self.rebuild_repo_picker();
        self.push_overlay(OverlaySurface::RepoPicker, Some(FocusTarget::PickerInput));
    }

    fn open_theme_picker(&mut self) {
        let scale = self.ui_scale_factor();
        self.theme_preview_original = Some(self.settings.theme_name.clone());
        self.overlays.picker.kind = PickerKind::Theme;
        self.overlays.picker.query = String::new();
        self.overlays.picker.list.scroll_top_px = 0;
        self.overlays.picker.list.row_height_px = (Sz::ROW * scale).round() as u32;
        self.overlays.picker.list.gap_px = (Sp::XS * scale).round() as u32;
        self.overlays.picker.entries = self.build_theme_entries_grouped();
        self.overlays.picker.selected_index = self
            .overlays
            .picker
            .entries
            .iter()
            .position(|e| !e.section_header)
            .unwrap_or(0);
        self.push_overlay(OverlaySurface::ThemePicker, Some(FocusTarget::PickerInput));
    }

    fn build_theme_entries_grouped(&self) -> Vec<PickerEntry> {
        use crate::core::themes::ThemeVariant;

        let original = self
            .theme_preview_original
            .as_deref()
            .unwrap_or(&self.settings.theme_name);
        let make_entry = |name: &String| PickerEntry {
            label: name.clone(),
            detail: if *name == *original {
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
        let query = self.overlays.picker.query.trim().to_owned();
        let original = self
            .theme_preview_original
            .as_deref()
            .unwrap_or(&self.settings.theme_name)
            .to_owned();
        if query.is_empty() {
            self.overlays.picker.entries = self.build_theme_entries_grouped();
            self.overlays.picker.selected_index = self
                .overlays
                .picker
                .entries
                .iter()
                .position(|e| !e.section_header)
                .unwrap_or(0);
        } else {
            let haystack: Vec<&str> = self.theme_names.iter().map(|s| s.as_str()).collect();
            let config = neo_frizbee::Config {
                max_typos: Some(2),
                sort: false,
                ..Default::default()
            };
            let mut matches = neo_frizbee::match_list_indices(&query, &haystack, &config);
            matches.sort_by(|a, b| b.score.cmp(&a.score));
            self.overlays.picker.entries = matches
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
            self.overlays.picker.selected_index = 0;
        }
        let entry_count = self.overlays.picker.entries.len();
        self.overlays.picker.list.viewport_height_px = self
            .overlays
            .picker
            .list
            .viewport_for_max_rows(Sz::PICKER_MAX_ROWS, entry_count);
        self.overlays.picker.list.scroll_top_px = 0;
    }

    fn open_ref_picker(&mut self, field: CompareField) {
        let scale = self.ui_scale_factor();
        self.update_compare_field(field, String::new());
        self.overlays.picker.kind = match field {
            CompareField::Left => PickerKind::LeftRef,
            CompareField::Right => PickerKind::RightRef,
        };
        self.overlays.picker.selected_index = 0;
        self.overlays.picker.list.scroll_top_px = 0;
        self.overlays.picker.list.row_height_px = (Sz::ROW * scale).round() as u32;
        self.overlays.picker.list.gap_px = (Sp::XS * scale).round() as u32;
        self.rebuild_ref_picker(field);
        self.push_overlay(
            OverlaySurface::RefPicker(field),
            Some(FocusTarget::PickerInput),
        );
    }

    fn open_command_palette(&mut self) {
        let scale = self.ui_scale_factor();
        self.overlays.command_palette.list.row_height_px = (Sz::ROW * scale).round() as u32;
        self.overlays.command_palette.list.gap_px = (Sp::XS * scale).round() as u32;
        self.overlays.command_palette.list.scroll_top_px = 0;
        self.rebuild_command_palette();
        self.push_overlay(
            OverlaySurface::CommandPalette,
            Some(FocusTarget::CommandPaletteInput),
        );
    }

    fn push_overlay(&mut self, surface: OverlaySurface, focus_target: Option<FocusTarget>) {
        if self.overlays.top() == Some(surface) {
            self.set_focus(focus_target);
            return;
        }
        self.overlays.stack.push(OverlayEntry {
            surface,
            focus_return: self.focus.current,
        });
        self.set_focus(focus_target);
    }

    fn pop_overlay(&mut self) {
        let Some(entry) = self.overlays.stack.pop() else {
            return;
        };
        match entry.surface {
            OverlaySurface::ThemePicker => {
                if let Some(original) = self.theme_preview_original.take() {
                    self.settings.theme_name = original;
                }
                self.overlays.picker = PickerState::default();
            }
            OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_) => {
                self.overlays.picker = PickerState::default();
            }
            OverlaySurface::CommandPalette => {
                self.overlays.command_palette = CommandPaletteState::default();
            }
            _ => {}
        }
        self.set_focus(entry.focus_return);
    }

    fn move_overlay_selection(&mut self, delta: i32) {
        match self.overlays.top() {
            Some(OverlaySurface::ThemePicker) => {
                let entries = &self.overlays.picker.entries;
                let len = entries.len();
                if len == 0 {
                    return;
                }
                let max = len.saturating_sub(1) as i32;
                let mut idx =
                    (self.overlays.picker.selected_index as i32 + delta).clamp(0, max) as usize;
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
                self.overlays.picker.selected_index = idx;
                self.overlays.picker.list.reveal_index(idx, len);
                if let Some(entry) = self.overlays.picker.entries.get(idx) {
                    if !entry.section_header {
                        tracing::debug!(theme = %entry.value, "theme preview");
                        self.settings.theme_name = entry.value.clone();
                    }
                }
            }
            Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_)) => {
                let entries = &self.overlays.picker.entries;
                let len = entries.len();
                if len == 0 {
                    return;
                }
                let max = len.saturating_sub(1) as i32;
                let mut idx =
                    (self.overlays.picker.selected_index as i32 + delta).clamp(0, max) as usize;
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
                self.overlays.picker.selected_index = idx;
                self.overlays
                    .picker
                    .list
                    .reveal_index(self.overlays.picker.selected_index, len);
            }
            Some(OverlaySurface::CommandPalette) => {
                let max = self
                    .overlays
                    .command_palette
                    .entries
                    .len()
                    .saturating_sub(1) as i32;
                self.overlays.command_palette.selected_index =
                    (self.overlays.command_palette.selected_index as i32 + delta)
                        .clamp(0, max.max(0)) as usize;
                self.overlays.command_palette.list.reveal_index(
                    self.overlays.command_palette.selected_index,
                    self.overlays.command_palette.entries.len(),
                );
            }
            _ => {}
        }
    }

    fn select_overlay_entry(&mut self, index: usize) {
        match self.overlays.top() {
            Some(OverlaySurface::ThemePicker) => {
                let clamped = index.min(self.overlays.picker.entries.len().saturating_sub(1));
                self.overlays.picker.selected_index = clamped;
                if let Some(entry) = self.overlays.picker.entries.get(clamped) {
                    self.settings.theme_name = entry.value.clone();
                }
                self.overlays
                    .picker
                    .list
                    .reveal_index(clamped, self.overlays.picker.entries.len());
            }
            Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_)) => {
                let clamped = index.min(self.overlays.picker.entries.len().saturating_sub(1));
                if self
                    .overlays
                    .picker
                    .entries
                    .get(clamped)
                    .map_or(false, |e| e.section_header)
                {
                    return;
                }
                self.overlays.picker.selected_index = clamped;
                self.overlays.picker.list.reveal_index(
                    self.overlays.picker.selected_index,
                    self.overlays.picker.entries.len(),
                );
            }
            Some(OverlaySurface::CommandPalette) => {
                self.overlays.command_palette.selected_index = index.min(
                    self.overlays
                        .command_palette
                        .entries
                        .len()
                        .saturating_sub(1),
                );
                self.overlays.command_palette.list.reveal_index(
                    self.overlays.command_palette.selected_index,
                    self.overlays.command_palette.entries.len(),
                );
            }
            _ => {}
        }
    }

    fn confirm_overlay_selection(&mut self) -> Vec<Effect> {
        match self.overlays.top() {
            Some(OverlaySurface::ThemePicker) => {
                if let Some(entry) = self
                    .overlays
                    .picker
                    .entries
                    .get(self.overlays.picker.selected_index)
                {
                    tracing::info!(theme = %entry.value, "theme confirmed");
                    self.settings.theme_name = entry.value.clone();
                }
                self.theme_preview_original = None;
                self.pop_overlay();
                self.persist_settings_effect()
            }
            Some(OverlaySurface::RepoPicker) => self.confirm_repo_picker(),
            Some(OverlaySurface::RefPicker(field)) => self.confirm_ref_picker(field),
            Some(OverlaySurface::CommandPalette) => self.confirm_command_palette(),
            Some(OverlaySurface::PullRequestModal) => self.submit_pull_request(),
            Some(OverlaySurface::GitHubAuthModal) => {
                if self.github.auth.device_flow.is_some() {
                    self.apply_action(Action::OpenDeviceFlowBrowser)
                } else {
                    self.apply_action(Action::StartGitHubDeviceFlow)
                }
            }
            Some(OverlaySurface::CompareSheet) => {
                if self.focus.current == Some(FocusTarget::CompareStartButton) {
                    self.kickoff_compare()
                } else {
                    Vec::new()
                }
            }
            Some(OverlaySurface::KeyboardShortcuts) => Vec::new(),
            None => Vec::new(),
        }
    }

    fn confirm_repo_picker(&mut self) -> Vec<Effect> {
        let entry = self
            .overlays
            .picker
            .entries
            .get(self.overlays.picker.selected_index)
            .cloned();

        let Some(entry) = entry else {
            let query = self.overlays.picker.query.trim().to_owned();
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

        if self.overlays.picker.browse_path.is_some() {
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
        if self.overlays.picker.kind != PickerKind::Repository {
            return;
        }
        let entry = self
            .overlays
            .picker
            .entries
            .get(self.overlays.picker.selected_index)
            .cloned();
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
        self.overlays.picker.query = new_query.clone();
        self.text_edit.cursor = new_query.len();
        self.text_edit.anchor = new_query.len();
        self.text_edit.cursor_moved_at_ms = self.clock_ms;
        self.rebuild_repo_picker();
    }

    fn confirm_ref_picker(&mut self, field: CompareField) -> Vec<Effect> {
        let entry = self
            .overlays
            .picker
            .entries
            .get(self.overlays.picker.selected_index)
            .cloned()
            .or_else(|| {
                let query = match field {
                    CompareField::Left => self.compare.left_ref.trim(),
                    CompareField::Right => self.compare.right_ref.trim(),
                };
                (!query.is_empty()).then(|| PickerEntry {
                    label: query.to_owned(),
                    detail: "Use typed ref".to_owned(),
                    value: query.to_owned(),
                    highlights: vec![(0, query.len())],
                    icon: None,
                    section_header: false,
                })
            });
        let Some(entry) = entry else {
            return Vec::new();
        };
        if let Some(ref store) = self.frecency {
            store.record_access(&format!("ref:{}", entry.value));
        }
        self.update_compare_field(field, entry.value);
        self.pop_overlay();
        let mut effects = self.persist_settings_effect();
        if self.compare.repo_path.is_some()
            && self.workspace.status != AsyncStatus::Loading
            && compare_refs_are_valid(
                self.compare.mode,
                &self.compare.left_ref,
                &self.compare.right_ref,
            )
        {
            effects.extend(self.kickoff_compare());
        }
        effects
    }

    fn confirm_command_palette(&mut self) -> Vec<Effect> {
        let Some(entry) = self
            .overlays
            .command_palette
            .entries
            .get(self.overlays.command_palette.selected_index)
            .cloned()
        else {
            return Vec::new();
        };
        self.overlays.clear();
        match entry.kind {
            PaletteEntryKind::Command(command) => match command {
                PaletteCommand::OpenCompareSheet => {
                    self.open_compare_sheet();
                    Vec::new()
                }
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
            PaletteEntryKind::File(index) => {
                self.select_loaded_file(index, true);
                Vec::new()
            }
            PaletteEntryKind::Repo(path) => self.open_repository(path),
            PaletteEntryKind::Ref(field, value) => {
                self.update_compare_field(field, value);
                self.persist_settings_effect()
            }
        }
    }

    fn rebuild_repo_picker(&mut self) {
        let query = self.overlays.picker.query.clone();
        let trimmed = query.trim();

        if query_looks_like_path(trimmed) {
            self.rebuild_repo_picker_browse(trimmed);
        } else {
            self.overlays.picker.browse_path = None;
            self.rebuild_repo_picker_recent(trimmed);
        }

        self.overlays.picker.selected_index = if self.overlays.picker.entries.is_empty() {
            0
        } else {
            let first_selectable = self
                .overlays
                .picker
                .entries
                .iter()
                .position(|e| !e.section_header)
                .unwrap_or(0);
            self.overlays
                .picker
                .selected_index
                .max(first_selectable)
                .min(self.overlays.picker.entries.len().saturating_sub(1))
        };
        let entry_count = self.overlays.picker.entries.len();
        self.overlays.picker.list.viewport_height_px = self
            .overlays
            .picker
            .list
            .viewport_for_max_rows(Sz::PICKER_MAX_ROWS, entry_count);
        self.overlays.picker.list.clamp_scroll(entry_count);
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
        self.overlays.picker.entries = entries;
    }

    fn rebuild_repo_picker_browse(&mut self, query: &str) {
        let expanded = expand_tilde(query);
        let (dir_path, filter) = split_browse_query(&expanded);

        let dir = PathBuf::from(&dir_path);
        if !dir.is_dir() {
            self.overlays.picker.browse_path = None;
            self.overlays.picker.entries = Vec::new();
            return;
        }

        self.overlays.picker.browse_path = Some(dir.clone());

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

        self.overlays.picker.entries = entries;
    }

    fn rebuild_ref_picker(&mut self, field: CompareField) {
        let query = match field {
            CompareField::Left => self.compare.left_ref.trim(),
            CompareField::Right => self.compare.right_ref.trim(),
        };
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

        for branch in &self.repository.branches {
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

        for tag in &self.repository.tags {
            let label = tag.name.clone();
            push(
                format!("{label} Tag"),
                label.clone(),
                "Tag".to_owned(),
                label,
            );
        }

        for commit in &self.repository.commits {
            let label = commit.short_oid.clone();
            push(
                format!("{label} {} {}", commit.summary, commit.oid),
                label,
                commit.summary.clone(),
                commit.oid.clone(),
            );
        }

        if query.is_empty() {
            self.overlays.picker.entries = all_candidates
                .into_iter()
                .take(10)
                .map(|c| PickerEntry {
                    label: c.label,
                    detail: c.detail,
                    value: c.value,
                    highlights: Vec::new(),
                    icon: None,
                    section_header: false,
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
                        c.ordinal,
                        PickerEntry {
                            label: c.label.clone(),
                            detail: c.detail.clone(),
                            value: c.value.clone(),
                            highlights: highlight_ranges_for_visible_match(
                                query,
                                &c.label,
                                &m.indices,
                                &config,
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
            self.overlays.picker.entries = scored
                .into_iter()
                .map(|(_, _, entry)| entry)
                .take(10)
                .collect();
            if !self
                .overlays
                .picker
                .entries
                .iter()
                .any(|entry| entry.value == query)
            {
                self.overlays.picker.entries.insert(
                    0,
                    PickerEntry {
                        label: query.to_owned(),
                        detail: "Use typed ref".to_owned(),
                        value: query.to_owned(),
                        highlights: vec![(0, query.len())],
                        icon: None,
                        section_header: false,
                    },
                );
            }
        }

        self.overlays.picker.entries.truncate(10);
        self.overlays.picker.selected_index = self
            .overlays
            .picker
            .selected_index
            .min(self.overlays.picker.entries.len().saturating_sub(1));
        let entry_count = self.overlays.picker.entries.len();
        self.overlays.picker.list.viewport_height_px = self
            .overlays
            .picker
            .list
            .viewport_for_max_rows(Sz::PICKER_MAX_ROWS, entry_count);
        self.overlays.picker.list.clamp_scroll(entry_count);
    }

    fn rebuild_command_palette(&mut self) {
        let query = self.overlays.command_palette.query.trim();

        struct PaletteCandidate {
            search_text: String,
            label: String,
            detail: String,
            kind: PaletteEntryKind,
        }

        let mut all_candidates = Vec::new();

        for (label, detail, command) in [
            (
                "Compare Settings".to_owned(),
                "Configure compare mode, engine, and layout".to_owned(),
                PaletteCommand::OpenCompareSheet,
            ),
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

        for (index, file) in self.workspace.files.iter().enumerate() {
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

        for branch in &self.repository.branches {
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
                                query,
                                &c.label,
                                &m.indices,
                                &config,
                            ),
                        },
                    )
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.label.cmp(&b.1.label)));
            entries = scored.into_iter().map(|(_, e)| e).collect();
        }
        entries.truncate(18);
        self.overlays.command_palette.entries = entries;
        self.overlays.command_palette.selected_index =
            self.overlays.command_palette.selected_index.min(
                self.overlays
                    .command_palette
                    .entries
                    .len()
                    .saturating_sub(1),
            );
        let entry_count = self.overlays.command_palette.entries.len();
        self.overlays.command_palette.list.viewport_height_px = self
            .overlays
            .command_palette
            .list
            .viewport_for_max_rows(Sz::PICKER_MAX_ROWS, entry_count);
        self.overlays.command_palette.list.clamp_scroll(entry_count);
    }

    fn shift_loaded_file(&mut self, delta: isize) {
        if self.workspace.files.is_empty() {
            return;
        }
        let current = self.workspace.selected_file_index.unwrap_or(0);
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current
                .saturating_add(delta as usize)
                .min(self.workspace.files.len().saturating_sub(1))
        };
        self.select_loaded_file(next, true);
    }

    fn select_loaded_file(&mut self, index: usize, reveal: bool) {
        let Some(output) = self.workspace.compare_output.as_mut() else {
            self.startup.preferred_file_index = Some(index);
            return;
        };
        let Some(file) = output.files.get_mut(index) else {
            self.push_error("Selected file index is out of range.");
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

        let file = file.clone();

        self.workspace.selected_file_index = Some(index);
        self.workspace.selected_file_path = Some(file.path.clone());
        self.workspace.active_file = Some(ActiveFile {
            index,
            path: file.path.clone(),
            file: file.clone(),
            render_doc: build_render_doc(&file, index, &output.text_buffer, &output.token_buffer),
        });
        self.editor.clear_document();
        if self.editor.search.open {
            self.recompute_search_matches();
        }
        self.file_list.hovered_index = Some(index);
        if reveal {
            let row_top = index as f32 * self.file_list.row_stride();
            let row_bottom = row_top + self.file_list.row_height;
            if row_top < self.file_list.scroll_offset_px {
                self.file_list.scroll_offset_px = row_top;
            } else if row_bottom > self.file_list.scroll_offset_px + self.file_list.viewport_height
            {
                self.file_list.scroll_offset_px = row_bottom - self.file_list.viewport_height;
            }
            self.file_list.clamp_scroll(self.workspace.files.len());
        }
    }

    fn scroll_viewport_lines(&mut self, delta_lines: i32) {
        let step_px = 20_i32;
        let delta_px = delta_lines.saturating_mul(step_px);
        self.scroll_viewport_px(delta_px);
    }

    fn scroll_active_overlay_list_px(&mut self, delta_px: i32) {
        match self.overlays.top() {
            Some(
                OverlaySurface::RepoPicker
                | OverlaySurface::RefPicker(_)
                | OverlaySurface::ThemePicker,
            ) => {
                self.overlays
                    .picker
                    .list
                    .scroll_px(delta_px, self.overlays.picker.entries.len());
            }
            Some(OverlaySurface::CommandPalette) => {
                self.overlays
                    .command_palette
                    .list
                    .scroll_px(delta_px, self.overlays.command_palette.entries.len());
            }
            _ => {}
        }
    }

    fn scroll_viewport_px(&mut self, delta_px: i32) {
        self.editor.scroll_top_px = apply_scroll_delta_px(
            self.editor.scroll_top_px,
            delta_px,
            self.editor.max_scroll_top_px(),
        );
    }

    fn scroll_viewport_pages(&mut self, delta_pages: i32) {
        let page_px = ((self.editor.viewport_height_px as f32) * 0.85)
            .round()
            .max(1.0) as i32;
        let delta_px = delta_pages.saturating_mul(page_px);
        self.editor.scroll_top_px = apply_scroll_delta_px(
            self.editor.scroll_top_px,
            delta_px,
            self.editor.max_scroll_top_px(),
        );
    }

    fn scroll_viewport_half_page(&mut self, direction: i32) {
        let half_px = ((self.editor.viewport_height_px as f32) * 0.5)
            .round()
            .max(1.0) as i32;
        let delta_px = direction.saturating_mul(half_px);
        self.editor.scroll_top_px = apply_scroll_delta_px(
            self.editor.scroll_top_px,
            delta_px,
            self.editor.max_scroll_top_px(),
        );
    }

    fn navigate_to_hunk(&mut self, forward: bool) {
        let positions = &self.editor.hunk_positions;
        if positions.is_empty() {
            return;
        }
        let current = self.editor.scroll_top_px;
        let target = if forward {
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
        };
        if let Some(y) = target {
            self.editor.scroll_top_px = y;
            self.editor.clamp_scroll();
        }
    }

    fn navigate_to_file(&mut self, forward: bool) {
        let positions = &self.editor.file_positions;
        if positions.is_empty() {
            return;
        }
        let current = self.editor.scroll_top_px;
        let target = if forward {
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
        };
        if let Some(y) = target {
            self.editor.scroll_top_px = y;
            self.editor.clamp_scroll();
        }
    }

    fn push_error(&mut self, message: &str) {
        self.last_error = Some(message.to_owned());
        self.push_toast(ToastKind::Error, message);
    }

    fn push_info(&mut self, message: &str) {
        self.push_toast(ToastKind::Info, message);
    }

    fn push_toast(&mut self, kind: ToastKind, message: &str) {
        let id = self.next_toast_id;
        self.next_toast_id = self.next_toast_id.saturating_add(1);
        self.toasts.push(Toast {
            id,
            kind,
            message: message.to_owned(),
            created_at_ms: self.clock_ms,
            hovered: false,
        });
        if self.toasts.len() > MAX_VISIBLE_TOASTS {
            self.toasts.remove(0);
        }
    }

    fn open_search(&mut self) {
        self.editor.search.open = true;
        let len = self.editor.search.query.len();
        self.text_edit = TextEditState {
            cursor: len,
            anchor: 0,
            cursor_moved_at_ms: self.clock_ms,
        };
        self.focus.current = Some(FocusTarget::SearchInput);
        self.editor.focused = false;
        self.recompute_search_matches();
    }

    fn close_search(&mut self) {
        self.editor.search.open = false;
        self.editor.search.matches.clear();
        self.editor.search.active_index = None;
        self.set_focus(Some(FocusTarget::Editor));
    }

    fn recompute_search_matches(&mut self) {
        use crate::ui::editor::state::{MatchSide, SearchMatch};

        self.editor.search.matches.clear();
        self.editor.search.active_index = None;

        let query = self.editor.search.query.to_ascii_lowercase();
        if query.is_empty() {
            return;
        }

        let Some(active_file) = self.workspace.active_file.as_ref() else {
            return;
        };
        let doc = &active_file.render_doc;

        for (line_idx, line) in doc.lines.iter().enumerate() {
            let line_idx = line_idx as u32;
            if line.left_text.is_valid() {
                let text = doc.line_text(line.left_text);
                let lower = text.to_ascii_lowercase();
                let mut start = 0;
                while let Some(pos) = lower[start..].find(&query) {
                    let byte_start = (start + pos) as u32;
                    self.editor.search.matches.push(SearchMatch {
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
                    self.editor.search.matches.push(SearchMatch {
                        line_index: line_idx,
                        byte_start,
                        byte_len: query.len() as u32,
                        side: MatchSide::Right,
                    });
                    start += pos + query.len();
                }
            }
        }

        if !self.editor.search.matches.is_empty() {
            self.editor.search.active_index = Some(0);
        }
    }

    fn search_navigate(&mut self, direction: i32) {
        let count = self.editor.search.matches.len();
        if count == 0 {
            return;
        }

        let current = self.editor.search.active_index.unwrap_or(0);
        let next = if direction > 0 {
            if current + 1 >= count { 0 } else { current + 1 }
        } else {
            if current == 0 { count - 1 } else { current - 1 }
        };
        self.editor.search.active_index = Some(next);
        self.scroll_to_search_match(next);
    }

    fn scroll_to_search_match(&mut self, match_index: usize) {
        let target_y = if let Some(&y) = self.editor.search_match_y_positions.get(match_index) {
            y
        } else {
            let Some(m) = self.editor.search.matches.get(match_index) else {
                return;
            };
            self.estimate_line_y(m.line_index)
        };

        let viewport_h = self.editor.viewport_height_px;
        let centered = target_y.saturating_sub(viewport_h / 3);
        self.editor.scroll_top_px = centered.min(self.editor.max_scroll_top_px());
    }

    fn estimate_line_y(&self, line_index: u32) -> u32 {
        if self.editor.content_height_px == 0 {
            return 0;
        }
        let Some(active_file) = self.workspace.active_file.as_ref() else {
            return 0;
        };
        let total_lines = active_file.render_doc.lines.len() as u32;
        if total_lines == 0 {
            return 0;
        }
        let avg_height = self.editor.content_height_px / total_lines;
        line_index.saturating_mul(avg_height)
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

fn overlay_name(surface: OverlaySurface) -> &'static str {
    match surface {
        OverlaySurface::CompareSheet => "compare-sheet",
        OverlaySurface::RepoPicker => "repo-picker",
        OverlaySurface::RefPicker(CompareField::Left) => "left-ref-picker",
        OverlaySurface::RefPicker(CompareField::Right) => "right-ref-picker",
        OverlaySurface::CommandPalette => "command-palette",
        OverlaySurface::PullRequestModal => "pull-request-modal",
        OverlaySurface::GitHubAuthModal => "github-auth-modal",
        OverlaySurface::KeyboardShortcuts => "keyboard-shortcuts",
        OverlaySurface::ThemePicker => "theme-picker",
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

// ---------------------------------------------------------------------------
// Grapheme / word boundary helpers
// ---------------------------------------------------------------------------

/// Snap a byte offset to the nearest grapheme cluster boundary (rounding down).
fn prev_grapheme_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let mut prev = 0;
    for (idx, _) in text.grapheme_indices(true) {
        if idx >= offset {
            break;
        }
        prev = idx;
    }
    prev
}

fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    for (idx, grapheme) in text.grapheme_indices(true) {
        if idx >= offset {
            return idx + grapheme.len();
        }
    }
    text.len()
}

fn prev_word_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let bytes = text.as_bytes();
    let mut pos = offset;
    // Skip whitespace/punctuation backwards
    while pos > 0 && !bytes[pos - 1].is_ascii_alphanumeric() {
        pos -= 1;
    }
    // Skip word chars backwards
    while pos > 0 && bytes[pos - 1].is_ascii_alphanumeric() {
        pos -= 1;
    }
    pos
}

fn next_word_boundary(text: &str, offset: usize) -> usize {
    let len = text.len();
    if offset >= len {
        return len;
    }
    let bytes = text.as_bytes();
    let mut pos = offset;
    // Skip word chars forward
    while pos < len && bytes[pos].is_ascii_alphanumeric() {
        pos += 1;
    }
    // Skip whitespace/punctuation forward
    while pos < len && !bytes[pos].is_ascii_alphanumeric() {
        pos += 1;
    }
    pos
}

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
    };
    use crate::core::compare::{CompareMode, CompareOutput, LayoutMode, RendererKind};
    use crate::core::diff::{DiffLine, FileDiff, Hunk, LineKind};
    use crate::platform::persistence::Settings;
    use crate::platform::startup::{Args, StartupOptions};
    use crate::actions::Action;

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

        state.workspace.compare_output = Some(CompareOutput {
            files: files.clone(),
            ..CompareOutput::default()
        });
        state.workspace.files = files.iter().map(FileListEntry::from).collect();
        state.workspace_mode = WorkspaceMode::Ready;
        state.file_list.row_height = 36.0;
        state.file_list.gap = 4.0;
        state.file_list.viewport_height = 80.0;
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
        assert_eq!(state.workspace_mode, WorkspaceMode::Empty);
        assert_eq!(
            state.focus.current,
            Some(FocusTarget::WorkspacePrimaryButton)
        );
        assert!(effects.is_empty());
    }

    #[test]
    fn bootstrap_with_repo_opens_compare_sheet() {
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
        assert_eq!(state.workspace_mode, WorkspaceMode::Empty);
        assert_eq!(state.active_overlay_name(), Some("compare-sheet"));
        assert_eq!(effects.len(), 1);
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
        assert_eq!(state.overlays.top(), Some(OverlaySurface::CommandPalette));
        state.apply_action(Action::CloseOverlay);
        assert_eq!(state.focus.current, Some(FocusTarget::TitleBar));
    }

    #[test]
    fn pixel_scroll_actions_clamp_file_list_and_viewport() {
        let mut state = AppState::default();

        state.workspace.files = vec![
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
        ];
        state.file_list.row_height = 36.0;
        state.file_list.gap = 4.0;
        state.file_list.viewport_height = 80.0;

        state.apply_action(Action::ScrollFileListPx(50));
        assert_eq!(state.file_list.scroll_offset_px, 50.0);

        state.apply_action(Action::ScrollFileListPx(500));
        assert_eq!(state.file_list.scroll_offset_px, 116.0);

        state.apply_action(Action::ScrollFileListPx(-500));
        assert_eq!(state.file_list.scroll_offset_px, 0.0);

        state.editor.content_height_px = 600;
        state.editor.viewport_height_px = 200;

        state.apply_action(Action::ScrollViewportPx(75));
        assert_eq!(state.editor.scroll_top_px, 75);

        state.apply_action(Action::ScrollViewportPx(500));
        assert_eq!(state.editor.scroll_top_px, 400);

        state.apply_action(Action::ScrollViewportPx(-500));
        assert_eq!(state.editor.scroll_top_px, 0);
    }

    #[test]
    fn clicking_a_visible_file_does_not_force_sidebar_reveal() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.file_list.scroll_offset_px = 10.0;

        state.apply_action(Action::SelectFile(0));

        assert_eq!(state.workspace.selected_file_index, Some(0));
        assert_eq!(state.file_list.scroll_offset_px, 10.0);
    }

    #[test]
    fn keyboard_file_navigation_still_reveals_selection() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs", "d.rs"]);
        state.workspace.selected_file_index = Some(0);
        state.workspace.selected_file_path = Some("a.rs".into());
        state.file_list.scroll_offset_px = 50.0;

        state.apply_action(Action::SelectNextFile);

        assert_eq!(state.workspace.selected_file_index, Some(1));
        assert_eq!(state.file_list.scroll_offset_px, 40.0);
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
        state.workspace.compare_output = Some(output);
        state.workspace.files = vec![FileListEntry {
            path: "src/lib.rs".to_owned(),
            status: "M".to_owned(),
            additions: 0,
            deletions: 0,
            is_binary: false,
        }];
        state.workspace_mode = WorkspaceMode::Ready;

        state.apply_action(Action::SelectFile(0));

        let output = state
            .workspace
            .compare_output
            .as_ref()
            .expect("compare output");
        assert!(output.files[0].syntax_annotated);
        assert!(
            !output
                .token_buffer
                .view(output.files[0].hunks[0].lines[0].syntax_tokens)
                .is_empty()
        );

        let previous_tokens = output.files[0].hunks[0].lines[0].syntax_tokens;
        state.apply_action(Action::SelectFile(0));
        let output = state
            .workspace
            .compare_output
            .as_ref()
            .expect("compare output");
        assert_eq!(
            output.files[0].hunks[0].lines[0].syntax_tokens,
            previous_tokens
        );
    }

    #[test]
    fn overlay_list_pixel_scroll_action_clamps_active_overlay() {
        let mut state = AppState::default();
        state.overlays.stack.push(super::OverlayEntry {
            surface: OverlaySurface::RepoPicker,
            focus_return: None,
        });
        state.overlays.picker.entries = (0..12)
            .map(|index| super::PickerEntry {
                label: format!("repo-{index}"),
                detail: format!("C:\\repo-{index}"),
                value: format!("C:\\repo-{index}"),
                highlights: Vec::new(),
                icon: None,
                section_header: false,
            })
            .collect();
        state.overlays.picker.list.viewport_height_px = 120;

        state.apply_action(Action::ScrollActiveOverlayListPx(50));
        assert_eq!(state.overlays.picker.list.scroll_top_px, 50);

        state.apply_action(Action::ScrollActiveOverlayListPx(1_000));
        assert_eq!(state.overlays.picker.list.scroll_top_px, 312);

        state.apply_action(Action::ScrollActiveOverlayListPx(-1_000));
        assert_eq!(state.overlays.picker.list.scroll_top_px, 0);
    }

    #[test]
    fn ref_picker_rebuilds_matches_while_typing_and_keeps_raw_git_revisions_selectable() {
        let mut state = AppState::default();
        state.repository.branches = vec![crate::core::vcs::git::BranchInfo {
            name: "main".to_owned(),
            is_remote: false,
            is_head: true,
        }];

        state.open_ref_picker(CompareField::Left);
        state.apply_action(Action::InsertText("mai".to_owned()));

        let branch_entry = state
            .overlays
            .picker
            .entries
            .iter()
            .find(|entry| entry.value == "main")
            .expect("main branch entry");
        assert_eq!(branch_entry.highlights, vec![(0, 3)]);

        let mut state = AppState::default();
        state.open_ref_picker(CompareField::Left);
        state.apply_action(Action::InsertText("HEAD~2".to_owned()));

        let typed_entry = state
            .overlays
            .picker
            .entries
            .first()
            .expect("typed ref entry");
        assert_eq!(typed_entry.value, "HEAD~2");
        assert_eq!(typed_entry.highlights, vec![(0, "HEAD~2".len())]);

        state.apply_action(Action::ConfirmOverlaySelection);
        assert_eq!(state.compare.left_ref, "HEAD~2");
    }

    #[test]
    fn command_palette_uses_actual_match_indices_for_highlighting() {
        let mut state = AppState::default();
        state.overlays.command_palette.query = "them".to_owned();

        state.rebuild_command_palette();

        let change_theme = state
            .overlays
            .command_palette
            .entries
            .iter()
            .find(|entry| entry.label == "Change Theme")
            .expect("Change Theme entry");
        assert_eq!(change_theme.highlights, vec![(7, 11)]);
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
