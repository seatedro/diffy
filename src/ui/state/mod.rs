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
mod text_compare;
mod text_edit;
mod update;
mod working_set;
mod workspace;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use halogen::Store;
use halogen::reactive::{Signal, SignalStore};

use self::syntax::SyntaxRequestTracker;
use self::working_set::{FileWorkingSet, WorkingSetFileKey};
use crate::actions::Action;
use crate::core::compare::{
    CompareFileSummary, CompareMode, CompareOutput, ComparePath, LayoutMode, RendererKind,
};
use crate::core::forge::github::{
    CreatePullRequestReviewComment, DeviceFlowState, GitHubReviewSide, GitHubUser, PullRequestInfo,
    PullRequestReviewComment,
};
use crate::core::frecency::FrecencyStore;
use crate::core::review::{ReviewSession, ReviewSessionStatus, ReviewTarget, ReviewThreadId};
use crate::core::syntax::Highlighter;
use crate::core::syntax::annotator::{SyntaxLineTokens, SyntaxRowWindow};
use crate::core::text::TokenBuffer;
use crate::core::update::{AvailableUpdate, StagedUpdate};
use crate::core::vcs::model::{
    ChangeBucket, FileChange, FileChangeStatus, FileOperation, JjOperation, PublishAction,
    PublishPlan, RefKind, RepoCapabilities, RepoLocation, VCS_PROFILE_GIT, VCS_PROFILE_JJ,
    VcsChange, VcsCompareRequest, VcsCompareSpec, VcsOperation, VcsOperationLogEntry, VcsRef,
};
use crate::core::vcs::patch;
use crate::editor::diff::render_doc::{
    CarbonStyleOverlays, RenderDoc, RenderLine, RenderRowKind, build_placeholder_render_doc,
    build_render_doc_from_carbon,
};
use crate::editor::diff::state::{EditorState, EditorStateStore, SearchMatch};
use crate::editor::{Editor, EditorMode};
use crate::effects::{
    AiEffect, BatchFileOperationRequest, CommitRequest, CompareEffect, CompareFileRequest,
    CompareFileStatsItem, CompareFileStatsRequest, CompareHistoryRequest, CompareRequest,
    CompareStatsRequest, CompareWorkPriority, Effect, FetchRemoteRequest, FileOperationRequest,
    GitHubEffect, LoadFileSyntaxRequest, PatchOperationRequest, PublishPlanRequest, PublishRequest,
    PullFfRequest, PushRequest, RepositoryEffect, SettingsEffect, StatusDiffRequest, SyntaxEffect,
    Task, TextCompareRequest, UiEffect, UpdateEffect, VcsOperationRequest,
};
use crate::events::{
    AppEvent, CompareFileFinished, CompareFileStat, CompareFileStatsReady, CompareFinished,
    CompareHistoryReady, CompareStatsReady, FileSyntaxReady, RepositoryChangeKind,
    RepositorySnapshot, RepositorySyncReason, StatusDiffFinished, TextCompareFinished,
};
use crate::fonts::{FontFamilyEntry, FontRole};
use crate::platform::persistence::{PersistedCompare, Settings};
use crate::platform::secrets::AiKeyKind;
use crate::platform::startup::{GitHubTokenStore, StartupOptions};
use crate::ui::components::ContextMenuState;
use crate::ui::design::{Sp, Sz};
use crate::ui::icons::lucide;
use crate::ui::theme::ThemeMode;

const MAX_VISIBLE_TOASTS: usize = 5;
const TOAST_LIFETIME_MS: u64 = 5_000;
const TOAST_ANIM_MS: u64 = 150;
const CURSOR_BLINK_INTERVAL_MS: u64 = 530;
const LARGE_COMPARE_FILE_LINES: i32 = 1_500;
const COMPARE_STATS_CHUNK_SIZE: usize = 64;
const COMPARE_STATS_BACKGROUND_CHUNK_SIZE: usize = 128 * 1024;
const COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT: usize = 10_000;
const COMPARE_STATS_VISIBLE_OVERSCAN_ROWS: usize = 32;
const SYNTAX_INITIAL_ROWS: usize = 200;
const SYNTAX_OVERSCAN_ROWS: usize = 160;
const MAX_PENDING_SYNTAX_WINDOWS: usize = 96;
const COMPARE_WORKING_SET_MAX_FILES: usize = 96;
const COMPARE_WORKING_SET_MIN_FILES: usize = 24;
const COMPARE_WORKING_SET_BYTE_BUDGET: usize = 64 * 1024 * 1024;
const COMPARE_WORKING_SET_PREFETCH_PAGES: u32 = 3;
const COMPARE_WORKING_SET_TRAILING_PAGES: u32 = 1;
const CONTINUOUS_BOTTOM_ANCHOR_TOLERANCE_PX: u32 = 2;

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
    Keymaps,
    Clankers,
    About,
}

impl SettingsSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Editor => "Editor",
            Self::Behavior => "Behavior",
            Self::Keymaps => "Keymaps",
            Self::Clankers => "Clankers",
            Self::About => "About",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Appearance => lucide::SUN,
            Self::Editor => lucide::FILE_CODE,
            Self::Behavior => lucide::SETTINGS,
            Self::Keymaps => lucide::KEY,
            Self::Clankers => lucide::SPARKLES,
            Self::About => lucide::INFO,
        }
    }

    pub const ALL: [Self; 6] = [
        Self::Appearance,
        Self::Editor,
        Self::Behavior,
        Self::Keymaps,
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
    TextCompare,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextCompareView {
    #[default]
    Edit,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextCompareSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextCompareLanguage {
    #[default]
    Auto,
    PlainText,
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Json,
    Toml,
    Shell,
    Nix,
    C,
    Cpp,
    Zig,
}

impl TextCompareLanguage {
    pub const OPTIONS: &'static [Self] = &[
        Self::Auto,
        Self::PlainText,
        Self::Rust,
        Self::TypeScript,
        Self::JavaScript,
        Self::Python,
        Self::Go,
        Self::Json,
        Self::Toml,
        Self::Shell,
        Self::Nix,
        Self::C,
        Self::Cpp,
        Self::Zig,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::PlainText => "Plain text",
            Self::Rust => "Rust",
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
            Self::Python => "Python",
            Self::Go => "Go",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Shell => "Shell",
            Self::Nix => "Nix",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Zig => "Zig",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::PlainText => "Text",
            Self::Rust => "Rust",
            Self::TypeScript => "TS",
            Self::JavaScript => "JS",
            Self::Python => "Py",
            Self::Go => "Go",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Shell => "Sh",
            Self::Nix => "Nix",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Zig => "Zig",
        }
    }

    pub fn scratch_path(self) -> &'static str {
        match self {
            Self::Auto | Self::PlainText => "text.txt",
            Self::Rust => "scratch.rs",
            Self::TypeScript => "scratch.ts",
            Self::JavaScript => "scratch.js",
            Self::Python => "scratch.py",
            Self::Go => "scratch.go",
            Self::Json => "scratch.json",
            Self::Toml => "scratch.toml",
            Self::Shell => "scratch.sh",
            Self::Nix => "scratch.nix",
            Self::C => "scratch.c",
            Self::Cpp => "scratch.cpp",
            Self::Zig => "scratch.zig",
        }
    }
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
    TextCompareLeft,
    TextCompareRight,
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
                | Self::TextCompareLeft
                | Self::TextCompareRight
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

#[derive(Debug, Clone)]
pub struct TextCompareState {
    pub left_editor: Editor,
    pub right_editor: Editor,
    pub language: TextCompareLanguage,
    pub detected_language: Option<TextCompareLanguage>,
    pub path_hint: String,
    pub view: TextCompareView,
    pub generation: u64,
    pub last_compared_generation: Option<u64>,
    pub status: AsyncStatus,
}

impl Default for TextCompareState {
    fn default() -> Self {
        let mut left_editor = Editor::new(EditorMode::CodeInput);
        let mut right_editor = Editor::new(EditorMode::CodeInput);
        left_editor.set_syntax_path("text.txt");
        right_editor.set_syntax_path("text.txt");
        Self {
            left_editor,
            right_editor,
            language: TextCompareLanguage::Auto,
            detected_language: None,
            path_hint: "text.txt".to_owned(),
            view: TextCompareView::default(),
            generation: 0,
            last_compared_generation: None,
            status: AsyncStatus::Idle,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct RepositoryState {
    pub status: AsyncStatus,
    pub location: Option<RepoLocation>,
    pub capabilities: Option<RepoCapabilities>,
    pub refs: Vec<VcsRef>,
    pub changes: Vec<VcsChange>,
    pub operation_log: Vec<VcsOperationLogEntry>,
    pub file_changes: Vec<FileChange>,
    pub publish_plan: Option<PublishPlan>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveFileLoading {
    pub index: usize,
    pub path: String,
    pub priority: CompareWorkPriority,
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
/// `reveal_at_ms` controls when the panel is rendered. Compares show
/// immediately; repo-open still uses the short delay to avoid flashing a
/// loading panel for tiny repositories.
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
    pub render_doc: Arc<RenderDoc>,
    pub token_buffer: TokenBuffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportDocumentMode {
    Single,
    Continuous,
}

#[derive(Debug, Clone)]
pub struct ViewportDocument {
    pub doc: Arc<RenderDoc>,
    pub mode: ViewportDocumentMode,
    pub generation: u64,
    pub start_index: usize,
    pub start_offset_px: u32,
    pub scroll_top_px: u32,
    pub slot_indices: Vec<usize>,
    pub slot_item_ids: Vec<VirtualDiffItemId>,
    pub stream_items: Vec<VirtualDiffStreamItem>,
    pub slot_loading: Vec<bool>,
    pub path: String,
}

impl ViewportDocument {
    pub fn single(doc: Arc<RenderDoc>, generation: u64, file_index: usize, path: String) -> Self {
        Self {
            doc,
            mode: ViewportDocumentMode::Single,
            generation,
            start_index: file_index,
            start_offset_px: 0,
            scroll_top_px: 0,
            slot_indices: vec![file_index],
            slot_item_ids: vec![VirtualDiffItemId::file(
                WorkspaceSource::None,
                generation,
                file_index,
            )],
            stream_items: Vec::new(),
            slot_loading: vec![false],
            path,
        }
    }

    pub fn is_continuous(&self) -> bool {
        self.mode == ViewportDocumentMode::Continuous
    }

    pub fn insert_stream_item(&mut self, item: VirtualDiffStreamItem) {
        let index = self
            .stream_items
            .partition_point(|existing| existing.sort_key <= item.sort_key);
        self.stream_items.insert(index, item);
    }
}

fn virtual_stream_item_kind(
    slot: &ViewportSlotKey,
    line: &RenderLine,
) -> Option<VirtualDiffItemKind> {
    match line.row_kind() {
        RenderRowKind::FileHeader => Some(VirtualDiffItemKind::FileHeader),
        RenderRowKind::HunkSeparator
            if matches!(slot.kind, ViewportSlotKind::Loading) || line.hunk_index < 0 =>
        {
            Some(VirtualDiffItemKind::LoadingPlaceholder)
        }
        RenderRowKind::HunkSeparator => Some(VirtualDiffItemKind::Hunk),
        RenderRowKind::Context
        | RenderRowKind::Added
        | RenderRowKind::Removed
        | RenderRowKind::Modified => Some(VirtualDiffItemKind::DiffRow),
        RenderRowKind::Block => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualDiffItemKind {
    File,
    FileHeader,
    Hunk,
    DiffRow,
    ReviewThread,
    ReviewComment,
    Composer,
    LoadingPlaceholder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualDiffItemId {
    pub source: WorkspaceSource,
    pub generation: u64,
    pub kind: VirtualDiffItemKind,
    pub index: usize,
    pub ordinal: u32,
    pub stable_key: u64,
}

impl VirtualDiffItemId {
    fn file(source: WorkspaceSource, generation: u64, index: usize) -> Self {
        Self {
            source,
            generation,
            kind: VirtualDiffItemKind::File,
            index,
            ordinal: 0,
            stable_key: 0,
        }
    }

    pub fn new(
        source: WorkspaceSource,
        generation: u64,
        kind: VirtualDiffItemKind,
        index: usize,
        ordinal: u32,
        stable_key: u64,
    ) -> Self {
        Self {
            source,
            generation,
            kind,
            index,
            ordinal,
            stable_key,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualDiffStreamItem {
    pub id: VirtualDiffItemId,
    pub sort_key: u64,
    pub estimated_height_px: u32,
    pub measured_height_px: Option<u32>,
}

impl VirtualDiffStreamItem {
    pub fn new(
        id: VirtualDiffItemId,
        sort_key: u64,
        estimated_height_px: u32,
        measured_height_px: Option<u32>,
    ) -> Self {
        Self {
            id,
            sort_key,
            estimated_height_px,
            measured_height_px,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportAnchorBias {
    PreserveTop,
    PreserveBottom,
    FollowEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportAnchor {
    pub item_id: VirtualDiffItemId,
    pub intra_item_offset_px: u32,
    pub bias: ViewportAnchorBias,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ViewportSlotKey {
    source: WorkspaceSource,
    index: usize,
    path: String,
    left_ref: String,
    right_ref: String,
    kind: ViewportSlotKind,
}

impl ViewportSlotKey {
    fn working_set_key(&self) -> Option<WorkingSetFileKey> {
        if self.source == WorkspaceSource::None {
            return None;
        }
        Some(WorkingSetFileKey::new(
            self.index,
            self.path.clone(),
            self.left_ref.clone(),
            self.right_ref.clone(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ViewportSlotKind {
    Text {
        line_count: usize,
        text_len: usize,
        style_run_count: usize,
        syntax_covered_count: usize,
    },
    Binary,
    Loading,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ViewportDocumentKey {
    source: WorkspaceSource,
    generation: u64,
    start_index: usize,
    slots: Vec<ViewportSlotKey>,
}

#[derive(Debug, Clone)]
struct ViewportDocumentCache {
    key: ViewportDocumentKey,
    doc: Arc<RenderDoc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollDirection {
    Backward,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxPendingWindow {
    request_id: u64,
    window: SyntaxRowWindow,
}

fn file_change_list_status(status: FileChangeStatus, bucket: ChangeBucket) -> FileListStatus {
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

fn vcs_compare_request(
    mode: CompareMode,
    left_ref: String,
    right_ref: String,
    layout: LayoutMode,
    renderer: RendererKind,
) -> VcsCompareRequest {
    let compare_spec = match mode {
        CompareMode::SingleCommit => {
            let revision = if right_ref.is_empty() {
                left_ref
            } else {
                right_ref
            };
            VcsCompareSpec::Change { revision }
        }
        CompareMode::TwoDot => VcsCompareSpec::Range {
            from: left_ref,
            to: right_ref,
        },
        CompareMode::ThreeDot => VcsCompareSpec::MergeBaseRange {
            base: left_ref,
            head: right_ref,
        },
    };
    VcsCompareRequest {
        spec: compare_spec,
        layout,
        renderer,
    }
}

fn append_active_file_doc(out: &mut RenderDoc, active: &ActiveFile) {
    if active.carbon_file.is_binary {
        out.append_doc(&build_placeholder_render_doc(
            &active.path,
            "Binary file. Diffy only shows text diffs here.",
        ));
    } else {
        out.append_doc(&active.render_doc);
    }
}

fn request_syntax_for_active_file(
    active: &mut ActiveFile,
    repo_path: PathBuf,
    generation: u64,
    syntax_epoch: u64,
    window: SyntaxRowWindow,
    request_id: u64,
) -> Option<LoadFileSyntaxRequest> {
    let window = next_missing_syntax_tile(active, window)?;
    if active
        .syntax_pending
        .iter()
        .any(|pending| pending.window.contains(window))
        || active
            .syntax_covered
            .iter()
            .any(|covered| covered.contains(window))
    {
        return None;
    }

    active
        .syntax_pending
        .push(SyntaxPendingWindow { request_id, window });
    Some(LoadFileSyntaxRequest {
        repo_path,
        file_index: active.index,
        path: active.path.clone(),
        carbon_file: active.carbon_file.clone(),
        carbon_expansion: active.carbon_expansion.clone(),
        left_ref: active.left_ref.clone(),
        right_ref: active.right_ref.clone(),
        window,
        request_id,
        cache_generation: generation,
        syntax_epoch,
    })
}

fn next_missing_syntax_tile(
    active: &ActiveFile,
    requested: SyntaxRowWindow,
) -> Option<SyntaxRowWindow> {
    let line_count = active.render_doc.lines.len();
    let start = requested.start.min(line_count);
    let end = requested.end.min(line_count);
    if line_count == 0 || end <= start {
        return None;
    }

    let tile_rows = SYNTAX_INITIAL_ROWS.max(1);
    let mut tile_start = (start / tile_rows) * tile_rows;
    while tile_start < end {
        let tile_end = tile_start.saturating_add(tile_rows).min(line_count);
        let candidate = SyntaxRowWindow {
            start: tile_start,
            end: tile_end,
        };
        let already_requested = active
            .syntax_pending
            .iter()
            .any(|pending| pending.window.contains(candidate))
            || active
                .syntax_covered
                .iter()
                .any(|covered| covered.contains(candidate));
        if !already_requested {
            return Some(candidate);
        }
        if tile_end == line_count {
            break;
        }
        tile_start = tile_end;
    }
    None
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

fn active_file_matches_language(
    active: &ActiveFile,
    highlighter: &Highlighter,
    language: &str,
) -> bool {
    !active.carbon_file.is_binary
        && [
            Some(active.path.as_str()),
            active.carbon_file.old_path.as_deref(),
            active.carbon_file.new_path.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|path| {
            highlighter
                .resolve_language(path)
                .is_some_and(|resolved| resolved.name() == language)
        })
}

fn file_change_syntax_paths(change: &FileChange) -> Vec<String> {
    let mut paths = Vec::with_capacity(2);
    if let Some(old_path) = change.old_path.as_ref() {
        paths.push(old_path.clone());
    }
    if !paths.iter().any(|path| path == &change.path) {
        paths.push(change.path.clone());
    }
    paths
}

fn ensure_syntax_packs_for_file_change_effect(change: &FileChange) -> Effect {
    let mut paths = file_change_syntax_paths(change);
    if paths.len() == 1 {
        return SyntaxEffect::EnsureSyntaxPackForPath {
            path: paths.pop().unwrap_or_else(|| change.path.clone()),
        }
        .into();
    }
    SyntaxEffect::EnsureSyntaxPacksForPaths { paths }.into()
}

fn reset_active_file_syntax(active: &mut ActiveFile) {
    active.syntax_pending.clear();
    active.syntax_covered.clear();
    let preserve_change_tokens = active.carbon_overlays.has_change_tokens();
    active.carbon_overlays.clear_syntax();
    if !preserve_change_tokens {
        active.token_buffer.clear();
    }
    active.render_doc = Arc::new(build_render_doc_from_carbon(
        &active.carbon_file,
        active.index,
        &active.carbon_expansion,
        &active.carbon_overlays,
        &active.token_buffer,
    ));
}

fn apply_compare_stat_to_active_file(active: &mut ActiveFile, stat: &CompareFileStat) -> bool {
    if active.index != stat.index || active.path != stat.path {
        return false;
    }

    let additions = i32_to_u32_nonnegative(stat.additions);
    let deletions = i32_to_u32_nonnegative(stat.deletions);
    let carbon_file = Arc::make_mut(&mut active.carbon_file);
    if carbon_file.additions == additions
        && carbon_file.deletions == deletions
        && !carbon_file.stats_deferred
    {
        return false;
    }

    carbon_file.additions = additions;
    carbon_file.deletions = deletions;
    carbon_file.stats_deferred = false;
    active.render_doc = Arc::new(build_render_doc_from_carbon(
        &active.carbon_file,
        active.index,
        &active.carbon_expansion,
        &active.carbon_overlays,
        &active.token_buffer,
    ));
    true
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

fn remove_pending_syntax_window(
    pending: &mut Vec<SyntaxPendingWindow>,
    request_id: u64,
    window: SyntaxRowWindow,
) -> bool {
    let Some(index) = pending
        .iter()
        .position(|pending| pending.request_id == request_id && pending.window == window)
    else {
        return false;
    };
    pending.swap_remove(index);
    true
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

fn text_store_estimated_bytes(text: &carbon::TextStore) -> usize {
    text.as_bytes()
        .len()
        .saturating_add(text.line_count() as usize * std::mem::size_of::<u32>())
}

fn render_doc_estimated_bytes(doc: &RenderDoc) -> usize {
    doc.text_bytes
        .len()
        .saturating_add(
            doc.style_runs.len() * std::mem::size_of::<crate::editor::diff::render_doc::StyleRun>(),
        )
        .saturating_add(
            doc.lines.len() * std::mem::size_of::<crate::editor::diff::render_doc::RenderLine>(),
        )
        .saturating_add(
            doc.file_metadata
                .iter()
                .map(|meta| {
                    meta.path
                        .len()
                        .saturating_add(meta.old_path.as_ref().map_or(0, String::len))
                })
                .sum::<usize>(),
        )
}

fn carbon_file_estimated_bytes(file: &carbon::FileDiff) -> usize {
    file.old_path
        .as_ref()
        .map_or(0, String::len)
        .saturating_add(file.new_path.as_ref().map_or(0, String::len))
        .saturating_add(file.old_oid.as_ref().map_or(0, |oid| oid.0.len()))
        .saturating_add(file.new_oid.as_ref().map_or(0, |oid| oid.0.len()))
        .saturating_add(file.old_mode.as_ref().map_or(0, |mode| mode.0.len()))
        .saturating_add(file.new_mode.as_ref().map_or(0, |mode| mode.0.len()))
        .saturating_add(file.old_text.as_ref().map_or(0, text_store_estimated_bytes))
        .saturating_add(file.new_text.as_ref().map_or(0, text_store_estimated_bytes))
        .saturating_add(file.hunks.len() * std::mem::size_of::<carbon::Hunk>())
        .saturating_add(
            file.hunks
                .iter()
                .map(|hunk| hunk.header.len())
                .sum::<usize>(),
        )
        .saturating_add(file.blocks.len() * std::mem::size_of::<carbon::Block>())
        .saturating_add(
            file.blocks
                .iter()
                .map(|block| {
                    block.old_inline.len() * std::mem::size_of::<carbon::InlineSpan>()
                        + block.new_inline.len() * std::mem::size_of::<carbon::InlineSpan>()
                })
                .sum::<usize>(),
        )
}

fn line_vec_estimated_bytes(lines: &Arc<Vec<String>>) -> usize {
    lines
        .iter()
        .map(|line| {
            std::mem::size_of::<String>()
                .saturating_add(line.len())
                .saturating_add(1)
        })
        .fold(0usize, usize::saturating_add)
}

fn i32_to_u32_nonnegative(value: i32) -> u32 {
    u32::try_from(value).unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct ActiveFile {
    pub index: usize,
    pub path: String,
    pub carbon_file: Arc<carbon::FileDiff>,
    pub carbon_expansion: carbon::ExpansionState,
    pub carbon_overlays: CarbonStyleOverlays,
    pub render_doc: Arc<RenderDoc>,
    pub token_buffer: TokenBuffer,
    pub left_ref: String,
    pub right_ref: String,
    pub file_line_count: Option<u32>,
    pub old_file_lines: Option<Arc<Vec<String>>>,
    pub file_lines: Option<Arc<Vec<String>>>,
    pub syntax_pending: Vec<SyntaxPendingWindow>,
    pub syntax_covered: Vec<SyntaxRowWindow>,
    pub last_used_tick: u64,
}

impl ActiveFile {
    fn working_set_key(&self) -> WorkingSetFileKey {
        WorkingSetFileKey::new(
            self.index,
            self.path.clone(),
            self.left_ref.clone(),
            self.right_ref.clone(),
        )
    }

    fn working_set_bytes(&self) -> usize {
        self.path
            .len()
            .saturating_add(self.left_ref.len())
            .saturating_add(self.right_ref.len())
            .saturating_add(render_doc_estimated_bytes(&self.render_doc))
            .saturating_add(
                self.token_buffer
                    .len()
                    .saturating_mul(std::mem::size_of::<crate::core::text::DiffTokenSpan>()),
            )
            .saturating_add(carbon_file_estimated_bytes(&self.carbon_file))
            .saturating_add(
                self.old_file_lines
                    .as_ref()
                    .map_or(0, line_vec_estimated_bytes),
            )
            .saturating_add(self.file_lines.as_ref().map_or(0, line_vec_estimated_bytes))
    }
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
        render_doc: Arc::new(render_doc),
        token_buffer,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidebarWidthCache {
    pub compare_generation: u64,
    pub ui_scale_pct: u16,
    pub intrinsic_width_px: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportScrollbarMetrics {
    pub content_height_px: u32,
    pub viewport_height_px: u32,
    pub scroll_top_px: u32,
    pub max_scroll_top_px: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportScrollbarDragState {
    pub metrics: ViewportScrollbarMetrics,
    pub file_heights_px: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompareStatsHydrationState {
    #[default]
    Idle,
    Running,
    Failed,
}

#[derive(Debug, Clone, Default, Store)]
pub struct WorkspaceState {
    pub source: WorkspaceSource,
    pub status: AsyncStatus,
    pub status_operation_pending: bool,
    pub compare_generation: u64,
    pub status_generation: u64,
    pub files: Vec<FileListEntry>,
    pub status_file_changes: Vec<FileChange>,
    pub selected_file_index: Option<usize>,
    pub selected_file_path: Option<String>,
    pub selected_change_bucket: Option<ChangeBucket>,
    pub compare_output: Option<CompareOutput>,
    pub compare_total_stats: Option<(i32, i32)>,
    pub compare_hydrated_stats: Option<(i32, i32)>,
    pub compare_deferred_stats_remaining: Option<usize>,
    pub compare_deferred_stats_cursor: usize,
    pub compare_total_stats_loading: bool,
    pub compare_stats_hydration: CompareStatsHydrationState,
    pub active_file: Option<ActiveFile>,
    pub active_file_loading: Option<ActiveFileLoading>,
    pub file_cache: HashMap<usize, ActiveFile>,
    pub file_cache_loading: HashMap<usize, ActiveFileLoading>,
    pub raw_diff_len: usize,
    pub used_fallback: bool,
    pub fallback_message: String,
    pub sidebar_auto_width: Option<SidebarWidthCache>,
    pub range_commits: Vec<VcsChange>,
    pub compare_history_pending: Option<CompareHistoryRequest>,
    pub pre_drill_compare: Option<(String, String, CompareMode)>,
    pub expansions: HashMap<String, carbon::ExpansionState>,
    pub file_content_heights: Vec<Option<u32>>,
    pub file_scroll_total_height_px: u32,
    pub pending_file_content_heights: HashMap<usize, u32>,
    pub file_scroll_recompute_pending: bool,
    pub global_scroll_top_px: u32,
    pub measured_px_per_row_q16: u32,
    pub viewport_scrollbar_drag: Option<ViewportScrollbarDragState>,
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

    pub fn keymaps_max_scroll_px(&self) -> f32 {
        let content = self.keymaps_content_height_px.get(&self.store);
        let viewport = self.keymaps_viewport_height_px.get(&self.store);
        (content - viewport).max(0.0)
    }

    pub fn clamp_keymaps_scroll(&mut self) {
        let max = self.keymaps_max_scroll_px();
        let cur = self.keymaps_scroll_top_px.get(&self.store);
        self.keymaps_scroll_top_px
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

    fn sidebar_row_index_for_file(&self, index: usize) -> usize {
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

    fn compare_file_is_large(&self, index: usize) -> bool {
        if self.workspace.source.get(&self.store) == WorkspaceSource::TextCompare {
            return false;
        }
        if self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .is_some_and(|output| compare_output_summary_is_deferred(output, index))
        }) {
            return true;
        }

        let meta = self.file_list_entry_meta(index);
        !meta.is_binary && meta.additions.saturating_add(meta.deletions) >= LARGE_COMPARE_FILE_LINES
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
            carbon_file: Arc::new(prepared.carbon_file),
            carbon_expansion: prepared.carbon_expansion.clone(),
            carbon_overlays: prepared.carbon_overlays,
            render_doc: prepared.render_doc,
            token_buffer: prepared.token_buffer,
            left_ref,
            right_ref,
            file_line_count: None,
            old_file_lines: None,
            file_lines: None,
            syntax_pending: Vec::new(),
            syntax_covered: Vec::new(),
            last_used_tick: 0,
        }
    }

    fn clear_file_cache(&mut self) {
        self.workspace.file_cache.set(&self.store, HashMap::new());
        self.workspace
            .file_cache_loading
            .set(&self.store, HashMap::new());
        self.viewport_document_cache = None;
        self.last_virtual_scroll_top_px = None;
        self.file_working_set.reset();
    }

    fn next_file_working_set_tick(&mut self) -> u64 {
        self.file_working_set.next_tick()
    }

    fn syntax_pending_window_count(&self) -> usize {
        let active_count = self.workspace.active_file.with(&self.store, |active| {
            active
                .as_ref()
                .map_or(0, |active| active.syntax_pending.len())
        });
        let cache_count = self.workspace.file_cache.with(&self.store, |files| {
            files
                .values()
                .map(|file| file.syntax_pending.len())
                .sum::<usize>()
        });
        active_count.saturating_add(cache_count)
    }

    fn syntax_outstanding_window_count(&self) -> usize {
        self.syntax_requests
            .outstanding_count(self.syntax_pending_window_count())
    }

    fn syntax_request_budget_available(&self) -> bool {
        self.syntax_requests
            .budget_available(self.syntax_pending_window_count())
    }

    fn track_syntax_request(&mut self, request: &LoadFileSyntaxRequest) {
        self.syntax_requests.track(request);
    }

    fn finish_syntax_request(&mut self, generation: u64, request_id: u64) {
        self.syntax_requests.finish(generation, request_id);
    }

    fn clear_syntax_pending_windows(&mut self) {
        self.workspace.active_file.update(&self.store, |active| {
            if let Some(active) = active.as_mut() {
                active.syntax_pending.clear();
            }
        });
        self.workspace.file_cache.update(&self.store, |files| {
            for active in files.values_mut() {
                active.syntax_pending.clear();
            }
        });
    }

    fn clear_syntax_inflight(&mut self) {
        self.clear_syntax_pending_windows();
        self.syntax_requests.invalidate();
    }

    fn syntax_epoch_effect(&self) -> Effect {
        SyntaxEffect::SetFileSyntaxEpoch {
            epoch: self.syntax_requests.epoch(),
        }
        .into()
    }

    fn invalidate_syntax_epoch_effect(&mut self) -> Effect {
        self.clear_syntax_inflight();
        self.syntax_epoch_effect()
    }

    fn protect_working_set_slots(&mut self, slots: &[ViewportSlotKey]) {
        self.file_working_set.protect_slots(slots);
    }

    fn cache_active_file(&mut self, mut active_file: ActiveFile) -> ActiveFile {
        let index = active_file.index;
        active_file.last_used_tick = self.next_file_working_set_tick();
        let cached = active_file.clone();
        self.workspace.file_cache.update(&self.store, |files| {
            files.insert(index, cached);
        });
        self.workspace
            .file_cache_loading
            .update(&self.store, |files| {
                files.remove(&index);
            });
        self.trim_file_working_set();
        active_file
    }

    fn touch_viewport_slot(&mut self, key: &ViewportSlotKey) {
        let tick = self.next_file_working_set_tick();
        self.workspace.active_file.update(&self.store, |slot| {
            if let Some(active) = slot.as_mut()
                && active.index == key.index
                && active.path == key.path
                && active.left_ref == key.left_ref
                && active.right_ref == key.right_ref
            {
                active.last_used_tick = tick;
            }
        });
        self.workspace.file_cache.update(&self.store, |files| {
            if let Some(active) = files.get_mut(&key.index)
                && active.index == key.index
                && active.path == key.path
                && active.left_ref == key.left_ref
                && active.right_ref == key.right_ref
            {
                active.last_used_tick = tick;
            }
        });
    }

    fn trim_file_working_set(&mut self) {
        let mut keep = self.file_working_set.protected_snapshot();
        if let Some(active) = self.workspace.active_file.with(&self.store, |active| {
            active.as_ref().map(ActiveFile::working_set_key)
        }) {
            keep.insert(active);
        }
        if let Some(cache) = self.viewport_document_cache.as_ref() {
            keep.extend(
                cache
                    .key
                    .slots
                    .iter()
                    .filter_map(ViewportSlotKey::working_set_key),
            );
        }

        self.workspace.file_cache.update(&self.store, |files| {
            let mut bytes = files
                .values()
                .map(ActiveFile::working_set_bytes)
                .fold(0usize, usize::saturating_add);
            if files.len() <= COMPARE_WORKING_SET_MAX_FILES
                && bytes <= COMPARE_WORKING_SET_BYTE_BUDGET
            {
                return;
            }

            let mut victims = files
                .iter()
                .filter(|(_, file)| !keep.contains(&file.working_set_key()))
                .map(|(index, file)| (*index, file.last_used_tick))
                .collect::<Vec<_>>();
            victims.sort_by_key(|(_, last_used)| *last_used);

            for (index, _) in victims {
                if files.len() <= COMPARE_WORKING_SET_MAX_FILES
                    && (files.len() <= COMPARE_WORKING_SET_MIN_FILES
                        || bytes <= COMPARE_WORKING_SET_BYTE_BUDGET)
                {
                    break;
                }
                if let Some(file) = files.remove(&index) {
                    bytes = bytes.saturating_sub(file.working_set_bytes());
                }
            }
        });
    }

    fn cached_file_at(&self, index: usize) -> Option<ActiveFile> {
        self.workspace
            .file_cache
            .with(&self.store, |files| files.get(&index).cloned())
    }

    pub(crate) fn viewport_file_snapshot(&self, index: usize) -> Option<ActiveFile> {
        if let Some(active) = self.workspace.active_file.with(&self.store, |file| {
            file.as_ref()
                .filter(|active| active.index == index)
                .cloned()
        }) {
            return Some(active);
        }
        self.cached_file_at(index)
    }

    fn file_load_pending_priority(&self, index: usize, path: &str) -> Option<CompareWorkPriority> {
        self.workspace
            .active_file_loading
            .with(&self.store, |loading| {
                loading
                    .as_ref()
                    .filter(|loading| loading.index == index && loading.path == path)
                    .map(|loading| loading.priority)
            })
            .or_else(|| {
                self.workspace
                    .file_cache_loading
                    .with(&self.store, |loading| {
                        loading
                            .get(&index)
                            .filter(|loading| loading.path == path)
                            .map(|loading| loading.priority)
                    })
            })
    }

    fn should_enqueue_file_load(
        &self,
        index: usize,
        path: &str,
        priority: CompareWorkPriority,
    ) -> bool {
        self.file_load_pending_priority(index, path)
            .is_none_or(|pending| priority.rank() > pending.rank())
    }

    fn mark_file_cache_loading(
        &mut self,
        index: usize,
        path: String,
        priority: CompareWorkPriority,
    ) {
        self.workspace
            .file_cache_loading
            .update(&self.store, |loading| {
                loading.insert(
                    index,
                    ActiveFileLoading {
                        index,
                        path,
                        priority,
                    },
                );
            });
    }

    fn clear_file_cache_loading(&mut self, index: usize) {
        self.workspace
            .file_cache_loading
            .update(&self.store, |loading| {
                loading.remove(&index);
            });
    }

    fn compare_refs(&self) -> (String, String) {
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
        (left_ref, right_ref)
    }

    fn cached_compare_file_at(&self, index: usize, path: &str) -> Option<ActiveFile> {
        let (left_ref, right_ref) = self.compare_refs();
        if let Some(active_file) = self.workspace.active_file.with(&self.store, |file| {
            file.as_ref()
                .filter(|file| {
                    file.index == index
                        && file.path == path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .cloned()
        }) {
            return Some(active_file);
        }
        self.cached_file_at(index).filter(|file| {
            file.index == index
                && file.path == path
                && file.left_ref == left_ref
                && file.right_ref == right_ref
        })
    }

    fn cached_status_file_at(&self, index: usize, change: &FileChange) -> Option<ActiveFile> {
        let (left_ref, right_ref) = self.status_refs_for_bucket(change.bucket);
        if let Some(active_file) = self.workspace.active_file.with(&self.store, |file| {
            file.as_ref()
                .filter(|file| {
                    file.index == index
                        && file.path == change.path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .cloned()
        }) {
            return Some(active_file);
        }
        self.cached_file_at(index).filter(|file| {
            file.index == index
                && file.path == change.path
                && file.left_ref == left_ref
                && file.right_ref == right_ref
        })
    }

    fn status_refs_for_bucket(&self, bucket: ChangeBucket) -> (String, String) {
        self.vcs_ui_profile().status_compare_refs(bucket)
    }

    fn vcs_ui_profile(&self) -> crate::ui::vcs::VcsUiProfile {
        self.repository.location.with(&self.store, |location| {
            crate::ui::vcs::profile(location.as_ref())
        })
    }

    fn active_file_slot_key(
        &self,
        source: WorkspaceSource,
        active: &ActiveFile,
    ) -> ViewportSlotKey {
        let kind = if active.carbon_file.is_binary {
            ViewportSlotKind::Binary
        } else {
            ViewportSlotKind::Text {
                line_count: active.render_doc.lines.len(),
                text_len: active.render_doc.text_bytes.len(),
                style_run_count: active.render_doc.style_runs.len(),
                syntax_covered_count: active.syntax_covered.len(),
            }
        };
        ViewportSlotKey {
            source,
            index: active.index,
            path: active.path.clone(),
            left_ref: active.left_ref.clone(),
            right_ref: active.right_ref.clone(),
            kind,
        }
    }

    fn loading_slot_key(
        &self,
        source: WorkspaceSource,
        index: usize,
        path: &str,
        left_ref: String,
        right_ref: String,
    ) -> ViewportSlotKey {
        ViewportSlotKey {
            source,
            index,
            path: path.to_owned(),
            left_ref,
            right_ref,
            kind: ViewportSlotKind::Loading,
        }
    }

    fn compare_slot_key_at(&self, index: usize, path: &str) -> ViewportSlotKey {
        let source = match self.workspace.source.get(&self.store) {
            WorkspaceSource::TextCompare => WorkspaceSource::TextCompare,
            _ => WorkspaceSource::Compare,
        };
        let (left_ref, right_ref) = self.compare_refs();
        if let Some(key) = self.workspace.active_file.with(&self.store, |file| {
            file.as_ref()
                .filter(|file| {
                    file.index == index
                        && file.path == path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .map(|file| self.active_file_slot_key(source, file))
        }) {
            return key;
        }
        if let Some(key) = self.workspace.file_cache.with(&self.store, |files| {
            files
                .get(&index)
                .filter(|file| {
                    file.index == index
                        && file.path == path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .map(|file| self.active_file_slot_key(source, file))
        }) {
            return key;
        }
        self.loading_slot_key(source, index, path, left_ref, right_ref)
    }

    fn status_slot_key_at(&self, index: usize, change: &FileChange) -> ViewportSlotKey {
        let (left_ref, right_ref) = self.status_refs_for_bucket(change.bucket);
        if let Some(key) = self.workspace.active_file.with(&self.store, |file| {
            file.as_ref()
                .filter(|file| {
                    file.index == index
                        && file.path == change.path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .map(|file| self.active_file_slot_key(WorkspaceSource::Status, file))
        }) {
            return key;
        }
        if let Some(key) = self.workspace.file_cache.with(&self.store, |files| {
            files
                .get(&index)
                .filter(|file| {
                    file.index == index
                        && file.path == change.path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .map(|file| self.active_file_slot_key(WorkspaceSource::Status, file))
        }) {
            return key;
        }
        self.loading_slot_key(
            WorkspaceSource::Status,
            index,
            &change.path,
            left_ref,
            right_ref,
        )
    }

    fn append_viewport_slot_doc(
        &self,
        out: &mut RenderDoc,
        key: &ViewportSlotKey,
        loading_message: &str,
    ) {
        if let ViewportSlotKind::Loading = key.kind {
            out.append_doc(&build_placeholder_render_doc(&key.path, loading_message));
            return;
        }

        let mut appended = false;
        self.workspace.active_file.with(&self.store, |file| {
            let Some(active) = file.as_ref() else {
                return;
            };
            if active.index == key.index
                && active.path == key.path
                && active.left_ref == key.left_ref
                && active.right_ref == key.right_ref
            {
                append_active_file_doc(out, active);
                appended = true;
            }
        });
        if appended {
            return;
        }

        self.workspace.file_cache.with(&self.store, |files| {
            let Some(active) = files.get(&key.index).filter(|active| {
                active.index == key.index
                    && active.path == key.path
                    && active.left_ref == key.left_ref
                    && active.right_ref == key.right_ref
            }) else {
                return;
            };
            append_active_file_doc(out, active);
            appended = true;
        });

        if !appended {
            out.append_doc(&build_placeholder_render_doc(&key.path, loading_message));
        }
    }

    fn viewport_slot_syntax_window(
        &self,
        key: &ViewportSlotKey,
        slot_top_px: u32,
        slot_height_px: u32,
        viewport_top_px: u32,
        viewport_height_px: u32,
    ) -> Option<SyntaxRowWindow> {
        let ViewportSlotKind::Text { line_count, .. } = key.kind else {
            return None;
        };
        if line_count == 0 {
            return None;
        }

        let slot_bottom_px = slot_top_px.saturating_add(slot_height_px.max(1));
        let viewport_bottom_px = viewport_top_px.saturating_add(viewport_height_px.max(1));
        let visible_top_px = slot_top_px.max(viewport_top_px);
        let visible_bottom_px = slot_bottom_px.min(viewport_bottom_px);
        if visible_bottom_px <= visible_top_px {
            return None;
        }

        let row_height_q16 = self.workspace.measured_px_per_row_q16.get(&self.store);
        let row_height_q16 = if row_height_q16 == 0 {
            24_u32 << 16
        } else {
            row_height_q16
        };
        let row_height_q16 = u64::from(row_height_q16.max(1));
        let start_px = visible_top_px.saturating_sub(slot_top_px);
        let end_px = visible_bottom_px.saturating_sub(slot_top_px);
        let row_floor = |px: u32| ((u64::from(px) << 16) / row_height_q16) as usize;
        let row_ceil = |px: u32| {
            (((u64::from(px) << 16).saturating_add(row_height_q16 - 1)) / row_height_q16) as usize
        };

        let start = row_floor(start_px)
            .saturating_sub(SYNTAX_OVERSCAN_ROWS)
            .min(line_count);
        let mut end = row_ceil(end_px)
            .saturating_add(SYNTAX_OVERSCAN_ROWS)
            .min(line_count);
        if end <= start {
            end = start.saturating_add(SYNTAX_INITIAL_ROWS).min(line_count);
        }
        Some(SyntaxRowWindow { start, end })
    }

    fn request_viewport_slot_syntax_window(
        &mut self,
        key: &ViewportSlotKey,
        window: SyntaxRowWindow,
    ) -> Option<Effect> {
        if window.end <= window.start {
            return None;
        }
        if !self.syntax_request_budget_available() {
            return None;
        }
        let repo_path = self.compare.repo_path.get(&self.store)?;
        let generation = self.active_syntax_generation();
        let syntax_epoch = self.syntax_requests.epoch();
        let mut request = None;
        let request_id = self.syntax_requests.next_request_id();
        let mut matched_active = false;
        let mut active_to_cache = None;

        self.workspace.active_file.update(&self.store, |slot| {
            let Some(active) = slot.as_mut() else {
                return;
            };
            if active.index != key.index
                || active.path != key.path
                || active.left_ref != key.left_ref
                || active.right_ref != key.right_ref
            {
                return;
            }
            matched_active = true;
            if let Some(next_request) = request_syntax_for_active_file(
                active,
                repo_path.clone(),
                generation,
                syntax_epoch,
                window,
                request_id,
            ) {
                active_to_cache = Some(active.clone());
                request = Some(next_request);
            }
        });
        if let Some(active_file) = active_to_cache {
            self.cache_active_file(active_file);
        }
        if matched_active {
            if let Some(request) = request {
                self.track_syntax_request(&request);
                return Some(
                    SyntaxEffect::LoadFileSyntax(Task {
                        generation,
                        request,
                    })
                    .into(),
                );
            }
            return None;
        }

        let request_id = self.syntax_requests.next_request_id();
        self.workspace.file_cache.update(&self.store, |files| {
            let Some(active) = files.get_mut(&key.index).filter(|active| {
                active.index == key.index
                    && active.path == key.path
                    && active.left_ref == key.left_ref
                    && active.right_ref == key.right_ref
            }) else {
                return;
            };
            request = request_syntax_for_active_file(
                active,
                repo_path,
                generation,
                syntax_epoch,
                window,
                request_id,
            );
        });

        request.map(|request| {
            self.track_syntax_request(&request);
            SyntaxEffect::LoadFileSyntax(Task {
                generation,
                request,
            })
            .into()
        })
    }

    fn cache_compare_file_from_output(&mut self, index: usize, path: &str) -> Option<ActiveFile> {
        let carbon_file = self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .and_then(|output| output.carbon.files.get(index))
                .filter(|file| file.path() == path)
                .filter(|file| !(file.is_partial && file.hunks.is_empty()))
                .cloned()
        })?;
        let prepared = prepare_active_file(index, &carbon_file);
        let (left_ref, right_ref) = self.compare_refs();
        let active_file =
            self.build_active_file(index, path.to_owned(), prepared, left_ref, right_ref);
        let active_file = self.cache_active_file(active_file);
        Some(active_file)
    }

    fn ensure_compare_file_cached_for_viewport(
        &mut self,
        index: usize,
        path: &str,
        priority: CompareWorkPriority,
    ) -> Vec<Effect> {
        if self.cached_compare_file_at(index, path).is_some() {
            return Vec::new();
        }
        if self.workspace.source.get(&self.store) == WorkspaceSource::TextCompare {
            if self.cache_compare_file_from_output(index, path).is_some() {
                return vec![
                    SyntaxEffect::EnsureSyntaxPackForPath {
                        path: path.to_owned(),
                    }
                    .into(),
                ];
            }
            return Vec::new();
        }
        if !self.compare_file_is_large(index)
            && self.cache_compare_file_from_output(index, path).is_some()
        {
            return vec![
                SyntaxEffect::EnsureSyntaxPackForPath {
                    path: path.to_owned(),
                }
                .into(),
            ];
        }
        if !self.should_enqueue_file_load(index, path, priority) {
            return Vec::new();
        }

        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let deferred_file = self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .and_then(|output| compare_output_deferred_summary(output, index))
                .filter(|summary| summary.path() == path)
        });
        self.mark_file_cache_loading(index, path.to_owned(), priority);
        vec![
            SyntaxEffect::EnsureSyntaxPackForPath {
                path: path.to_owned(),
            }
            .into(),
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
                    path: path.to_owned(),
                    index,
                    deferred_file,
                    priority,
                },
            })
            .into(),
        ]
    }

    fn ensure_status_file_cached_for_viewport(&mut self, index: usize) -> Vec<Effect> {
        let Some(file_change) = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| changes.get(index).cloned())
        else {
            return Vec::new();
        };
        if self.cached_status_file_at(index, &file_change).is_some() {
            return Vec::new();
        }
        if !self.should_enqueue_file_load(
            index,
            &file_change.path,
            CompareWorkPriority::VisibleViewportDiff,
        ) {
            return Vec::new();
        }

        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        self.mark_file_cache_loading(
            index,
            file_change.path.clone(),
            CompareWorkPriority::VisibleViewportDiff,
        );
        let generation = self.workspace.status_generation.get(&self.store);
        let renderer = self.compare.renderer.get(&self.store);
        vec![
            ensure_syntax_packs_for_file_change_effect(&file_change),
            RepositoryEffect::LoadStatusDiff {
                task: Task {
                    generation,
                    request: StatusDiffRequest {
                        repo_path,
                        file_change,
                        renderer,
                    },
                },
                index,
            }
            .into(),
        ]
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
        let active_file = self.cache_active_file(active_file);
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
        self.workspace.selected_change_bucket.set(&self.store, None);
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
    UiFont,
    MonoFont,
}

pub trait PickerItem {
    fn label(&self) -> &str;
    fn detail(&self) -> Option<&str>;
    fn label_style(&self) -> PickerLabelStyle {
        PickerLabelStyle::Default
    }
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PickerLabelStyle {
    #[default]
    Default,
    JjChangeId {
        prefix_len: usize,
        working_copy: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerEntry {
    pub label: String,
    pub detail: String,
    pub value: String,
    pub highlights: Vec<(usize, usize)>,
    pub label_style: PickerLabelStyle,
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
    fn label_style(&self) -> PickerLabelStyle {
        self.label_style
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
    NewTextCompare,
    OpenGitHubAuthModal,
    OpenGitHubAccountMenu,
    SignOutGitHub,
    FocusFileList,
    FocusViewport,
    ShowWorkingTree,
    RefreshRepository,
    OpenBaseRefPicker,
    OpenHeadRefPicker,
    SwapRefs,
    StartCompare,
    OpenCompareMenu,
    ShowKeyboardShortcuts,
    RestoreCompare,
    ToggleSidebar,
    ToggleFileTree,
    ExpandAllFolders,
    CollapseAllFolders,
    ToggleWrap,
    ToggleContinuousScroll,
    SetSettingsSection(SettingsSection),
    SetThemeMode(ThemeMode),
    SetUiScalePct(u16),
    SetWrapColumn(u32),
    SetWheelScrollLines(u8),
    ToggleAutoUpdate,
    ToggleThemeMode,
    ChangeTheme,
    SetLayout(LayoutMode),
    SetRenderer(RendererKind),
    SetTheme(String),
    ExpandAllContext,
    ClearLineSelection,
    GenerateCommitMessage,
    OpenReviewComment,
    OpenPullRequestInGitHub,
    CheckForUpdates,
    InstallUpdate,
    RestartToUpdate,
    RunOperation(VcsOperation),
    FetchOrigin,
    FetchAllRemotes,
    PushCurrentBranch,
    PublishOptions,
    PushCurrentBranchForceWithLease,
    PullCurrentBranch,
    OpenSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEntryKind {
    Command(PaletteCommand),
    File(usize),
    Commit(String),
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

fn palette_command_available(
    command: &PaletteCommand,
    capabilities: Option<RepoCapabilities>,
) -> bool {
    match command {
        PaletteCommand::FetchOrigin
        | PaletteCommand::FetchAllRemotes
        | PaletteCommand::PushCurrentBranch
        | PaletteCommand::PublishOptions => {
            capabilities.is_some_and(|capabilities| capabilities.remotes)
        }
        PaletteCommand::PushCurrentBranchForceWithLease => {
            capabilities.is_some_and(|capabilities| capabilities.remotes && capabilities.branches)
        }
        PaletteCommand::PullCurrentBranch => {
            capabilities.is_some_and(|capabilities| capabilities.pull_fast_forward)
        }
        _ => true,
    }
}

fn vcs_operation_available_for_location(
    operation: &VcsOperation,
    location: Option<&RepoLocation>,
) -> bool {
    match operation {
        VcsOperation::Jj(_) => location.is_some_and(|location| location.profile == VCS_PROFILE_JJ),
        VcsOperation::JjRebaseCurrentChangeOnto { .. } => {
            location.is_some_and(|location| location.profile == VCS_PROFILE_JJ)
        }
        VcsOperation::JjEditRevision { .. } => {
            location.is_some_and(|location| location.profile == VCS_PROFILE_JJ)
        }
        VcsOperation::JjRestoreOperation { .. } => {
            location.is_some_and(|location| location.profile == VCS_PROFILE_JJ)
        }
    }
}

fn operation_log_entry_detail(entry: &VcsOperationLogEntry) -> String {
    match (
        entry.description.is_empty(),
        entry.user.is_empty(),
        entry.time.is_empty(),
    ) {
        (false, false, false) => format!("{} - {} - {}", entry.description, entry.user, entry.time),
        (false, false, true) => format!("{} - {}", entry.description, entry.user),
        (false, true, false) => format!("{} - {}", entry.description, entry.time),
        (false, true, true) => entry.description.clone(),
        (true, false, false) => format!("{} - {}", entry.user, entry.time),
        (true, false, true) => entry.user.clone(),
        (true, true, false) => entry.time.clone(),
        (true, true, true) => "jj operation log entry".to_owned(),
    }
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
    /// When set, submitting the composer replies to this thread instead of
    /// creating a new inline draft.
    pub reply_target: Option<ReviewThreadId>,
    /// When set, submitting the composer edits this comment (by GraphQL node id)
    /// instead of creating a new draft.
    pub edit_target: Option<String>,
    /// Write (false) vs Preview (true) tab — Preview renders the markdown.
    pub preview: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveReviewStatus {
    pub status: ReviewSessionStatus,
    pub message: Option<String>,
    pub unresolved_threads: usize,
    pub resolved_threads: usize,
    pub outdated_threads: usize,
    pub pending_drafts: usize,
    pub failed_drafts: usize,
    pub review_decision: Option<String>,
    pub viewer_latest_review_state: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct PullRequestState {
    pub status: AsyncStatus,
    pub cache: HashMap<PrKey, PrCacheEntry>,
    pub pending_confirm: Option<PrKey>,
    pub active: Option<PrKey>,
    pub review_comments: HashMap<PrKey, PrReviewCommentsEntry>,
    pub review_sessions: HashMap<PrKey, ReviewSession>,
    pub review_composer: ReviewCommentComposerState,
    /// Ephemeral, UI-only expand/collapse override per thread. Takes precedence
    /// over the default (unresolved=expanded, resolved=collapsed). Not persisted
    /// and intentionally separate from the backend `ReviewThreadStatus.collapsed`.
    pub review_thread_expanded: HashMap<ReviewThreadId, bool>,
    /// Fetched comment-author avatars, keyed by `avatar_cache_key` of the sized
    /// URL. Shared across PRs (avatars are immutable per URL); populated by the
    /// shared `AvatarFetched` handler and read by the review card overlay.
    pub review_avatars: HashMap<u64, ReviewAvatar>,
    /// Active drag-selection within a single review comment body, or `None`.
    /// Mutually exclusive with the editor's viewport text selection.
    pub card_text_selection: Option<CardTextSelection>,
}

/// Drag-selection within one review comment body. Offsets are byte indices into
/// `text` (a snapshot of the cleaned, wrapped-source body), so they remain valid
/// across re-wrap; `text` is stored so copy never has to re-derive it. Only the
/// comment whose `source_key` matches renders the highlight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardTextSelection {
    pub source_key: u64,
    pub text: String,
    pub anchor: usize,
    pub focus: usize,
}

impl CardTextSelection {
    pub fn new(source_key: u64, text: String, byte: usize) -> Self {
        let byte = byte.min(text.len());
        Self {
            source_key,
            text,
            anchor: byte,
            focus: byte,
        }
    }

    pub fn normalized(&self) -> (usize, usize) {
        (self.anchor.min(self.focus), self.anchor.max(self.focus))
    }

    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    /// The selected substring, or `None` when the selection is empty/invalid.
    pub fn selected_text(&self) -> Option<String> {
        let (lo, hi) = self.normalized();
        if lo >= hi {
            return None;
        }
        self.text.get(lo..hi).map(str::to_owned)
    }
}

/// Lifecycle of a single comment-author avatar fetch. `Failed` is terminal (no
/// retry) so a persistently-broken URL falls back to initials without re-fetching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewAvatar {
    Fetching,
    Ready(AvatarBitmap),
    Failed,
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
    Confirmation,
    GitHubAuthModal,
    KeyboardShortcuts,
    ThemePicker,
    FontPicker,
    CompareMenu,
    AccountMenu,
    PublishMenu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayEntry {
    pub surface: OverlaySurface,
    pub focus_return: Option<FocusTarget>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct ConfirmationState {
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub action: Option<Action>,
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
    #[store(flatten)]
    pub confirmation: ConfirmationState,
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

    /// `(pending, failed)` draft counts for the active pull request's review
    /// session, for the submit bar. Zeroes when no PR/session is active.
    pub fn active_review_draft_metrics(&self) -> (usize, usize) {
        let Some(key) = self.active_pull_request_key() else {
            return (0, 0);
        };
        self.github
            .pull_request
            .review_sessions
            .with(&self.store, |sessions| {
                sessions
                    .get(&key)
                    .map(|session| {
                        let metrics = session.metrics();
                        (metrics.pending_drafts, metrics.failed_drafts)
                    })
                    .unwrap_or((0, 0))
            })
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

    pub fn reset_confirmation(&mut self) {
        let d = ConfirmationState::default();
        self.overlays.confirmation.title.set(&self.store, d.title);
        self.overlays
            .confirmation
            .message
            .set(&self.store, d.message);
        self.overlays
            .confirmation
            .confirm_label
            .set(&self.store, d.confirm_label);
        self.overlays.confirmation.action.set(&self.store, d.action);
    }

    pub fn clear_overlays(&mut self) {
        self.overlays
            .stack
            .update(&self.store, |stack| stack.clear());
        self.reset_picker();
        self.reset_command_palette();
        self.reset_confirmation();
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
    pub keyring_enabled: bool,
    pub github_token_store: GitHubTokenStore,
    pub auto_compare_pending: bool,
    pub bootstrap_compare_started: bool,
    pub pending_pr_url: Option<String>,
    pub preferred_file_index: Option<usize>,
    pub preferred_file_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct DebugState {
    pub overlay_visible: bool,
}

const FILE_HEIGHT_SPARSE_MIN_COUNT: usize = 4096;

#[derive(Debug)]
enum FileHeightIndex {
    Empty,
    Dense {
        heights: Vec<u32>,
        tree: Vec<u32>,
    },
    Sparse {
        count: usize,
        default_height: u32,
        total: u64,
        overrides: BTreeMap<usize, u32>,
        tree: Vec<u64>,
    },
}

impl Default for FileHeightIndex {
    fn default() -> Self {
        Self::Empty
    }
}

impl FileHeightIndex {
    fn rebuild(&mut self, heights: Vec<u32>) {
        if heights.is_empty() {
            self.clear();
            return;
        }

        if let Some((default_height, overrides, total)) = sparse_height_index_parts(&heights) {
            let mut tree = vec![0; heights.len() + 1];
            for (index, height) in heights.iter().copied().enumerate() {
                height_tree_add(&mut tree, index, u64::from(height));
            }
            *self = Self::Sparse {
                count: heights.len(),
                default_height,
                total,
                overrides,
                tree,
            };
            return;
        }

        let mut tree = vec![0; heights.len() + 1];
        for (index, height) in heights.iter().copied().enumerate() {
            dense_tree_add(&mut tree, index, height);
        }
        *self = Self::Dense { heights, tree };
    }

    fn clear(&mut self) {
        *self = Self::Empty;
    }

    fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Dense { heights, .. } => heights.len(),
            Self::Sparse { count, .. } => *count,
        }
    }

    fn total_u64(&self) -> u64 {
        match self {
            Self::Empty => 0,
            Self::Dense { heights, .. } => self.prefix_u64(heights.len()),
            Self::Sparse { total, .. } => *total,
        }
    }

    fn total_u32(&self) -> u32 {
        self.total_u64().min(u64::from(u32::MAX)) as u32
    }

    fn prefix_u32(&self, index: usize) -> u32 {
        self.prefix_u64(index).min(u64::from(u32::MAX)) as u32
    }

    fn update(&mut self, index: usize, height: u32) {
        match self {
            Self::Empty => {}
            Self::Dense { heights, tree } => {
                if index >= heights.len() {
                    return;
                }
                let old = heights[index];
                if old == height {
                    return;
                }
                heights[index] = height;
                if height >= old {
                    dense_tree_add(tree, index, height - old);
                } else {
                    dense_tree_sub(tree, index, old - height);
                }
            }
            Self::Sparse {
                count,
                default_height,
                total,
                overrides,
                tree,
            } => {
                if index >= *count {
                    return;
                }
                let old = overrides.get(&index).copied().unwrap_or(*default_height);
                if old == height {
                    return;
                }
                if height == *default_height {
                    overrides.remove(&index);
                } else {
                    overrides.insert(index, height);
                }
                *total = total
                    .saturating_sub(u64::from(old))
                    .saturating_add(u64::from(height));
                if height >= old {
                    height_tree_add(tree, index, u64::from(height - old));
                } else {
                    height_tree_sub(tree, index, u64::from(old - height));
                }
                if overrides.len() > *count / 4 {
                    self.promote_sparse_to_dense();
                }
            }
        }
    }

    fn locate(&self, target_px: u32) -> Option<(usize, u32)> {
        match self {
            Self::Empty => None,
            Self::Dense { heights, tree } => locate_dense_height(heights, tree, target_px),
            Self::Sparse {
                count, total, tree, ..
            } => locate_sparse_height(self, *count, *total, tree, target_px),
        }
    }

    fn prefix_u64(&self, index: usize) -> u64 {
        match self {
            Self::Empty => 0,
            Self::Dense { heights, tree } => dense_prefix_u64(heights, tree, index),
            Self::Sparse { count, tree, .. } => height_tree_prefix_u64(tree, index.min(*count)),
        }
    }

    fn height_at(&self, index: usize) -> u32 {
        match self {
            Self::Empty => 0,
            Self::Dense { heights, .. } => heights.get(index).copied().unwrap_or(0),
            Self::Sparse {
                count,
                default_height,
                overrides,
                ..
            } => {
                if index >= *count {
                    0
                } else {
                    overrides.get(&index).copied().unwrap_or(*default_height)
                }
            }
        }
    }

    fn promote_sparse_to_dense(&mut self) {
        let Self::Sparse {
            count,
            default_height,
            overrides,
            ..
        } = self
        else {
            return;
        };
        let mut heights = vec![*default_height; *count];
        for (index, height) in overrides.iter() {
            if let Some(slot) = heights.get_mut(*index) {
                *slot = *height;
            }
        }
        self.rebuild(heights);
    }
}

#[derive(Debug, Default)]
struct VirtualDiffDocument {
    source: WorkspaceSource,
    generation: u64,
    file_count: usize,
    height_index: FileHeightIndex,
}

impl VirtualDiffDocument {
    fn sync_identity(
        &mut self,
        source: WorkspaceSource,
        generation: u64,
        file_count: usize,
    ) -> bool {
        let changed =
            self.source != source || self.generation != generation || self.file_count != file_count;
        if changed {
            self.source = source;
            self.generation = generation;
            self.file_count = file_count;
            self.height_index.clear();
        }
        changed
    }

    fn clear(&mut self) {
        self.source = WorkspaceSource::None;
        self.generation = 0;
        self.file_count = 0;
        self.height_index.clear();
    }

    fn rebuild_heights(&mut self, heights: Vec<u32>) {
        self.file_count = heights.len();
        self.height_index.rebuild(heights);
    }

    fn item_id(&self, index: usize) -> Option<VirtualDiffItemId> {
        (index < self.file_count)
            .then(|| VirtualDiffItemId::file(self.source, self.generation, index))
    }

    fn anchor_is_current(&self, anchor: ViewportAnchor) -> bool {
        anchor.item_id.source == self.source
            && anchor.item_id.generation == self.generation
            && anchor.item_id.kind == VirtualDiffItemKind::File
            && anchor.item_id.index < self.file_count
    }

    fn len(&self) -> usize {
        self.height_index.len()
    }

    fn total_u32(&self) -> u32 {
        self.height_index.total_u32()
    }

    fn prefix_u32(&self, index: usize) -> u32 {
        self.height_index.prefix_u32(index)
    }

    fn locate(&self, target_px: u32) -> Option<(usize, u32)> {
        self.height_index.locate(target_px)
    }

    fn height_at(&self, index: usize) -> u32 {
        self.height_index.height_at(index)
    }

    fn update_height(&mut self, index: usize, height: u32) {
        self.height_index.update(index, height);
    }
}

#[derive(Debug, Default)]
struct VirtualScrollModel {
    anchor: Option<ViewportAnchor>,
}

impl VirtualScrollModel {
    fn clear(&mut self) {
        self.anchor = None;
    }

    fn set_anchor(&mut self, anchor: ViewportAnchor) {
        self.anchor = Some(anchor);
    }
}

const VIRTUAL_STREAM_SORT_STRIDE: u64 = 1024;
const VIRTUAL_STREAM_ROW_OFFSET: u64 = 512;
const VIRTUAL_STREAM_BLOCK_BELOW_OFFSET: u64 = 768;

fn virtual_row_sort_key(line_index: usize) -> u64 {
    (line_index as u64)
        .saturating_mul(VIRTUAL_STREAM_SORT_STRIDE)
        .saturating_add(VIRTUAL_STREAM_ROW_OFFSET)
}

pub fn virtual_block_below_sort_key(anchor_line_index: u32, block_order: usize) -> u64 {
    u64::from(anchor_line_index)
        .saturating_mul(VIRTUAL_STREAM_SORT_STRIDE)
        .saturating_add(VIRTUAL_STREAM_BLOCK_BELOW_OFFSET)
        .saturating_add(block_order.min(255) as u64)
}

pub fn stable_virtual_key(text: &str) -> u64 {
    let mut key = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.as_bytes() {
        key ^= u64::from(*byte);
        key = key.wrapping_mul(0x100_0000_01b3);
    }
    key
}

fn estimated_virtual_item_height_px(kind: VirtualDiffItemKind) -> u32 {
    match kind {
        VirtualDiffItemKind::File => 192,
        VirtualDiffItemKind::FileHeader => 40,
        VirtualDiffItemKind::Hunk => 28,
        VirtualDiffItemKind::DiffRow => 24,
        VirtualDiffItemKind::ReviewThread => 160,
        VirtualDiffItemKind::ReviewComment => 96,
        VirtualDiffItemKind::Composer => 248,
        VirtualDiffItemKind::LoadingPlaceholder => 48,
    }
}

fn virtual_row_stable_key(line: &RenderLine, local_ordinal: u32) -> u64 {
    let mut key = u64::from(line.kind);
    key = key
        .wrapping_mul(1_099_511_628_211)
        .wrapping_add(line.hunk_index as i64 as u64);
    key = key
        .wrapping_mul(1_099_511_628_211)
        .wrapping_add(u64::from(line.old_line_no));
    key = key
        .wrapping_mul(1_099_511_628_211)
        .wrapping_add(u64::from(line.new_line_no));
    key = key
        .wrapping_mul(1_099_511_628_211)
        .wrapping_add(line.line_index as i64 as u64);
    key.wrapping_mul(1_099_511_628_211)
        .wrapping_add(u64::from(local_ordinal))
}

fn sparse_height_index_parts(heights: &[u32]) -> Option<(u32, BTreeMap<usize, u32>, u64)> {
    if heights.len() < FILE_HEIGHT_SPARSE_MIN_COUNT {
        return None;
    }
    let default_height = most_common_height(heights);
    let mut overrides = BTreeMap::new();
    let mut total = 0_u64;
    for (index, height) in heights.iter().copied().enumerate() {
        total = total.saturating_add(u64::from(height));
        if height != default_height {
            overrides.insert(index, height);
        }
    }

    if overrides.len() <= heights.len() / 4 {
        Some((default_height, overrides, total))
    } else {
        None
    }
}

fn most_common_height(heights: &[u32]) -> u32 {
    let mut counts: HashMap<u32, usize> = HashMap::new();
    let mut best_height = heights[0];
    let mut best_count = 0;
    for height in heights {
        let count = counts
            .entry(*height)
            .and_modify(|count| *count += 1)
            .or_insert(1);
        if *count > best_count {
            best_height = *height;
            best_count = *count;
        }
    }
    best_height
}

fn dense_tree_add(tree: &mut [u32], index: usize, delta: u32) {
    let mut idx = index + 1;
    while idx < tree.len() {
        tree[idx] = tree[idx].saturating_add(delta);
        idx += idx & idx.wrapping_neg();
    }
}

fn dense_tree_sub(tree: &mut [u32], index: usize, delta: u32) {
    let mut idx = index + 1;
    while idx < tree.len() {
        tree[idx] = tree[idx].saturating_sub(delta);
        idx += idx & idx.wrapping_neg();
    }
}

fn height_tree_add(tree: &mut [u64], index: usize, delta: u64) {
    let mut idx = index + 1;
    while idx < tree.len() {
        tree[idx] = tree[idx].saturating_add(delta);
        idx += idx & idx.wrapping_neg();
    }
}

fn height_tree_sub(tree: &mut [u64], index: usize, delta: u64) {
    let mut idx = index + 1;
    while idx < tree.len() {
        tree[idx] = tree[idx].saturating_sub(delta);
        idx += idx & idx.wrapping_neg();
    }
}

fn dense_prefix_u64(heights: &[u32], tree: &[u32], index: usize) -> u64 {
    let mut idx = index.min(heights.len());
    let mut sum = 0_u64;
    while idx > 0 {
        sum = sum.saturating_add(u64::from(tree[idx]));
        idx &= idx - 1;
    }
    sum
}

fn height_tree_prefix_u64(tree: &[u64], index: usize) -> u64 {
    let mut idx = index.min(tree.len().saturating_sub(1));
    let mut sum = 0_u64;
    while idx > 0 {
        sum = sum.saturating_add(tree[idx]);
        idx &= idx - 1;
    }
    sum
}

fn locate_dense_height(heights: &[u32], tree: &[u32], target_px: u32) -> Option<(usize, u32)> {
    if heights.is_empty() {
        return None;
    }
    let target = u64::from(target_px);
    let total = dense_prefix_u64(heights, tree, heights.len());
    if target >= total {
        let index = heights.len() - 1;
        return Some((index, heights[index].saturating_sub(1)));
    }

    let mut idx = 0_usize;
    let mut bit = 1_usize;
    while bit < tree.len() {
        bit <<= 1;
    }
    let mut sum = 0_u64;
    while bit > 0 {
        let next = idx + bit;
        if next < tree.len() {
            let next_sum = sum.saturating_add(u64::from(tree[next]));
            if next_sum <= target {
                idx = next;
                sum = next_sum;
            }
        }
        bit >>= 1;
    }
    let index = idx.min(heights.len().saturating_sub(1));
    Some((
        index,
        target.saturating_sub(sum).min(u64::from(u32::MAX)) as u32,
    ))
}

fn locate_sparse_height(
    index: &FileHeightIndex,
    count: usize,
    total: u64,
    tree: &[u64],
    target_px: u32,
) -> Option<(usize, u32)> {
    if count == 0 {
        return None;
    }
    let target = u64::from(target_px);
    if target >= total {
        let slot = count - 1;
        return Some((slot, index.height_at(slot).saturating_sub(1)));
    }

    let mut slot = 0_usize;
    let mut bit = 1_usize;
    while bit < tree.len() {
        bit <<= 1;
    }
    let mut sum = 0_u64;
    while bit > 0 {
        let next = slot + bit;
        if next < tree.len() {
            let next_sum = sum.saturating_add(tree[next]);
            if next_sum <= target {
                slot = next;
                sum = next_sum;
            }
        }
        bit >>= 1;
    }
    let slot = slot.min(count.saturating_sub(1));
    Some((
        slot,
        target.saturating_sub(sum).min(u64::from(u32::MAX)) as u32,
    ))
}

#[derive(Debug)]
pub struct AppState {
    pub workspace_mode: Signal<WorkspaceMode>,
    pub compare_progress: Signal<Option<CompareProgress>>,
    pub app_view: Signal<AppView>,
    pub settings_section: Signal<SettingsSection>,
    pub keymap_capture: Signal<Option<crate::input::ShortcutCommand>>,
    pub keymaps_scroll_top_px: Signal<f32>,
    pub keymaps_viewport_height_px: Signal<f32>,
    pub keymaps_content_height_px: Signal<f32>,
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
    pub context_menu: ContextMenuState,
    /// Memoized: `true` when `focus` targets a text-editing field.
    pub text_focused: Signal<bool>,
    pub animation: crate::ui::animation::AnimationState,
    pub commit_editor: Editor,
    pub review_comment_editor: Editor,
    pub steering_prompt_editor: Editor,
    pub text_compare: TextCompareState,
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
    viewport_document_cache: Option<ViewportDocumentCache>,
    virtual_diff_document: VirtualDiffDocument,
    virtual_scroll: VirtualScrollModel,
    file_working_set: FileWorkingSet,
    syntax_requests: SyntaxRequestTracker,
    last_virtual_scroll_top_px: Option<u32>,
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
        let keymap_capture = store.create(None::<crate::input::ShortcutCommand>);
        let keymaps_scroll_top_px = store.create(0.0_f32);
        let keymaps_viewport_height_px = store.create(0.0_f32);
        let keymaps_content_height_px = store.create(0.0_f32);
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
            keymap_capture,
            keymaps_scroll_top_px,
            keymaps_viewport_height_px,
            keymaps_content_height_px,
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
            context_menu: ContextMenuState::default(),
            text_focused,
            animation: crate::ui::animation::AnimationState::default(),
            commit_editor: Editor::default(),
            review_comment_editor: Editor::default(),
            steering_prompt_editor: Editor::default(),
            text_compare: TextCompareState::default(),
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
            viewport_document_cache: None,
            virtual_diff_document: VirtualDiffDocument::default(),
            virtual_scroll: VirtualScrollModel::default(),
            file_working_set: FileWorkingSet::default(),
            syntax_requests: SyntaxRequestTracker::default(),
            last_virtual_scroll_top_px: None,
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
        let bootstrap_compare_started = repo_path.is_some()
            && startup.args.open_pr.is_none()
            && auto_compare_pending
            && (startup.args.left.is_some()
                || startup.args.right.is_some()
                || startup.args.compare_mode.is_some());

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
        let keymap_capture = store.create(None::<crate::input::ShortcutCommand>);
        let keymaps_scroll_top_px = store.create(0.0_f32);
        let keymaps_viewport_height_px = store.create(0.0_f32);
        let keymaps_content_height_px = store.create(0.0_f32);
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
            keymap_capture,
            keymaps_scroll_top_px,
            keymaps_viewport_height_px,
            keymaps_content_height_px,
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
                keyring_enabled: startup.keyring_enabled,
                github_token_store: startup.github_token_store,
                auto_compare_pending: auto_compare_pending && !bootstrap_compare_started,
                bootstrap_compare_started,
                pending_pr_url: startup.args.open_pr.clone(),
                preferred_file_index: startup.args.file_index,
                preferred_file_path: startup.args.file_path.clone(),
            },
            last_error,
            toasts,
            syntax_pack_installs,
            update,
            context_menu: ContextMenuState::default(),
            text_focused,
            animation: crate::ui::animation::AnimationState::default(),
            commit_editor: Editor::default(),
            review_comment_editor: Editor::default(),
            steering_prompt_editor: Editor::default(),
            text_compare: TextCompareState::default(),
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
            viewport_document_cache: None,
            virtual_diff_document: VirtualDiffDocument::default(),
            virtual_scroll: VirtualScrollModel::default(),
            file_working_set: FileWorkingSet::default(),
            syntax_requests: SyntaxRequestTracker::default(),
            last_virtual_scroll_top_px: None,
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
            effects.push(state.invalidate_syntax_epoch_effect());
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
                    subject: if bootstrap_compare_started {
                        LoadingSubject::Compare {
                            left_label: state.vcs_ui_profile().compare_ref_display_label(
                                &state.compare.left_ref.get(&state.store),
                            ),
                            right_label: state.vcs_ui_profile().compare_ref_display_label(
                                &state.compare.right_ref.get(&state.store),
                            ),
                        }
                    } else {
                        LoadingSubject::RepoOpen { name: repo_name }
                    },
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
                    reporter_generation: (!bootstrap_compare_started).then_some(boot_gen),
                }
                .into(),
            );
            effects.push(RepositoryEffect::WatchRepository { path: Some(path) }.into());
            if bootstrap_compare_started {
                effects.push(
                    CompareEffect::Run(Task {
                        generation: boot_gen,
                        request: CompareRequest {
                            repo_path: state.compare.repo_path.get(&state.store).unwrap(),
                            request: vcs_compare_request(
                                state.compare.mode.get(&state.store),
                                state.compare.left_ref.get(&state.store),
                                state.compare.right_ref.get(&state.store),
                                state.compare.layout.get(&state.store),
                                state.compare.renderer.get(&state.store),
                            ),
                            github_token: startup.github_token.clone(),
                        },
                    })
                    .into(),
                );
            }
        }
        if let Some(token) = startup.github_token.clone() {
            state.github_access_token = Some(token.clone());
            state.github.auth.token_present.set(&state.store, true);
            if startup.github_token_store.is_enabled() {
                effects.push(GitHubEffect::SaveGitHubToken(token).into());
            }
        } else if startup.github_token_store.is_enabled() {
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
        if startup.keyring_enabled {
            effects.push(AiEffect::LoadAiKeys.into());
        }
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
            Action::TextCompare(action) => text_compare::reduce_action(self, action),
            Action::Compare(action) => compare::reduce_action(self, action),
            Action::Repository(action) => repository::reduce_action(self, action),
            Action::FileList(action) => file_list::reduce_action(self, action),
            Action::Overlay(action) => overlay::reduce_action(self, action),
            Action::Editor(action) => editor::reduce_action(self, action),
            Action::TextEdit(action) => text_edit::reduce_action(self, action),
            Action::Settings(action) => settings::reduce_action(self, action),
            Action::GitHub(action) => github::reduce_action(self, action),
            Action::Update(action) => update::reduce_action(self, action),
            Action::Window(_) => Vec::new(),
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
        let workspace_mode = if self.compare_progress.with(&self.store, |p| p.is_some()) {
            "loading"
        } else {
            workspace_mode_name(self.workspace_mode.get(&self.store))
        };
        let title_prefix = crate::platform::startup::window_title_prefix();
        if self.workspace.source.get(&self.store) == WorkspaceSource::TextCompare {
            return format!("{title_prefix} - Text Compare [{workspace_mode}]");
        }
        let repo = self.compare.repo_path.with(&self.store, |p| {
            p.as_deref()
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .unwrap_or("native")
                .to_owned()
        });
        let selected_path = self.workspace.selected_file_path.get(&self.store);
        if let Some(path) = selected_path.as_deref() {
            format!("{title_prefix} - {repo} [{workspace_mode}] {path}")
        } else {
            format!("{title_prefix} - {repo} [{workspace_mode}]")
        }
    }

    pub fn update_time(&mut self, now_ms: u64) {
        self.clock_ms = now_ms;
        self.animation.tick(now_ms);
        let has_expired_toast = self.toasts.with(&self.store, |toasts| {
            toasts.iter().any(|toast| {
                !toast.hovered
                    && toast.progress.is_none()
                    && now_ms.saturating_sub(toast.created_at_ms) >= TOAST_LIFETIME_MS
            })
        });
        if has_expired_toast {
            self.toasts.update(&self.store, |toasts| {
                toasts.retain(|toast| {
                    toast.hovered
                        || toast.progress.is_some()
                        || now_ms.saturating_sub(toast.created_at_ms) < TOAST_LIFETIME_MS
                });
            });
        }
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
        let path = normalize_repository_open_path(path);
        self.workspace_mode.set(&self.store, WorkspaceMode::Loading);
        self.compare.repo_path.set(&self.store, Some(path.clone()));
        self.compare.left_ref.set(&self.store, String::new());
        self.compare.right_ref.set(&self.store, String::new());
        self.compare.mode.set(&self.store, CompareMode::default());
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        self.repository
            .status
            .set(&self.store, AsyncStatus::Loading);
        self.repository.location.set(&self.store, None);
        self.repository.capabilities.set(&self.store, None);
        self.repository.refs.set(&self.store, Vec::new());
        self.repository.changes.set(&self.store, Vec::new());
        self.repository.operation_log.set(&self.store, Vec::new());
        self.repository.file_changes.set(&self.store, Vec::new());
        self.repository.publish_plan.set(&self.store, None);
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
        let syntax_epoch_effect = self.invalidate_syntax_epoch_effect();
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
            syntax_epoch_effect,
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
        self.clear_syntax_inflight();
        self.workspace.files.set(&self.store, Vec::new());
        self.workspace
            .status_file_changes
            .set(&self.store, Vec::new());
        self.workspace.selected_file_index.set(&self.store, None);
        self.workspace.selected_file_path.set(&self.store, None);
        self.workspace.selected_change_bucket.set(&self.store, None);
        self.workspace.compare_output.set(&self.store, None);
        self.workspace.compare_total_stats.set(&self.store, None);
        self.workspace.compare_hydrated_stats.set(&self.store, None);
        self.workspace
            .compare_deferred_stats_remaining
            .set(&self.store, None);
        self.workspace
            .compare_deferred_stats_cursor
            .set(&self.store, 0);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.set_compare_stats_hydration(CompareStatsHydrationState::Idle);
        self.workspace.active_file.set(&self.store, None);
        self.workspace.active_file_loading.set(&self.store, None);
        self.clear_file_cache();
        self.workspace.raw_diff_len.set(&self.store, 0);
        self.workspace.used_fallback.set(&self.store, false);
        self.workspace
            .fallback_message
            .set(&self.store, String::new());
        self.workspace.sidebar_auto_width.set(&self.store, None);
        self.workspace.range_commits.set(&self.store, Vec::new());
        self.workspace
            .compare_history_pending
            .set(&self.store, None);
        self.workspace.pre_drill_compare.set(&self.store, None);
        self.workspace.expansions.update(&self.store, |m| m.clear());
        self.clear_file_scroll_layout();
        self.workspace.global_scroll_top_px.set(&self.store, 0);
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
        self.repository
            .location
            .set(&self.store, Some(payload.location.clone()));
        self.repository
            .capabilities
            .set(&self.store, Some(payload.capabilities));
        let file_changes = payload.file_changes;
        self.repository.refs.set(&self.store, payload.refs);
        self.repository.changes.set(&self.store, payload.changes);
        self.repository
            .operation_log
            .set(&self.store, payload.operation_log);
        self.repository
            .file_changes
            .set(&self.store, file_changes.clone());
        self.repository.publish_plan.set(&self.store, None);
        self.workspace
            .status_file_changes
            .set(&self.store, file_changes);

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
                    self.startup.auto_compare_pending = false;
                    self.github
                        .pull_request
                        .status
                        .set(&self.store, AsyncStatus::Loading);
                    if let Some(parsed) = crate::core::forge::github::parse_pr_url(&url) {
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
                } else if self.startup.bootstrap_compare_started {
                    self.startup.bootstrap_compare_started = false;
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
                    let profile = crate::ui::vcs::profile(Some(&payload.location));
                    let (left, right, mode) = profile.default_compare();
                    self.compare.left_ref.set(&self.store, left.to_owned());
                    self.compare.right_ref.set(&self.store, right.to_owned());
                    self.compare.mode.set(&self.store, mode);
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
                    Some(RepositoryChangeKind::Metadata | RepositoryChangeKind::Both) => {
                        self.kickoff_compare()
                    }
                    Some(RepositoryChangeKind::Worktree)
                        if self.vcs_ui_profile().is_working_copy_ref(&right_ref) =>
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
        direction: crate::editor::diff::expansion::ExpandDirection,
        amount: u32,
    ) -> Vec<Effect> {
        use crate::editor::diff::expansion::ExpandDirection;
        use crate::events::ContextDirection;

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

        let generation = self.workspace.compare_generation.get(&self.store);
        let Some((
            file_index,
            path,
            old_reference,
            new_reference,
            cached_old_lines,
            cached_new_lines,
        )) = self.workspace.active_file.with(&self.store, |af| {
            let active = af.as_ref()?;
            if active.carbon_file.hunks.is_empty() {
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
                active.old_file_lines.clone(),
                active.file_lines.clone(),
            ))
        })
        else {
            return Vec::new();
        };

        if let (Some(old_lines), Some(new_lines)) = (cached_old_lines, cached_new_lines) {
            self.apply_context_expansion(direction, hunk_index, amount, old_lines, new_lines);
            let mut effects = vec![self.invalidate_syntax_epoch_effect()];
            effects.extend(self.request_active_file_syntax_effect());
            return effects;
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
        let mut effects = vec![self.invalidate_syntax_epoch_effect()];
        effects.extend(self.request_active_file_syntax_effect());
        effects
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
            mut carbon_overlays,
            mut token_buffer,
        )) = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref().map(|a| {
                (
                    a.index,
                    a.path.clone(),
                    (*a.carbon_file).clone(),
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

        let preserve_change_tokens = carbon_overlays.has_change_tokens();
        carbon_overlays.clear_syntax();
        if !preserve_change_tokens {
            token_buffer.clear();
        }
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
                active.carbon_file = Arc::new(carbon_file);
                active.carbon_expansion = expansion;
                active.carbon_overlays = carbon_overlays;
                active.token_buffer = token_buffer;
                active.render_doc = Arc::new(render_doc);
                active.file_line_count = Some(total_lines);
                active.old_file_lines = Some(old_lines);
                active.file_lines = Some(new_lines);
                active.syntax_pending.clear();
                active.syntax_covered.clear();
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

        let history_left = payload.resolved_left.clone();
        let history_right = self
            .vcs_ui_profile()
            .history_right_ref(&payload.resolved_right);
        self.workspace
            .status_operation_pending
            .set(&self.store, false);
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Compare);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace_mode.set(&self.store, WorkspaceMode::Ready);
        self.compare.layout.set(&self.store, payload.request.layout);
        self.compare
            .renderer
            .set(&self.store, payload.request.renderer);
        self.compare
            .resolved_left
            .set(&self.store, Some(payload.resolved_left));
        self.compare
            .resolved_right
            .set(&self.store, Some(payload.resolved_right));
        self.workspace
            .raw_diff_len
            .set(&self.store, payload.output.raw_diff_len);
        self.workspace
            .used_fallback
            .set(&self.store, payload.output.used_fallback);
        self.workspace
            .fallback_message
            .set(&self.store, payload.output.fallback_message.clone());
        let total_files = payload.output.file_count() as u32;
        let stats_snapshot = compare_output_stats_snapshot(&payload.output);
        let has_deferred_stats = stats_snapshot.deferred_count > 0;
        let eager_total_stats = (!has_deferred_stats).then_some(stats_snapshot.hydrated_total);
        self.workspace
            .compare_output
            .set(&self.store, Some(payload.output));
        self.workspace.files.set(&self.store, Vec::new());
        self.workspace
            .compare_total_stats
            .set(&self.store, eager_total_stats);
        self.workspace.compare_hydrated_stats.set(
            &self.store,
            has_deferred_stats.then_some(stats_snapshot.hydrated_total),
        );
        self.workspace
            .compare_deferred_stats_remaining
            .set(&self.store, Some(stats_snapshot.deferred_count));
        self.workspace
            .compare_deferred_stats_cursor
            .set(&self.store, 0);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.set_compare_stats_hydration(CompareStatsHydrationState::Idle);
        self.workspace.active_file_loading.set(&self.store, None);
        self.workspace.sidebar_auto_width.set(&self.store, None);
        self.clear_file_cache();
        self.reset_file_scroll_layout();
        self.workspace.global_scroll_top_px.set(&self.store, 0);
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

        let file_count = self.workspace_file_count();
        let index_for_path = preferred_path
            .as_deref()
            .and_then(|path| self.workspace_file_index_for_path(path));

        let mut effects = Vec::new();
        let mut selected_syntax_paths = Vec::new();
        let should_load_history = self
            .workspace
            .pre_drill_compare
            .with(&self.store, |p| p.is_none());
        let history_effect = should_load_history
            .then(|| self.compare_history_request(history_left, history_right))
            .flatten()
            .and_then(|request| {
                if has_deferred_stats {
                    self.workspace
                        .compare_history_pending
                        .set(&self.store, Some(request));
                    None
                } else {
                    Some(self.compare_history_effect(request))
                }
            });
        if let Some(index) = index_for_path
            .or(preferred_index.filter(|index| *index < file_count))
            .or_else(|| (file_count > 0).then_some(0))
        {
            if let Some(path) = self.workspace_file_path_at(index) {
                selected_syntax_paths.push(path);
            }
            effects.extend(self.select_file(index, true));
            if let Some(effect) = self.start_compare_stats_hydration_if_idle() {
                effects.push(effect);
            }
            if let Some(effect) = self.start_compare_total_stats_if_needed() {
                effects.push(effect);
            }
        } else {
            self.workspace.selected_file_index.set(&self.store, None);
            self.workspace.selected_file_path.set(&self.store, None);
            self.workspace.selected_change_bucket.set(&self.store, None);
            self.workspace.active_file.set(&self.store, None);
            self.workspace.active_file_loading.set(&self.store, None);
            // No files to select — the compare succeeded but has no diffs.
            // Tear down the progress panel; the "repo ready" hint takes over.
            self.compare_progress.set(&self.store, None);
            self.editor_clear_document();
        }
        if let Some(effect) = self.syntax_pack_warmup_effect_for_compare(&selected_syntax_paths) {
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
            payload_path = %payload.file_change.path,
            payload_bucket = ?payload.file_change.bucket,
            "handle_status_diff_finished: entered"
        );
        if payload.generation != current_gen {
            tracing::debug!(
                "handle_status_diff_finished: generation mismatch, discarding (pending NOT cleared)"
            );
            return Vec::new();
        }
        let matches = self.repository.file_changes.with(&self.store, |changes| {
            match changes.get(payload.index) {
                Some(current) => current == &payload.file_change,
                None => false,
            }
        });
        if !matches {
            let current_change_at_idx = self.repository.file_changes.with(&self.store, |changes| {
                changes
                    .get(payload.index)
                    .map(|change| format!("{}:{:?}", change.path, change.bucket))
                    .unwrap_or_else(|| "<out of range>".to_owned())
            });
            tracing::debug!(
                current_change_at_idx,
                "handle_status_diff_finished: file change mismatch, discarding (pending NOT cleared)"
            );
            return Vec::new();
        }
        let matches_selection = self.workspace.selected_file_index.get(&self.store)
            == Some(payload.index)
            && self
                .workspace
                .selected_file_path
                .get(&self.store)
                .as_deref()
                == Some(payload.file_change.path.as_str())
            && self.workspace.selected_change_bucket.get(&self.store)
                == Some(payload.file_change.bucket);
        let output = payload.output;

        let Some(carbon_file) = output.carbon.files.first() else {
            self.clear_file_cache_loading(payload.index);
            if matches_selection {
                self.workspace.active_file.set(&self.store, None);
                self.workspace.active_file_loading.set(&self.store, None);
                self.editor_clear_document();
            }
            return Vec::new();
        };
        let prepared = prepare_active_file(payload.index, carbon_file);
        let bucket = payload.file_change.bucket;
        let (left_ref, right_ref) = self.status_refs_for_bucket(bucket);
        let active_file = self.build_active_file(
            payload.index,
            payload.file_change.path.clone(),
            prepared,
            left_ref,
            right_ref,
        );
        let active_file = self.cache_active_file(active_file);
        if !matches_selection {
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
        self.workspace
            .used_fallback
            .set(&self.store, output.used_fallback);
        self.workspace
            .fallback_message
            .set(&self.store, output.fallback_message.clone());
        self.workspace
            .raw_diff_len
            .set(&self.store, output.raw_diff_len);
        self.workspace.compare_output.set(&self.store, None);
        self.workspace.compare_total_stats.set(&self.store, None);
        self.workspace.compare_hydrated_stats.set(&self.store, None);
        self.workspace
            .compare_deferred_stats_remaining
            .set(&self.store, None);
        self.workspace
            .compare_deferred_stats_cursor
            .set(&self.store, 0);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.set_compare_stats_hydration(CompareStatsHydrationState::Idle);
        self.workspace.active_file_loading.set(&self.store, None);

        self.workspace
            .selected_file_index
            .set(&self.store, Some(payload.index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(payload.file_change.path.clone()));
        self.workspace
            .selected_change_bucket
            .set(&self.store, Some(bucket));
        // Preserve scroll/hover/positional editor state when refreshing the
        // same file (e.g. after staging a hunk). Only reset when the path
        // changed (navigating to a different file).
        let same_file = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref().is_some_and(|a| {
                a.path == payload.file_change.path
                    && a.left_ref == active_file.left_ref
                    && a.right_ref == active_file.right_ref
            })
        });
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
        let mut effects = self.sync_editor_scroll_from_global();
        effects.extend(self.request_active_file_syntax_effect());
        effects
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
        let matches_cache_loading =
            self.workspace
                .file_cache_loading
                .with(&self.store, |loading| {
                    loading
                        .get(&payload.index)
                        .is_some_and(|loading| loading.path == payload.path)
                });
        if !matches_selected && !matches_cache_loading {
            return Vec::new();
        }

        if matches_selected && matches_loading {
            self.install_compare_active_file(payload.index, payload.path, payload.prepared);
        } else {
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
            let active_file = self.build_active_file(
                payload.index,
                payload.path,
                payload.prepared,
                left_ref,
                right_ref,
            );
            self.cache_active_file(active_file);
        }
        let mut effects = self.sync_editor_scroll_from_global();
        if matches_selected {
            effects.extend(self.request_active_file_syntax_effect());
        }
        if let Some(effect) = self.start_compare_stats_hydration_if_idle() {
            effects.push(effect);
        } else if let Some(effect) = self.start_compare_total_stats_if_needed() {
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
        let mut effects = Vec::new();
        if let Some(effect) = self.start_compare_stats_hydration_if_idle() {
            let is_background_stats = matches!(
                &effect,
                Effect::Compare(CompareEffect::LoadFileStats(task))
                    if task.request.priority == CompareWorkPriority::Warmup
            );
            effects.push(effect);
            if is_background_stats && let Some(effect) = self.take_pending_compare_history_effect()
            {
                effects.push(effect);
            }
        } else if !self.compare_stats_hydration_running()
            && let Some(effect) = self.take_pending_compare_history_effect()
        {
            effects.push(effect);
        }
        effects
    }

    fn handle_compare_file_stats_ready(&mut self, payload: CompareFileStatsReady) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

        self.apply_compare_file_stats(&payload.stats);
        let mut effects = self.sync_editor_scroll_from_global();
        if !payload.request_complete {
            return effects;
        }
        if let Some(effect) = self.next_compare_stats_hydration_effect() {
            effects.push(effect);
            effects
        } else {
            self.set_compare_stats_hydration(CompareStatsHydrationState::Idle);
            let history_effect = self.take_pending_compare_history_effect();
            if let Some(stats) = self.exact_compare_total_stats_if_ready() {
                if !self.workspace.compare_total_stats_loading.get(&self.store) {
                    self.workspace
                        .compare_total_stats
                        .set(&self.store, Some(stats));
                    self.workspace
                        .compare_total_stats_loading
                        .set(&self.store, false);
                }
                if let Some(effect) = history_effect {
                    effects.push(effect);
                }
                return effects;
            }
            if let Some(effect) = self.start_compare_total_stats_if_needed() {
                effects.push(effect);
            }
            if let Some(effect) = history_effect {
                effects.push(effect);
            }
            effects
        }
    }

    fn compare_stats_hydration_running(&self) -> bool {
        self.workspace.compare_stats_hydration.get(&self.store)
            == CompareStatsHydrationState::Running
    }

    fn compare_stats_hydration_failed(&self) -> bool {
        self.workspace.compare_stats_hydration.get(&self.store)
            == CompareStatsHydrationState::Failed
    }

    fn set_compare_stats_hydration(&self, state: CompareStatsHydrationState) {
        self.workspace
            .compare_stats_hydration
            .set(&self.store, state);
    }

    fn start_compare_stats_hydration_if_idle(&mut self) -> Option<Effect> {
        if self.compare_stats_hydration_running() || self.compare_stats_hydration_failed() {
            return None;
        }

        let effect = self.next_compare_stats_hydration_effect()?;
        self.set_compare_stats_hydration(CompareStatsHydrationState::Running);
        Some(effect)
    }

    fn start_visible_compare_stats_hydration(&mut self) -> Option<Effect> {
        if self.compare_stats_hydration_failed() {
            return None;
        }
        let prioritize_visible = self
            .workspace
            .compare_output
            .with(&self.store, |maybe_output| {
                maybe_output.as_ref().is_some_and(|output| {
                    output.file_count() > COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT
                })
            });
        if !prioritize_visible {
            return self.start_compare_stats_hydration_if_idle();
        }
        let visible_files = self.visible_compare_stats_hydration_items();
        if visible_files.is_empty() {
            return self.start_compare_stats_hydration_if_idle();
        }
        let effect = self.compare_file_stats_hydration_effect(
            visible_files,
            CompareWorkPriority::VisibleSidebarStats,
        )?;
        self.set_compare_stats_hydration(CompareStatsHydrationState::Running);
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
        let repo_path = self.compare.repo_path.get(&self.store)?;
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, true);

        Some(
            CompareEffect::LoadStats(Task {
                generation: self.workspace.compare_generation.get(&self.store),
                request: CompareStatsRequest {
                    repo_path,
                    request: vcs_compare_request(
                        self.compare.mode.get(&self.store),
                        self.compare.left_ref.get(&self.store),
                        self.compare.right_ref.get(&self.store),
                        self.compare.layout.get(&self.store),
                        self.compare.renderer.get(&self.store),
                    ),
                    priority: CompareWorkPriority::TotalStats,
                },
            })
            .into(),
        )
    }

    fn next_compare_stats_hydration_effect(&self) -> Option<Effect> {
        let prioritize_visible = self
            .workspace
            .compare_output
            .with(&self.store, |maybe_output| {
                maybe_output.as_ref().is_some_and(|output| {
                    output.file_count() > COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT
                })
            });
        let (files, priority) = if prioritize_visible {
            let visible_files = self.visible_compare_stats_hydration_items();
            if !visible_files.is_empty() {
                (visible_files, CompareWorkPriority::VisibleSidebarStats)
            } else {
                (
                    self.next_deferred_compare_stats_items(COMPARE_STATS_BACKGROUND_CHUNK_SIZE),
                    CompareWorkPriority::Warmup,
                )
            }
        } else {
            (
                self.next_deferred_compare_stats_items(COMPARE_STATS_BACKGROUND_CHUNK_SIZE),
                CompareWorkPriority::Warmup,
            )
        };
        if files.is_empty() {
            return None;
        }

        self.compare_file_stats_hydration_effect(files, priority)
    }

    fn compare_file_stats_hydration_effect(
        &self,
        files: Vec<CompareFileStatsItem>,
        priority: CompareWorkPriority,
    ) -> Option<Effect> {
        if files.is_empty() {
            return None;
        }
        let repo_path = self.compare.repo_path.get(&self.store)?;
        Some(
            CompareEffect::LoadFileStats(Task {
                generation: self.workspace.compare_generation.get(&self.store),
                request: CompareFileStatsRequest {
                    repo_path,
                    request: vcs_compare_request(
                        self.compare.mode.get(&self.store),
                        self.compare.left_ref.get(&self.store),
                        self.compare.right_ref.get(&self.store),
                        self.compare.layout.get(&self.store),
                        self.compare.renderer.get(&self.store),
                    ),
                    files,
                    priority,
                },
            })
            .into(),
        )
    }

    fn compare_history_request(
        &self,
        left_ref: String,
        right_ref: String,
    ) -> Option<CompareHistoryRequest> {
        Some(CompareHistoryRequest {
            repo_path: self.compare.repo_path.get(&self.store)?,
            left_ref,
            right_ref,
        })
    }

    fn compare_history_effect(&self, request: CompareHistoryRequest) -> Effect {
        CompareEffect::LoadHistory(Task {
            generation: self.workspace.compare_generation.get(&self.store),
            request,
        })
        .into()
    }

    fn take_pending_compare_history_effect(&mut self) -> Option<Effect> {
        if self
            .workspace
            .pre_drill_compare
            .with(&self.store, |p| p.is_some())
        {
            self.workspace
                .compare_history_pending
                .set(&self.store, None);
            return None;
        }
        let pending = self.workspace.compare_history_pending.get(&self.store)?;
        self.workspace
            .compare_history_pending
            .set(&self.store, None);
        Some(self.compare_history_effect(pending))
    }

    fn next_deferred_compare_stats_items(&self, limit: usize) -> Vec<CompareFileStatsItem> {
        if limit == 0
            || self
                .workspace
                .compare_deferred_stats_remaining
                .get(&self.store)
                == Some(0)
        {
            return Vec::new();
        }

        let cursor = self
            .workspace
            .compare_deferred_stats_cursor
            .get(&self.store);
        let (items, next_cursor) =
            self.workspace
                .compare_output
                .with(&self.store, |maybe_output| {
                    let Some(output) = maybe_output.as_ref() else {
                        return (Vec::new(), None);
                    };
                    let file_count = output.file_count();
                    if file_count == 0 {
                        return (Vec::new(), None);
                    }
                    let mut items = Vec::new();
                    let mut index = cursor.min(file_count - 1);
                    let mut scanned = 0_usize;
                    while scanned < file_count && items.len() < limit {
                        if let Some(target) = output.deferred_stats_target_at(index) {
                            items.push(CompareFileStatsItem { index, target });
                        }
                        index = if index + 1 == file_count {
                            0
                        } else {
                            index + 1
                        };
                        scanned += 1;
                    }
                    (items, Some(index))
                });
        if let Some(next_cursor) = next_cursor {
            self.workspace
                .compare_deferred_stats_cursor
                .set(&self.store, next_cursor);
        }
        items
    }

    fn visible_compare_stats_hydration_items(&self) -> Vec<CompareFileStatsItem> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Compare
            || self.file_list.tab.get(&self.store) != SidebarTab::Files
        {
            return Vec::new();
        }

        let stride = self.file_list_row_stride();
        if stride <= 0.0 {
            return Vec::new();
        }
        let scroll_px = self.file_list.scroll_offset_px.get(&self.store);
        let viewport_px = self.file_list.viewport_height.get(&self.store);
        let first = (scroll_px / stride).floor().max(0.0) as usize;
        let visible = (viewport_px / stride).ceil().max(1.0) as usize;
        let start = first.saturating_sub(COMPARE_STATS_VISIBLE_OVERSCAN_ROWS);
        let end = first
            .saturating_add(visible)
            .saturating_add(COMPARE_STATS_VISIBLE_OVERSCAN_ROWS);

        let filter = self
            .file_list
            .filter
            .with(&self.store, |filter| filter.clone());
        if !filter.is_empty() {
            let filtered_indices = self.workspace_file_filter_matches(&filter);
            let end = end.min(filtered_indices.len());
            if start >= end {
                return Vec::new();
            }
            return self.compare_stats_hydration_items_for_indices(
                filtered_indices[start..end].iter().copied(),
            );
        }

        if self.file_list.mode.get(&self.store) == SidebarMode::TreeView {
            let expanded_folders = self.file_list.expanded_folders.get(&self.store);
            let tree_indices = crate::ui::components::file_tree_visible_file_indices_by(
                |visit| {
                    self.for_each_workspace_file_path(|index, path| visit(index, path));
                },
                &expanded_folders,
                start..end,
            );
            return self.compare_stats_hydration_items_for_indices(tree_indices);
        }

        let end = end.min(self.workspace_file_count());
        if start >= end {
            return Vec::new();
        }
        self.compare_stats_hydration_items_for_indices(start..end)
    }

    fn compare_stats_hydration_items_for_indices(
        &self,
        indices: impl IntoIterator<Item = usize>,
    ) -> Vec<CompareFileStatsItem> {
        self.workspace
            .compare_output
            .with(&self.store, |maybe_output| {
                let Some(output) = maybe_output.as_ref() else {
                    return Vec::new();
                };
                let mut items = Vec::new();
                for index in indices {
                    if items.len() >= COMPARE_STATS_CHUNK_SIZE {
                        break;
                    }
                    if let Some(target) = output.deferred_stats_target_at(index) {
                        items.push(CompareFileStatsItem { index, target });
                    }
                }
                items
            })
    }

    fn exact_compare_total_stats_if_ready(&self) -> Option<(i32, i32)> {
        if let Some(remaining) = self
            .workspace
            .compare_deferred_stats_remaining
            .get(&self.store)
        {
            if remaining > 0 {
                return None;
            }
            if let Some(total) = self.workspace.compare_hydrated_stats.get(&self.store) {
                return Some(total);
            }
        }

        let ready = self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .is_some_and(|output| !compare_output_has_deferred_stats(output))
        });
        if !ready {
            return None;
        }
        self.workspace.compare_output.with(&self.store, |output| {
            let output = output.as_ref()?;
            let mut total = (0_i32, 0_i32);
            output.for_each_summary(|_, summary| {
                let stats = summary.fallback_stats();
                total = (
                    total.0.saturating_add(stats.0),
                    total.1.saturating_add(stats.1),
                );
            });
            Some(total)
        })
    }

    fn apply_compare_file_stats(&mut self, stats: &[CompareFileStat]) {
        if stats.is_empty() {
            return;
        }

        let old_scroll_heights = stats
            .iter()
            .map(|stat| (stat.index, self.file_scroll_height_px(stat.index)))
            .collect::<Vec<_>>();

        let mut stats_delta = (0_i32, 0_i32);
        let mut newly_hydrated = 0_usize;
        self.workspace
            .compare_output
            .update(&self.store, |maybe_output| {
                let Some(output) = maybe_output.as_mut() else {
                    return;
                };
                for stat in stats {
                    let additions = i32_to_u32_nonnegative(stat.additions);
                    let deletions = i32_to_u32_nonnegative(stat.deletions);

                    if !output.file_summaries.is_empty() {
                        let Some(summary) = output.file_summaries.get_mut(stat.index) else {
                            continue;
                        };
                        if summary.path() != stat.path {
                            continue;
                        }
                        let old_stats = summary.fallback_stats();
                        let was_deferred = summary.stats_deferred;
                        summary.additions = additions;
                        summary.deletions = deletions;
                        summary.stats_deferred = false;
                        stats_delta = (
                            stats_delta
                                .0
                                .saturating_add(stat.additions.saturating_sub(old_stats.0)),
                            stats_delta
                                .1
                                .saturating_add(stat.deletions.saturating_sub(old_stats.1)),
                        );
                        newly_hydrated = newly_hydrated.saturating_add(was_deferred as usize);
                        continue;
                    }

                    if let Some(file) = output.carbon.files.get_mut(stat.index)
                        && file.path() == stat.path
                    {
                        let old_stats = carbon_file_stats(file);
                        let was_deferred = file.stats_deferred;
                        file.additions = additions;
                        file.deletions = deletions;
                        file.stats_deferred = false;
                        stats_delta = (
                            stats_delta
                                .0
                                .saturating_add(stat.additions.saturating_sub(old_stats.0)),
                            stats_delta
                                .1
                                .saturating_add(stat.deletions.saturating_sub(old_stats.1)),
                        );
                        newly_hydrated = newly_hydrated.saturating_add(was_deferred as usize);
                    }
                }
            });

        if stats_delta != (0, 0) {
            self.workspace
                .compare_hydrated_stats
                .update(&self.store, |total| {
                    let current = total.get_or_insert((0, 0));
                    *current = (
                        current.0.saturating_add(stats_delta.0),
                        current.1.saturating_add(stats_delta.1),
                    );
                });
        }
        if newly_hydrated > 0 {
            self.workspace
                .compare_deferred_stats_remaining
                .update(&self.store, |remaining| {
                    if let Some(count) = remaining.as_mut() {
                        *count = count.saturating_sub(newly_hydrated);
                    }
                });
        }

        let mut rebuilt_viewport_doc = false;
        self.workspace.active_file.update(&self.store, |slot| {
            let Some(active) = slot.as_mut() else {
                return;
            };
            for stat in stats {
                if apply_compare_stat_to_active_file(active, stat) {
                    rebuilt_viewport_doc = true;
                    break;
                }
            }
        });
        self.workspace.file_cache.update(&self.store, |files| {
            for active in files.values_mut() {
                for stat in stats {
                    if apply_compare_stat_to_active_file(active, stat) {
                        rebuilt_viewport_doc = true;
                        break;
                    }
                }
            }
        });
        if rebuilt_viewport_doc {
            self.viewport_document_cache = None;
        }

        let dragging_scrollbar = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.is_some());
        if dragging_scrollbar {
            self.workspace
                .file_scroll_recompute_pending
                .set(&self.store, true);
        } else {
            self.update_file_scroll_heights(old_scroll_heights);
            if self.settings.continuous_scroll {
                self.clamp_global_scroll_top_px();
            }
        }
    }

    fn handle_file_syntax_ready(&mut self, payload: FileSyntaxReady) -> Vec<Effect> {
        self.finish_syntax_request(payload.generation, payload.request_id);
        if payload.generation != self.active_syntax_generation() {
            return Vec::new();
        }

        let mut applied_file = None;
        let mut applied_active = false;
        let mut matched_active = false;
        self.workspace.active_file.update(&self.store, |slot| {
            let Some(active) = slot.as_mut() else {
                return;
            };
            if active.index != payload.file_index || active.path != payload.path {
                return;
            }
            matched_active = true;

            if !remove_pending_syntax_window(
                &mut active.syntax_pending,
                payload.request_id,
                payload.window,
            ) {
                return;
            }
            if active
                .syntax_covered
                .iter()
                .any(|covered| covered.contains(payload.window))
            {
                return;
            }
            push_syntax_covered_window(&mut active.syntax_covered, payload.window);
            apply_syntax_tokens_to_file(
                &mut active.carbon_overlays,
                &mut active.token_buffer,
                &payload.tokens,
            );
            active.render_doc = Arc::new(build_render_doc_from_carbon(
                &active.carbon_file,
                active.index,
                &active.carbon_expansion,
                &active.carbon_overlays,
                &active.token_buffer,
            ));
            applied_file = Some(active.clone());
            applied_active = true;
        });
        if matched_active && applied_file.is_none() {
            tracing::debug!(
                file_index = payload.file_index,
                path = %payload.path,
                request_id = payload.request_id,
                "stale active syntax response dropped"
            );
            return Vec::new();
        }

        if applied_file.is_none() {
            self.workspace.file_cache.update(&self.store, |files| {
                let Some(active) = files.get_mut(&payload.file_index) else {
                    return;
                };
                if active.index != payload.file_index || active.path != payload.path {
                    return;
                }

                if !remove_pending_syntax_window(
                    &mut active.syntax_pending,
                    payload.request_id,
                    payload.window,
                ) {
                    return;
                }
                if active
                    .syntax_covered
                    .iter()
                    .any(|covered| covered.contains(payload.window))
                {
                    return;
                }
                push_syntax_covered_window(&mut active.syntax_covered, payload.window);
                apply_syntax_tokens_to_file(
                    &mut active.carbon_overlays,
                    &mut active.token_buffer,
                    &payload.tokens,
                );
                active.render_doc = Arc::new(build_render_doc_from_carbon(
                    &active.carbon_file,
                    active.index,
                    &active.carbon_expansion,
                    &active.carbon_overlays,
                    &active.token_buffer,
                ));
                applied_file = Some(active.clone());
            });
        }

        let Some(active_file) = applied_file else {
            return Vec::new();
        };
        self.cache_active_file(active_file);
        self.viewport_document_cache = None;

        if applied_active {
            self.request_active_file_syntax_effect()
                .into_iter()
                .collect()
        } else {
            Vec::new()
        }
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

    fn syntax_pack_warmup_effect_for_compare(&self, exclude_paths: &[String]) -> Option<Effect> {
        let highlighter = phosphor::Highlighter::new();
        let excluded_languages = exclude_paths
            .iter()
            .filter_map(|path| highlighter.guess_language(Path::new(path)))
            .collect::<HashSet<_>>();
        let active_languages = self.syntax_pack_installs.with(&self.store, |active| {
            active.iter().cloned().collect::<HashSet<_>>()
        });

        self.workspace.compare_output.with(&self.store, |output| {
            let output = output.as_ref()?;
            let mut seen = HashSet::new();
            let mut warmup_paths = Vec::new();
            output.for_each_summary(|_, summary| {
                for path in [summary.paths.old_path(), summary.paths.new_path()]
                    .into_iter()
                    .flatten()
                {
                    let Some(language) = highlighter.guess_language(Path::new(path.as_ref()))
                    else {
                        continue;
                    };
                    if excluded_languages.contains(&language)
                        || active_languages.contains(language.name())
                        || highlighter.is_parser_available(language)
                    {
                        continue;
                    }
                    if seen.insert(language) {
                        warmup_paths.push(path.into_owned());
                    }
                }
            });

            (!warmup_paths.is_empty()).then_some(
                SyntaxEffect::EnsureSyntaxPacksForPaths {
                    paths: warmup_paths,
                }
                .into(),
            )
        })
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

    fn handle_syntax_packs_installed(&mut self, languages: &[String]) -> Vec<Effect> {
        if languages.is_empty() {
            return Vec::new();
        }
        let mut effects = vec![self.invalidate_syntax_epoch_effect()];
        for language in languages {
            effects.extend(self.refresh_active_file_syntax_for_language(language));
            effects.extend(self.request_cached_file_syntax_effects_for_language(language));
        }
        effects
    }

    fn refresh_active_file_syntax_for_language(&mut self, language: &str) -> Vec<Effect> {
        let highlighter = Highlighter::new();
        let mut refreshed = false;
        self.workspace.active_file.update(&self.store, |slot| {
            let Some(active) = slot.as_mut() else {
                return;
            };
            if !active_file_matches_language(active, &highlighter, language) {
                return;
            }
            reset_active_file_syntax(active);
            refreshed = true;
        });
        if !refreshed {
            return Vec::new();
        }
        self.viewport_document_cache = None;
        self.request_active_file_syntax_effect()
            .into_iter()
            .collect()
    }

    fn request_cached_file_syntax_effects_for_language(&mut self, language: &str) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let generation = self.active_syntax_generation();
        let syntax_epoch = self.syntax_requests.epoch();
        let mut remaining_budget =
            MAX_PENDING_SYNTAX_WINDOWS.saturating_sub(self.syntax_outstanding_window_count());
        if remaining_budget == 0 {
            return Vec::new();
        }
        let active_key = self.workspace.active_file.with(&self.store, |active| {
            active.as_ref().map(ActiveFile::working_set_key)
        });
        let highlighter = Highlighter::new();
        let mut requests = Vec::new();
        let mut next_request_id = self.syntax_requests.last_request_id();

        self.workspace.file_cache.update(&self.store, |files| {
            for active in files.values_mut() {
                if remaining_budget == 0 {
                    break;
                }
                if active_key
                    .as_ref()
                    .is_some_and(|key| key == &active.working_set_key())
                {
                    continue;
                }
                if !active_file_matches_language(active, &highlighter, language) {
                    continue;
                }
                let line_count = active.render_doc.lines.len();
                if line_count == 0 {
                    continue;
                }
                reset_active_file_syntax(active);
                let window = SyntaxRowWindow {
                    start: 0,
                    end: line_count.min(SYNTAX_INITIAL_ROWS),
                };
                next_request_id = next_request_id.saturating_add(1);
                if let Some(request) = request_syntax_for_active_file(
                    active,
                    repo_path.clone(),
                    generation,
                    syntax_epoch,
                    window,
                    next_request_id,
                ) {
                    requests.push(request);
                    remaining_budget = remaining_budget.saturating_sub(1);
                }
            }
        });
        self.syntax_requests.set_last_request_id(next_request_id);

        requests
            .into_iter()
            .map(|request| {
                self.track_syntax_request(&request);
                SyntaxEffect::LoadFileSyntax(Task {
                    generation,
                    request,
                })
                .into()
            })
            .collect()
    }

    fn activate_status_view(&mut self, reset_scroll: bool) -> Vec<Effect> {
        tracing::debug!(
            reset_scroll,
            pending = self.workspace.status_operation_pending.get(&self.store),
            status_gen = self.workspace.status_generation.get(&self.store),
            status_file_changes_count = self
                .workspace
                .status_file_changes
                .with(&self.store, |i| i.len()),
            "activate_status_view: entered"
        );
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Status);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace_mode.set(&self.store, WorkspaceMode::Ready);
        self.workspace.compare_output.set(&self.store, None);
        self.workspace.compare_total_stats.set(&self.store, None);
        self.workspace.compare_hydrated_stats.set(&self.store, None);
        self.workspace
            .compare_deferred_stats_remaining
            .set(&self.store, None);
        self.workspace
            .compare_deferred_stats_cursor
            .set(&self.store, 0);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.set_compare_stats_hydration(CompareStatsHydrationState::Idle);
        self.workspace.active_file_loading.set(&self.store, None);
        let new_files = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| build_status_file_entries(changes));
        self.workspace.files.set(&self.store, new_files);
        let next_status_gen = self
            .workspace
            .status_generation
            .get(&self.store)
            .saturating_add(1);
        self.workspace
            .status_generation
            .set(&self.store, next_status_gen);
        let syntax_epoch_effect = self.invalidate_syntax_epoch_effect();
        self.clear_file_cache();
        self.workspace.sidebar_auto_width.set(&self.store, None);
        self.workspace.used_fallback.set(&self.store, false);
        self.workspace
            .fallback_message
            .set(&self.store, String::new());
        self.workspace.raw_diff_len.set(&self.store, 0);
        self.reset_file_scroll_layout();
        if reset_scroll {
            self.file_list.scroll_offset_px.set(&self.store, 0.0);
            self.workspace.global_scroll_top_px.set(&self.store, 0);
        } else if self.settings.continuous_scroll {
            self.clamp_global_scroll_top_px();
        }

        let current_path = self.workspace.selected_file_path.get(&self.store);
        let current_bucket = self.workspace.selected_change_bucket.get(&self.store);
        let (status_syntax_paths, selected_index, selected_syntax_paths) = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| {
                let paths = changes
                    .iter()
                    .flat_map(file_change_syntax_paths)
                    .collect::<Vec<_>>();
                let selected_index =
                    if let Some((path, bucket)) = current_path.clone().zip(current_bucket) {
                        if let Some(idx) = changes
                            .iter()
                            .position(|change| change.path == path && change.bucket == bucket)
                        {
                            Some(idx)
                        } else {
                            None
                        }
                    } else if let Some(path) = current_path.as_deref() {
                        if let Some(idx) = changes.iter().position(|change| change.path == path) {
                            Some(idx)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                    .or_else(|| (!changes.is_empty()).then_some(0));
                let selected_paths = selected_index
                    .and_then(|index| changes.get(index))
                    .map(file_change_syntax_paths)
                    .unwrap_or_default();
                (paths, selected_index, selected_paths)
            });

        tracing::debug!(
            ?selected_index,
            "activate_status_view: resolved selected_index"
        );
        match selected_index {
            Some(index) => {
                let mut effects = self.select_status_item(index, false);
                effects.insert(0, syntax_epoch_effect);
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
                self.workspace.selected_change_bucket.set(&self.store, None);
                self.workspace.active_file.set(&self.store, None);
                self.workspace.active_file_loading.set(&self.store, None);
                self.editor_clear_document();
                vec![syntax_epoch_effect]
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
        let syntax_epoch_effect = self.invalidate_syntax_epoch_effect();
        self.workspace.compare_total_stats.set(&self.store, None);
        self.workspace.compare_hydrated_stats.set(&self.store, None);
        self.workspace
            .compare_deferred_stats_remaining
            .set(&self.store, None);
        self.workspace
            .compare_deferred_stats_cursor
            .set(&self.store, 0);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.set_compare_stats_hydration(CompareStatsHydrationState::Idle);
        self.workspace.expansions.update(&self.store, |m| m.clear());
        self.clear_overlays();
        self.sync_settings_snapshot();

        let started_at_ms = self.clock_ms;
        let reveal_at_ms = started_at_ms;
        let has_prior_state = self.workspace_file_count() > 0
            || self
                .workspace
                .active_file
                .with(&self.store, |af| af.is_some());

        if !has_prior_state {
            self.workspace_mode.set(&self.store, WorkspaceMode::Loading);
            self.workspace.status.set(&self.store, AsyncStatus::Loading);
        }

        let profile = self.vcs_ui_profile();
        let left_label = profile.compare_ref_display_label(&left_ref);
        let right_label = profile.compare_ref_display_label(&right_ref);
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
            syntax_epoch_effect,
            SettingsEffect::SaveSettings(self.settings.clone()).into(),
            CompareEffect::Run(Task {
                generation: next_gen,
                request: CompareRequest {
                    repo_path,
                    request: vcs_compare_request(mode, left_ref, right_ref, layout, renderer),
                    github_token: self.github_access_token.clone(),
                },
            })
            .into(),
        ]
    }

    /// Soft-cancel an in-flight compare. Bumps the generation so any
    /// result that eventually arrives is dropped by the guard, clears the
    /// progress panel, and returns the viewport to the default empty state.
    /// We do not attempt to interrupt backend work mid-flight; stale-result
    /// guards keep late answers from mutating newer state.
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
        let syntax_epoch_effect = self.invalidate_syntax_epoch_effect();
        self.compare_progress.set(&self.store, None);
        self.workspace.active_file_loading.set(&self.store, None);
        // Only revert the workspace mode if kickoff flipped it to Loading
        // (i.e. no prior state was preserved). When the user cancels a
        // re-compare, the old diff is still mounted and should stay visible.
        if self.workspace_mode.get(&self.store) == WorkspaceMode::Loading {
            self.workspace_mode.set(&self.store, WorkspaceMode::Empty);
            self.workspace.status.set(&self.store, AsyncStatus::Idle);
        }
        vec![syntax_epoch_effect]
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
        let (left, right, mode) = self.vcs_ui_profile().working_copy_compare();
        self.compare.left_ref.set(&self.store, left.to_owned());
        self.compare.right_ref.set(&self.store, right.to_owned());
        self.compare.mode.set(&self.store, mode);
        let mut effects = self.persist_settings_effect();
        effects.extend(self.activate_status_view(true));
        effects
    }

    fn preview_pull_request(&mut self) -> Vec<Effect> {
        let profile = self.vcs_ui_profile();
        if !profile.accepts_compare_mode(CompareMode::ThreeDot)
            || self.repository.location.with(&self.store, |location| {
                !location
                    .as_ref()
                    .is_some_and(|location| location.profile == VCS_PROFILE_GIT)
            })
        {
            self.push_error("PR preview is only available for Git repositories.");
            return Vec::new();
        }
        let Some(base_ref) = self.default_pull_request_base_ref() else {
            self.push_error("No default branch found for PR preview.");
            return Vec::new();
        };
        let (_, workdir_ref, _) = profile.working_copy_compare();
        self.workspace.pre_drill_compare.set(&self.store, None);
        self.compare.left_ref.set(&self.store, base_ref);
        self.compare
            .right_ref
            .set(&self.store, workdir_ref.to_owned());
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        self.compare.mode.set(&self.store, CompareMode::ThreeDot);
        let mut effects = self.persist_settings_effect();
        effects.extend(self.kickoff_compare());
        effects
    }

    fn default_pull_request_base_ref(&self) -> Option<String> {
        let refs = self.repository.refs.get(&self.store);
        let active = refs
            .iter()
            .find(|reference| reference.active && reference.kind == RefKind::Branch)
            .map(|reference| reference.name.as_str());
        let branch_ref = |name: &str| {
            refs.iter()
                .find(|reference| {
                    reference.name == name
                        && active != Some(reference.name.as_str())
                        && matches!(reference.kind, RefKind::Branch | RefKind::RemoteBranch)
                })
                .map(|reference| reference.name.clone())
        };
        for name in [
            "origin/main",
            "origin/master",
            "upstream/main",
            "upstream/master",
            "origin/develop",
            "origin/development",
            "main",
            "master",
            "develop",
            "development",
        ] {
            if let Some(reference) = branch_ref(name) {
                return Some(reference);
            }
        }
        for trunk in ["main", "master", "develop", "development"] {
            let suffix = format!("/{trunk}");
            if let Some(reference) = refs
                .iter()
                .find(|reference| {
                    reference.name.ends_with(&suffix)
                        && active != Some(reference.name.as_str())
                        && reference.kind == RefKind::RemoteBranch
                })
                .map(|reference| reference.name.clone())
            {
                return Some(reference);
            }
        }
        None
    }

    fn swap_refs(&mut self) -> Vec<Effect> {
        let left = self.compare.left_ref.get(&self.store);
        let right = self.compare.right_ref.get(&self.store);
        let profile = self.vcs_ui_profile();
        if left.trim().is_empty()
            || right.trim().is_empty()
            || !profile.can_swap_ref(&right)
            || !profile.can_swap_ref(&left)
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
        self.settings.fonts = self.settings.fonts.normalized();
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
                PickerKind::Repository
                | PickerKind::Theme
                | PickerKind::UiFont
                | PickerKind::MonoFont => {
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
            FocusTarget::TextCompareLeft | FocusTarget::TextCompareRight => None,
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
                PickerKind::Repository
                | PickerKind::Theme
                | PickerKind::UiFont
                | PickerKind::MonoFont => {
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
        let profile = self.vcs_ui_profile();
        if !profile.should_auto_select_trunk_mode() {
            return;
        }
        let left = self.compare.left_ref.get(&self.store);
        let right = self.compare.right_ref.get(&self.store);
        if left.is_empty() || right.is_empty() {
            return;
        }
        if left == right && !profile.is_working_copy_ref(&right) {
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
            label_style: PickerLabelStyle::Default,
            icon: None,
            section_header: false,
        };
        let make_header = |label: &str| PickerEntry {
            label: label.to_owned(),
            detail: String::new(),
            value: String::new(),
            highlights: Vec::new(),
            label_style: PickerLabelStyle::Default,
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
                        label_style: PickerLabelStyle::Default,
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

    fn open_font_picker(&mut self, role: FontRole) {
        let scale = self.ui_scale_factor();
        self.overlays.picker.kind.set(
            &self.store,
            match role {
                FontRole::Ui => PickerKind::UiFont,
                FontRole::Mono => PickerKind::MonoFont,
            },
        );
        self.overlays.picker.query.set(&self.store, String::new());
        self.overlays.picker.list.update(&self.store, |l| {
            l.scroll_top_px = 0;
            l.row_height_px = (Sz::ROW * scale).round() as u32;
            l.gap_px = (Sp::XS * scale).round() as u32;
        });
        self.rebuild_font_picker();
        self.reset_text_edit(0);
        self.push_overlay(OverlaySurface::FontPicker, Some(FocusTarget::PickerInput));
    }

    fn rebuild_font_picker(&mut self) {
        let Some(role) = self.font_picker_role() else {
            return;
        };
        let query = self
            .overlays
            .picker
            .query
            .with(&self.store, |q| q.trim().to_owned());
        let selected_family = self.selected_font_family(role);
        let font_entries = crate::fonts::font_family_entries(role);
        let entries: Vec<PickerEntry> = if query.is_empty() {
            font_entries
                .iter()
                .map(|entry| font_picker_entry(entry, &selected_family, Vec::new()))
                .collect()
        } else {
            let search_texts: Vec<String> = font_entries
                .iter()
                .map(|entry| {
                    if entry.label == entry.family {
                        entry.label.clone()
                    } else {
                        format!("{} {}", entry.label, entry.family)
                    }
                })
                .collect();
            let haystack: Vec<&str> = search_texts.iter().map(|s| s.as_str()).collect();
            let config = neo_frizbee::Config {
                max_typos: Some(2),
                sort: false,
                ..Default::default()
            };
            let mut matches = neo_frizbee::match_list_indices(&query, &haystack, &config);
            matches.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
            matches
                .into_iter()
                .map(|m| {
                    let entry = &font_entries[m.index as usize];
                    let highlights = highlight_ranges_for_visible_match(
                        &query,
                        &entry.label,
                        &m.indices,
                        &config,
                    );
                    font_picker_entry(entry, &selected_family, highlights)
                })
                .collect()
        };

        let selected = entries
            .iter()
            .position(|entry| entry.value == selected_family)
            .unwrap_or(0);
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

    fn font_picker_role(&self) -> Option<FontRole> {
        match self.overlays.picker.kind.get(&self.store) {
            PickerKind::UiFont => Some(FontRole::Ui),
            PickerKind::MonoFont => Some(FontRole::Mono),
            _ => None,
        }
    }

    fn selected_font_family(&self, role: FontRole) -> String {
        match role {
            FontRole::Ui => {
                crate::fonts::normalize_font_selection(role, &self.settings.fonts.ui_family)
            }
            FontRole::Mono => {
                crate::fonts::normalize_font_selection(role, &self.settings.fonts.mono_family)
            }
        }
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
            OverlaySurface::RepoPicker | OverlaySurface::RefPicker | OverlaySurface::FontPicker => {
                self.reset_picker();
            }
            OverlaySurface::CommandPalette => {
                self.reset_command_palette();
            }
            OverlaySurface::Confirmation => {
                self.reset_confirmation();
            }
            _ => {}
        }
        self.set_focus(entry.focus_return);
    }

    fn open_confirmation(
        &mut self,
        title: impl Into<String>,
        message: impl Into<String>,
        confirm_label: impl Into<String>,
        action: Action,
    ) {
        self.overlays
            .confirmation
            .title
            .set(&self.store, title.into());
        self.overlays
            .confirmation
            .message
            .set(&self.store, message.into());
        self.overlays
            .confirmation
            .confirm_label
            .set(&self.store, confirm_label.into());
        self.overlays
            .confirmation
            .action
            .set(&self.store, Some(action));
        self.set_focus(None);
        self.push_overlay(OverlaySurface::Confirmation, None);
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
            Some(
                OverlaySurface::RepoPicker | OverlaySurface::RefPicker | OverlaySurface::FontPicker,
            ) => {
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
            Some(
                OverlaySurface::RepoPicker | OverlaySurface::RefPicker | OverlaySurface::FontPicker,
            ) => {
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
            Some(OverlaySurface::FontPicker) => self.confirm_font_picker(),
            Some(OverlaySurface::RepoPicker) => self.confirm_repo_picker(),
            Some(OverlaySurface::RefPicker) => {
                let field = self.overlays.ref_picker.active_field.get(&self.store);
                self.confirm_ref_picker(field)
            }
            Some(OverlaySurface::CommandPalette) => self.confirm_command_palette(),
            Some(OverlaySurface::Confirmation) => {
                let action = self.overlays.confirmation.action.get(&self.store);
                self.pop_overlay();
                if let Some(action) = action {
                    self.apply_action(action)
                } else {
                    Vec::new()
                }
            }
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
                | OverlaySurface::AccountMenu
                | OverlaySurface::PublishMenu,
            ) => Vec::new(),
            None => Vec::new(),
        }
    }

    fn confirm_font_picker(&mut self) -> Vec<Effect> {
        let Some(role) = self.font_picker_role() else {
            return Vec::new();
        };
        let selected = self.overlays.picker.selected_index.get(&self.store);
        let family = self.overlays.picker.entries.with(&self.store, |entries| {
            entries.get(selected).map(|entry| entry.value.clone())
        });
        let Some(family) = family else {
            return Vec::new();
        };
        let family = crate::fonts::normalize_font_selection(role, &family);
        let changed = match role {
            FontRole::Ui => {
                if self.settings.fonts.ui_family == family {
                    false
                } else {
                    self.settings.fonts.ui_family = family;
                    true
                }
            }
            FontRole::Mono => {
                if self.settings.fonts.mono_family == family {
                    false
                } else {
                    self.settings.fonts.mono_family = family;
                    true
                }
            }
        };
        self.pop_overlay();
        if changed {
            self.persist_settings_effect()
        } else {
            Vec::new()
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
                if path.is_dir() && path_looks_like_repository(&path) {
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
            if path.is_dir() && path_looks_like_repository(&path) {
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
                    label_style: PickerLabelStyle::Default,
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
        let profile = self.vcs_ui_profile();
        let mode = if profile.accepts_compare_mode(mode) {
            mode
        } else {
            profile.compare_modes()[0].mode
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
            PaletteEntryKind::Command(command) => {
                match command {
                    PaletteCommand::OpenRepoPicker => {
                        self.open_repo_picker();
                        Vec::new()
                    }
                    PaletteCommand::NewTextCompare => {
                        self.apply_action(crate::actions::WorkspaceAction::NewTextCompare)
                    }
                    PaletteCommand::OpenGitHubAuthModal => {
                        self.push_overlay(
                            OverlaySurface::GitHubAuthModal,
                            Some(FocusTarget::AuthPrimaryAction),
                        );
                        Vec::new()
                    }
                    PaletteCommand::OpenGitHubAccountMenu => {
                        self.apply_action(crate::actions::GitHubAction::OpenAccountMenu)
                    }
                    PaletteCommand::SignOutGitHub => {
                        self.apply_action(crate::actions::GitHubAction::SignOutGitHub)
                    }
                    PaletteCommand::FocusFileList => {
                        self.set_focus(Some(FocusTarget::FileList));
                        Vec::new()
                    }
                    PaletteCommand::FocusViewport => {
                        self.set_focus(Some(FocusTarget::Editor));
                        Vec::new()
                    }
                    PaletteCommand::ShowWorkingTree => {
                        self.apply_action(crate::actions::WorkspaceAction::ShowWorkingTree)
                    }
                    PaletteCommand::RefreshRepository => {
                        self.apply_action(crate::actions::WorkspaceAction::RefreshRepository)
                    }
                    PaletteCommand::OpenBaseRefPicker => self.apply_action(
                        crate::actions::OverlayAction::OpenRefPicker(CompareField::Left),
                    ),
                    PaletteCommand::OpenHeadRefPicker => self.apply_action(
                        crate::actions::OverlayAction::OpenRefPicker(CompareField::Right),
                    ),
                    PaletteCommand::SwapRefs => {
                        self.apply_action(crate::actions::CompareAction::SwapRefs)
                    }
                    PaletteCommand::StartCompare => {
                        self.apply_action(crate::actions::CompareAction::StartCompare)
                    }
                    PaletteCommand::OpenCompareMenu => {
                        self.apply_action(crate::actions::CompareAction::OpenCompareMenu)
                    }
                    PaletteCommand::ShowKeyboardShortcuts => {
                        self.apply_action(crate::actions::SettingsAction::OpenKeymaps)
                    }
                    PaletteCommand::RestoreCompare => {
                        self.apply_action(crate::actions::CompareAction::ClearSidebarCommit)
                    }
                    PaletteCommand::ToggleSidebar => {
                        self.apply_action(crate::actions::FileListAction::ToggleSidebar)
                    }
                    PaletteCommand::ToggleFileTree => {
                        self.apply_action(crate::actions::FileListAction::ToggleSidebarMode)
                    }
                    PaletteCommand::ExpandAllFolders => {
                        self.apply_action(crate::actions::FileListAction::ExpandAllFolders)
                    }
                    PaletteCommand::CollapseAllFolders => {
                        self.apply_action(crate::actions::FileListAction::CollapseAllFolders)
                    }
                    PaletteCommand::ToggleWrap => {
                        self.apply_action(crate::actions::SettingsAction::ToggleWrap)
                    }
                    PaletteCommand::ToggleContinuousScroll => {
                        self.apply_action(crate::actions::SettingsAction::ToggleContinuousScroll)
                    }
                    PaletteCommand::SetSettingsSection(section) => self
                        .apply_action(crate::actions::SettingsAction::SetSettingsSection(section)),
                    PaletteCommand::SetThemeMode(mode) => {
                        self.apply_action(crate::actions::SettingsAction::SetThemeMode(mode))
                    }
                    PaletteCommand::SetUiScalePct(pct) => {
                        self.apply_action(crate::actions::SettingsAction::SetUiScalePct(pct))
                    }
                    PaletteCommand::SetWrapColumn(column) => {
                        self.apply_action(crate::actions::SettingsAction::SetWrapColumn(column))
                    }
                    PaletteCommand::SetWheelScrollLines(lines) => self
                        .apply_action(crate::actions::SettingsAction::SetWheelScrollLines(lines)),
                    PaletteCommand::ToggleAutoUpdate => {
                        self.apply_action(crate::actions::SettingsAction::ToggleAutoUpdate)
                    }
                    PaletteCommand::ToggleThemeMode => {
                        self.apply_action(crate::actions::SettingsAction::ToggleThemeMode)
                    }
                    PaletteCommand::SetLayout(layout) => {
                        self.apply_action(crate::actions::CompareAction::SetLayoutMode(layout))
                    }
                    PaletteCommand::SetRenderer(renderer) => {
                        self.apply_action(crate::actions::CompareAction::SetRenderer(renderer))
                    }
                    PaletteCommand::ChangeTheme => {
                        self.apply_action(crate::actions::SettingsAction::OpenThemePicker)
                    }
                    PaletteCommand::SetTheme(name) => {
                        self.apply_action(crate::actions::SettingsAction::SetThemeName(name))
                    }
                    PaletteCommand::ExpandAllContext => {
                        self.apply_action(crate::actions::EditorAction::ExpandAllContext)
                    }
                    PaletteCommand::ClearLineSelection => {
                        self.apply_action(crate::actions::RepositoryAction::ClearLineSelection)
                    }
                    PaletteCommand::GenerateCommitMessage => {
                        self.apply_action(crate::actions::AiAction::GenerateCommitMessage)
                    }
                    PaletteCommand::OpenReviewComment => {
                        self.apply_action(crate::actions::GitHubAction::OpenReviewCommentComposer)
                    }
                    PaletteCommand::OpenPullRequestInGitHub => {
                        self.apply_action(crate::actions::GitHubAction::OpenPullRequestInBrowser)
                    }
                    PaletteCommand::CheckForUpdates => {
                        self.apply_action(crate::actions::UpdateAction::CheckForUpdates)
                    }
                    PaletteCommand::InstallUpdate => {
                        self.apply_action(crate::actions::UpdateAction::InstallUpdate)
                    }
                    PaletteCommand::RestartToUpdate => {
                        self.apply_action(crate::actions::UpdateAction::RestartToUpdate)
                    }
                    PaletteCommand::RunOperation(operation) => {
                        self.confirm_or_run_vcs_operation(operation)
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
                    PaletteCommand::PublishOptions => {
                        self.apply_action(crate::actions::RepositoryAction::OpenPublishMenu)
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
                }
            }
            PaletteEntryKind::File(index) => self.select_file(index, true),
            PaletteEntryKind::Commit(oid) => {
                self.apply_action(crate::actions::CompareAction::SelectSidebarCommit(oid))
            }
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

    fn confirm_or_run_vcs_operation(&mut self, operation: VcsOperation) -> Vec<Effect> {
        let action = crate::actions::RepositoryAction::RunOperation(operation.clone());
        if let Some(message) = operation.confirmation_message() {
            self.open_confirmation(
                format!("Confirm {}", operation.label()),
                message,
                operation.label(),
                action.into(),
            );
            Vec::new()
        } else {
            self.apply_action(action)
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
                label_style: PickerLabelStyle::Default,
                icon: None,
                section_header: true,
            });
        }

        if query.is_empty() {
            for repo in &unique_repos {
                let display = repo.display().to_string();
                let is_repo = path_looks_like_repository(repo);
                entries.push(PickerEntry {
                    label: repo
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(&display)
                        .to_owned(),
                    detail: display.clone(),
                    value: repo.display().to_string(),
                    highlights: Vec::new(),
                    label_style: PickerLabelStyle::Default,
                    icon: Some(if is_repo {
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
                let is_repo = path_looks_like_repository(repo);
                entries.push(PickerEntry {
                    label,
                    detail: display.clone(),
                    value: repo.display().to_string(),
                    highlights,
                    label_style: PickerLabelStyle::Default,
                    icon: Some(if is_repo {
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

        if path_looks_like_repository(&dir) {
            entries.push(PickerEntry {
                label: "open this directory".to_owned(),
                detail: String::new(),
                value: format!("open:{}", dir.display()),
                icon: Some(lucide::CORNER_UP_LEFT),
                highlights: Vec::new(),
                label_style: PickerLabelStyle::Default,
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
                label_style: PickerLabelStyle::Default,
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
                let is_repo = path_looks_like_repository(&path);
                dirs.push((name, path, is_repo));
            }
        }

        dirs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

        if filter.is_empty() {
            for (name, path, is_repo) in &dirs {
                entries.push(PickerEntry {
                    label: name.clone(),
                    detail: String::new(),
                    value: path.display().to_string(),
                    highlights: Vec::new(),
                    label_style: PickerLabelStyle::Default,
                    icon: Some(if *is_repo {
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
                let (name, path, is_repo) = &dirs[m.index as usize];
                entries.push(PickerEntry {
                    label: name.clone(),
                    detail: String::new(),
                    value: path.display().to_string(),
                    highlights: highlight_ranges_from_match_indices(name, &m.indices),
                    label_style: PickerLabelStyle::Default,
                    icon: Some(if *is_repo {
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
            icon: Option<&'static str>,
            default_highlights: Vec<(usize, usize)>,
            label_style: PickerLabelStyle,
            ordinal: usize,
        }

        let mut all_candidates = Vec::new();
        let mut ordinal = 0_usize;

        let mut push = |search_text: String,
                        label: String,
                        detail: String,
                        value: String,
                        icon: Option<&'static str>,
                        default_highlights: Vec<(usize, usize)>,
                        label_style: PickerLabelStyle| {
            if !seen.insert(value.clone()) {
                return;
            }
            all_candidates.push(RefCandidate {
                search_text,
                label,
                detail,
                value,
                icon,
                default_highlights,
                label_style,
                ordinal,
            });
            ordinal += 1;
        };

        let profile = self.vcs_ui_profile();
        let refs = self.repository.refs.get(&self.store);
        let changes = self.repository.changes.get(&self.store);

        for reference in &refs {
            let value = reference.name.clone();
            let (kind_label, icon) = profile.ref_kind_label_and_icon(reference.kind);
            let mut detail = kind_label.to_owned();
            if reference.active {
                detail.push_str(" \u{2022} current");
            }
            let mut search_text = format!("{} {detail}", reference.name);
            if reference.target.id != reference.name {
                search_text.push(' ');
                search_text.push_str(&reference.target.id);
            }
            if reference.kind == RefKind::WorkingCopy
                && let Some((detail_suffix, search_suffix)) =
                    profile.working_copy_ref_suffix(&changes)
            {
                detail.push_str(&detail_suffix);
                search_text.push_str(&search_suffix);
            }
            push(
                search_text,
                reference.name.clone(),
                detail,
                value,
                icon,
                Vec::new(),
                PickerLabelStyle::Default,
            );
        }

        for change in &changes {
            let entry = profile.change_ref_entry(change);
            let label_style = entry
                .prefix_len
                .map(|prefix_len| PickerLabelStyle::JjChangeId {
                    prefix_len,
                    working_copy: entry.working_copy,
                })
                .unwrap_or_default();
            push(
                entry.search_text,
                entry.label,
                entry.detail,
                entry.value,
                Some(lucide::HASH),
                entry.default_highlights,
                label_style,
            );
        }

        let mut needs_resolve = false;

        if query.is_empty() {
            let entries = all_candidates
                .into_iter()
                .take(10)
                .map(|c| PickerEntry {
                    label: c.label,
                    detail: c.detail,
                    value: c.value,
                    highlights: c.default_highlights,
                    label_style: c.label_style,
                    icon: c.icon,
                    section_header: false,
                })
                .collect();
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
                            label_style: c.label_style,
                            icon: c.icon,
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
            let mut entries = Vec::new();
            entries.extend(scored.into_iter().map(|(_, _, entry)| entry).take(10));
            if !entries.iter().any(|entry| entry.value == query) {
                entries.insert(
                    0,
                    PickerEntry {
                        label: query.to_owned(),
                        detail: "Resolving\u{2026}".to_owned(),
                        value: query.to_owned(),
                        highlights: vec![(0, query.len())],
                        label_style: PickerLabelStyle::Default,
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

        if let Some(parsed) = crate::core::forge::github::parse_pr_url(query) {
            let key: PrKey = (parsed.owner.clone(), parsed.repo.clone(), parsed.number);
            let token = self.github_access_token.clone();
            let repo_path = self.compare.repo_path.get(&self.store);
            let supports_github_prs = repo_path.is_some()
                && self
                    .repository
                    .capabilities
                    .with(&self.store, |capabilities| {
                        capabilities.is_some_and(|capabilities| capabilities.github_pull_requests)
                    });

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
            if supports_github_prs && let Some(repo_path) = repo_path.clone() {
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
                supports_github_prs,
            ));
        }

        struct PaletteCandidate {
            search_text: String,
            label: String,
            detail: String,
            kind: PaletteEntryKind,
        }

        let mut all_candidates = Vec::new();
        let repo_capabilities = self.repository.capabilities.get(&self.store);

        for (label, detail, command) in [
            (
                "Choose Repository".to_owned(),
                "Open repository picker".to_owned(),
                PaletteCommand::OpenRepoPicker,
            ),
            (
                "New Text Compare".to_owned(),
                "Compare arbitrary pasted text".to_owned(),
                PaletteCommand::NewTextCompare,
            ),
            (
                "GitHub Sign In".to_owned(),
                "Start device flow".to_owned(),
                PaletteCommand::OpenGitHubAuthModal,
            ),
            (
                "GitHub Account Menu".to_owned(),
                "Open GitHub account actions".to_owned(),
                PaletteCommand::OpenGitHubAccountMenu,
            ),
            (
                "GitHub Sign Out".to_owned(),
                "Remove the saved GitHub session".to_owned(),
                PaletteCommand::SignOutGitHub,
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
                "Show Working Tree".to_owned(),
                "Return to the repository working tree view".to_owned(),
                PaletteCommand::ShowWorkingTree,
            ),
            (
                "Refresh Repository".to_owned(),
                "Refresh status or rerun the current compare".to_owned(),
                PaletteCommand::RefreshRepository,
            ),
            (
                "Select Base Ref".to_owned(),
                "Open the left-side ref picker".to_owned(),
                PaletteCommand::OpenBaseRefPicker,
            ),
            (
                "Select Head Ref".to_owned(),
                "Open the right-side ref picker".to_owned(),
                PaletteCommand::OpenHeadRefPicker,
            ),
            (
                "Swap Compare Refs".to_owned(),
                "Swap the current base and head refs".to_owned(),
                PaletteCommand::SwapRefs,
            ),
            (
                "Run Compare".to_owned(),
                "Compare the selected refs now".to_owned(),
                PaletteCommand::StartCompare,
            ),
            (
                "Open Compare Menu".to_owned(),
                "Change compare mode or preset".to_owned(),
                PaletteCommand::OpenCompareMenu,
            ),
            (
                "Keymaps".to_owned(),
                "Review and rebind keyboard shortcuts".to_owned(),
                PaletteCommand::ShowKeyboardShortcuts,
            ),
            (
                "Toggle Sidebar".to_owned(),
                "Show or hide the file sidebar".to_owned(),
                PaletteCommand::ToggleSidebar,
            ),
            (
                "Toggle File Tree".to_owned(),
                "Switch sidebar between tree and flat list".to_owned(),
                PaletteCommand::ToggleFileTree,
            ),
            (
                "Expand All Folders".to_owned(),
                "Expand every folder in the file tree".to_owned(),
                PaletteCommand::ExpandAllFolders,
            ),
            (
                "Collapse All Folders".to_owned(),
                "Collapse every folder in the file tree".to_owned(),
                PaletteCommand::CollapseAllFolders,
            ),
            (
                "Toggle Wrap".to_owned(),
                "Enable or disable line wrapping".to_owned(),
                PaletteCommand::ToggleWrap,
            ),
            (
                "Toggle Continuous Scroll".to_owned(),
                "Switch between continuous and single-file diff navigation".to_owned(),
                PaletteCommand::ToggleContinuousScroll,
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
                "Use Built-in Renderer".to_owned(),
                "Render diffs with Diffy's built-in engine".to_owned(),
                PaletteCommand::SetRenderer(RendererKind::Builtin),
            ),
            (
                "Use Difftastic Renderer".to_owned(),
                "Render diffs with Difftastic".to_owned(),
                PaletteCommand::SetRenderer(RendererKind::Difftastic),
            ),
            (
                "Expand All Context".to_owned(),
                "Show all hidden context in the active diff".to_owned(),
                PaletteCommand::ExpandAllContext,
            ),
            (
                "Clear Line Selection".to_owned(),
                "Clear the current partial-line staging selection".to_owned(),
                PaletteCommand::ClearLineSelection,
            ),
            (
                "Generate Commit Message".to_owned(),
                "Draft a commit message from the current changes".to_owned(),
                PaletteCommand::GenerateCommitMessage,
            ),
            (
                "Fetch origin".to_owned(),
                "Update remote references from origin".to_owned(),
                PaletteCommand::FetchOrigin,
            ),
            (
                "Fetch all remotes".to_owned(),
                "Update remote references from every configured remote".to_owned(),
                PaletteCommand::FetchAllRemotes,
            ),
            (
                "Pull current branch".to_owned(),
                "Fast-forward the current Git branch from its upstream".to_owned(),
                PaletteCommand::PullCurrentBranch,
            ),
            (
                self.vcs_ui_profile().publish_command_label().to_owned(),
                self.vcs_ui_profile().publish_command_detail().to_owned(),
                PaletteCommand::PushCurrentBranch,
            ),
            (
                "Publish options".to_owned(),
                "Choose a backend-provided publish action".to_owned(),
                PaletteCommand::PublishOptions,
            ),
            (
                "Push current branch (force with lease)".to_owned(),
                "Force-push the current Git branch; refuse if upstream moved".to_owned(),
                PaletteCommand::PushCurrentBranchForceWithLease,
            ),
            (
                "Open Settings".to_owned(),
                "Configure appearance, editor, and behavior".to_owned(),
                PaletteCommand::OpenSettings,
            ),
        ] {
            if !palette_command_available(&command, repo_capabilities) {
                continue;
            }
            let search_text = format!("{label} {detail}");
            all_candidates.push(PaletteCandidate {
                search_text,
                label,
                detail,
                kind: PaletteEntryKind::Command(command),
            });
        }

        for section in SettingsSection::ALL {
            let label = format!("Settings: {}", section.label());
            let detail = "Switch settings section".to_owned();
            all_candidates.push(PaletteCandidate {
                search_text: format!("{label} {detail}"),
                label,
                detail,
                kind: PaletteEntryKind::Command(PaletteCommand::SetSettingsSection(section)),
            });
        }
        for (label, detail, mode) in [
            (
                "Use Dark Mode",
                "Set settings appearance to dark",
                ThemeMode::Dark,
            ),
            (
                "Use Light Mode",
                "Set settings appearance to light",
                ThemeMode::Light,
            ),
        ] {
            all_candidates.push(PaletteCandidate {
                search_text: format!("{label} {detail}"),
                label: label.to_owned(),
                detail: detail.to_owned(),
                kind: PaletteEntryKind::Command(PaletteCommand::SetThemeMode(mode)),
            });
        }
        for pct in [80, 90, 100, 110, 125, 150, 180] {
            let label = format!("Set UI Scale {pct}%");
            let detail = "Change interface density".to_owned();
            all_candidates.push(PaletteCandidate {
                search_text: format!("{label} {detail}"),
                label,
                detail,
                kind: PaletteEntryKind::Command(PaletteCommand::SetUiScalePct(pct)),
            });
        }
        for (column, label_suffix) in [(0, "Auto"), (80, "80"), (100, "100"), (120, "120")] {
            let label = format!("Set Wrap Column {label_suffix}");
            let detail = "Set line wrapping column".to_owned();
            all_candidates.push(PaletteCandidate {
                search_text: format!("{label} {detail}"),
                label,
                detail,
                kind: PaletteEntryKind::Command(PaletteCommand::SetWrapColumn(column)),
            });
        }
        for lines in [1, 2, 3, 5, 7] {
            let label = format!("Set Mouse Wheel Speed {lines}");
            let detail = "Set lines scrolled per wheel notch".to_owned();
            all_candidates.push(PaletteCandidate {
                search_text: format!("{label} {detail}"),
                label,
                detail,
                kind: PaletteEntryKind::Command(PaletteCommand::SetWheelScrollLines(lines)),
            });
        }
        all_candidates.push(PaletteCandidate {
            search_text: "Toggle Automatic Updates auto update".to_owned(),
            label: "Toggle Automatic Updates".to_owned(),
            detail: "Enable or disable hourly update checks".to_owned(),
            kind: PaletteEntryKind::Command(PaletteCommand::ToggleAutoUpdate),
        });
        all_candidates.push(PaletteCandidate {
            search_text: "Check For Updates update release".to_owned(),
            label: "Check For Updates".to_owned(),
            detail: "Check Diffy's release channel now".to_owned(),
            kind: PaletteEntryKind::Command(PaletteCommand::CheckForUpdates),
        });
        match self.update.get(&self.store) {
            UpdateState::Available(update) => {
                let label = format!("Install Update {}", update.version);
                let detail = "Download and verify the available update".to_owned();
                all_candidates.push(PaletteCandidate {
                    search_text: format!("{label} {detail}"),
                    label,
                    detail,
                    kind: PaletteEntryKind::Command(PaletteCommand::InstallUpdate),
                });
            }
            UpdateState::ReadyToRestart(update) => {
                let label = format!("Restart To Update {}", update.update.version);
                let detail = "Restart Diffy and apply the staged update".to_owned();
                all_candidates.push(PaletteCandidate {
                    search_text: format!("{label} {detail}"),
                    label,
                    detail,
                    kind: PaletteEntryKind::Command(PaletteCommand::RestartToUpdate),
                });
            }
            _ => {}
        }

        let repo_location = self.repository.location.get(&self.store);
        for operation in JjOperation::ALL.map(VcsOperation::Jj) {
            if !vcs_operation_available_for_location(&operation, repo_location.as_ref()) {
                continue;
            }
            let label = format!("jj: {}", operation.label());
            let detail = operation.detail();
            all_candidates.push(PaletteCandidate {
                search_text: format!("{label} {detail}"),
                label,
                detail,
                kind: PaletteEntryKind::Command(PaletteCommand::RunOperation(operation)),
            });
        }
        if repo_location
            .as_ref()
            .is_some_and(|location| location.profile == VCS_PROFILE_JJ)
        {
            let mut destinations = self.repository.refs.with(&self.store, |refs| {
                refs.iter()
                    .filter(|reference| {
                        !reference.active
                            && matches!(reference.kind, RefKind::Bookmark | RefKind::Branch)
                    })
                    .map(|reference| reference.name.clone())
                    .collect::<Vec<_>>()
            });
            destinations.sort();
            destinations.dedup();
            for destination in destinations.into_iter().take(12) {
                let operation = VcsOperation::JjRebaseCurrentChangeOnto {
                    destination: destination.clone(),
                };
                let label = format!("jj: {}", operation.label());
                let detail = operation.detail();
                all_candidates.push(PaletteCandidate {
                    search_text: format!("{label} {detail}"),
                    label,
                    detail,
                    kind: PaletteEntryKind::Command(PaletteCommand::RunOperation(operation)),
                });
            }
            let changes = self.repository.changes.get(&self.store);
            for change in changes
                .iter()
                .filter(|change| {
                    !change.flags.current && !change.flags.working_copy && !change.flags.immutable
                })
                .take(12)
            {
                let change_label = change
                    .short_change_id
                    .as_deref()
                    .unwrap_or(change.short_revision.as_str())
                    .to_owned();
                let operation = VcsOperation::JjEditRevision {
                    revision: change.revision.id.clone(),
                    label: change_label.clone(),
                };
                let label = format!("jj: {}", operation.label());
                let detail = crate::ui::vcs::change_summary_label(change);
                all_candidates.push(PaletteCandidate {
                    search_text: format!(
                        "{label} {detail} {} {}",
                        change.short_revision, change.revision.id
                    ),
                    label,
                    detail,
                    kind: PaletteEntryKind::Command(PaletteCommand::RunOperation(operation)),
                });
            }
            let operation_log = self.repository.operation_log.get(&self.store);
            for entry in operation_log.iter().skip(1).take(12) {
                let operation_label = entry.short_operation_id.clone();
                let operation = VcsOperation::JjRestoreOperation {
                    operation_id: entry.operation_id.clone(),
                    label: operation_label.clone(),
                };
                let label = format!("jj: {}", operation.label());
                let detail = operation_log_entry_detail(entry);
                all_candidates.push(PaletteCandidate {
                    search_text: format!(
                        "{label} {detail} {} {}",
                        entry.operation_id, entry.short_operation_id
                    ),
                    label,
                    detail,
                    kind: PaletteEntryKind::Command(PaletteCommand::RunOperation(operation)),
                });
            }
        }

        if self
            .workspace
            .pre_drill_compare
            .with(&self.store, |pre_drill| pre_drill.is_some())
        {
            all_candidates.push(PaletteCandidate {
                search_text: "Restore compare return range comparison commit drilldown".to_owned(),
                label: "Restore Compare".to_owned(),
                detail: "Return from the selected commit to the previous compare".to_owned(),
                kind: PaletteEntryKind::Command(PaletteCommand::RestoreCompare),
            });
        }

        if self
            .editor
            .line_selection
            .with(&self.store, |selection| !selection.is_empty())
        {
            all_candidates.push(PaletteCandidate {
                search_text: "Comment on selected lines review pull request".to_owned(),
                label: "Comment on Selected Lines".to_owned(),
                detail: "Open the pull request review comment composer".to_owned(),
                kind: PaletteEntryKind::Command(PaletteCommand::OpenReviewComment),
            });
        }

        if self.active_pull_request_web_url().is_some() {
            all_candidates.push(PaletteCandidate {
                search_text: "Open pull request in GitHub browser web PR".to_owned(),
                label: "Open Pull Request in GitHub".to_owned(),
                detail: "Open the active pull request on github.com".to_owned(),
                kind: PaletteEntryKind::Command(PaletteCommand::OpenPullRequestInGitHub),
            });
        }

        let file_count = self.workspace_file_count();
        for index in 0..file_count {
            let Some(file) = self.workspace_file_entry_at(index) else {
                continue;
            };
            let meta = self.file_list_entry_meta(index);
            let detail = format!(
                "File \u{2022} {} \u{2022} +{} -{}",
                meta.status.label(),
                meta.additions,
                meta.deletions
            );
            let search_text = format!("{} {detail}", file.path);
            all_candidates.push(PaletteCandidate {
                search_text,
                label: file.path.to_string(),
                detail,
                kind: PaletteEntryKind::File(index),
            });
        }

        let range_commits = self.workspace.range_commits.get(&self.store);
        for change in &range_commits {
            let label = crate::ui::vcs::change_summary_label(change);
            let detail = format!("Commit {}", change.short_revision);
            let search_text = format!("{} {} {}", change.short_revision, change.revision.id, label);
            all_candidates.push(PaletteCandidate {
                search_text,
                label,
                detail,
                kind: PaletteEntryKind::Commit(change.revision.id.clone()),
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

        let repo_refs = self.repository.refs.get(&self.store);
        for reference in repo_refs.iter().filter(|reference| {
            matches!(
                reference.kind,
                RefKind::Branch
                    | RefKind::RemoteBranch
                    | RefKind::Bookmark
                    | RefKind::RemoteBookmark
                    | RefKind::Tag
            )
        }) {
            let (detail, _) = self
                .vcs_ui_profile()
                .ref_kind_label_and_icon(reference.kind);
            let search_text = format!("{} {}", reference.name, detail);
            all_candidates.push(PaletteCandidate {
                search_text,
                label: reference.name.clone(),
                detail: detail.to_owned(),
                kind: PaletteEntryKind::Ref(CompareField::Left, reference.name.clone()),
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

    fn select_file(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
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

    fn select_file_inner(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
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

    fn active_file_matches_workspace_file(&self, index: usize) -> bool {
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

    fn select_text_compare_file(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
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

    fn select_compare_file(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
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
            self.compare_progress.set(&self.store, None);
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
        let mut effects = self.sync_editor_scroll_from_global();
        effects.extend(self.request_active_file_syntax_effect());
        effects
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
        let Some(file_change) = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| changes.get(index).cloned())
        else {
            tracing::warn!(
                index,
                "select_status_item: index out of range, returning empty"
            );
            return Vec::new();
        };
        tracing::debug!(
            index,
            path = %file_change.path,
            bucket = ?file_change.bucket,
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
            .set(&self.store, Some(file_change.path.clone()));
        self.workspace
            .selected_change_bucket
            .set(&self.store, Some(file_change.bucket));
        let (left_ref, right_ref) = self.status_refs_for_bucket(file_change.bucket);
        let active_matches_selection = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref().is_some_and(|active| {
                active.index == index
                    && active.path == file_change.path
                    && active.left_ref == left_ref
                    && active.right_ref == right_ref
            })
        });
        if active_matches_selection {
            self.workspace.active_file_loading.set(&self.store, None);
            self.clear_file_cache_loading(index);
            self.file_list.hovered_index.set(&self.store, Some(index));
            if reveal {
                self.reveal_file_list_row(index);
            }
            let mut effects = self.sync_editor_scroll_from_global();
            effects.push(ensure_syntax_packs_for_file_change_effect(&file_change));
            effects.extend(self.request_active_file_syntax_effect());
            return effects;
        } else if let Some(mut active_file) = self.cached_status_file_at(index, &file_change) {
            active_file.last_used_tick = self.next_file_working_set_tick();
            self.workspace.active_file_loading.set(&self.store, None);
            self.workspace
                .active_file
                .set(&self.store, Some(active_file.clone()));
            self.cache_active_file(active_file);
            self.editor_clear_document();
            self.file_list.hovered_index.set(&self.store, Some(index));
            if reveal {
                self.reveal_file_list_row(index);
            }
            let mut effects = self.sync_editor_scroll_from_global();
            effects.push(ensure_syntax_packs_for_file_change_effect(&file_change));
            effects.extend(self.request_active_file_syntax_effect());
            return effects;
        } else {
            let should_load = self.should_enqueue_file_load(
                index,
                &file_change.path,
                CompareWorkPriority::InteractiveSelectedFile,
            );
            self.workspace.active_file_loading.set(
                &self.store,
                Some(ActiveFileLoading {
                    index,
                    path: file_change.path.clone(),
                    priority: CompareWorkPriority::InteractiveSelectedFile,
                }),
            );
            self.mark_file_cache_loading(
                index,
                file_change.path.clone(),
                CompareWorkPriority::InteractiveSelectedFile,
            );
            self.file_list.hovered_index.set(&self.store, Some(index));
            if reveal {
                self.reveal_file_list_row(index);
            }

            let mut effects = vec![ensure_syntax_packs_for_file_change_effect(&file_change)];
            if should_load {
                let generation = self.workspace.status_generation.get(&self.store);
                let renderer = self.compare.renderer.get(&self.store);
                effects.push(
                    RepositoryEffect::LoadStatusDiff {
                        task: Task {
                            generation,
                            request: StatusDiffRequest {
                                repo_path,
                                file_change,
                                renderer,
                            },
                        },
                        index,
                    }
                    .into(),
                );
            }
            return effects;
        }
    }

    fn apply_selected_status_operation(&mut self, operation: FileOperation) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.staging_area)
            })
        {
            self.push_error("This repository backend does not support staging operations.");
            return Vec::new();
        }
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
        let Some(file_change) = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| changes.get(index).cloned())
        else {
            return Vec::new();
        };

        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        vec![
            RepositoryEffect::ApplyFileOperation(FileOperationRequest {
                repo_path,
                file_change,
                operation,
            })
            .into(),
        ]
    }

    fn apply_file_status_operation(
        &mut self,
        index: usize,
        operation: FileOperation,
    ) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.staging_area)
            })
        {
            self.push_error("This repository backend does not support staging operations.");
            return Vec::new();
        }
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let Some(file_change) = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| changes.get(index).cloned())
        else {
            return Vec::new();
        };

        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        vec![
            RepositoryEffect::ApplyFileOperation(FileOperationRequest {
                repo_path,
                file_change,
                operation,
            })
            .into(),
        ]
    }

    fn apply_batch_scope_operation(
        &mut self,
        buckets: &[ChangeBucket],
        operation: FileOperation,
    ) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.staging_area)
            })
        {
            self.push_error("This repository backend does not support staging operations.");
            return Vec::new();
        }
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let file_changes: Vec<FileChange> =
            self.workspace
                .status_file_changes
                .with(&self.store, |changes| {
                    changes
                        .iter()
                        .filter(|change| buckets.contains(&change.bucket))
                        .cloned()
                        .collect()
                });
        if file_changes.is_empty() {
            return Vec::new();
        }

        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        vec![
            RepositoryEffect::ApplyBatchFileOperation(BatchFileOperationRequest {
                repo_path,
                file_changes,
                operation,
            })
            .into(),
        ]
    }

    fn current_hunk_index_from_hover(&self) -> Option<i16> {
        self.editor
            .hovered_hunk_index
            .get(&self.store)
            .or_else(|| self.editor_current_hunk_index().map(|(idx, _)| idx as i16))
    }

    fn current_render_line_index_from_hover(&self) -> Option<usize> {
        self.editor
            .hovered_render_line_index
            .get(&self.store)
            .or_else(|| self.editor.hovered_row.get(&self.store))
    }

    fn apply_hunk_operation(
        &mut self,
        operation: FileOperation,
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
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.partial_hunk_mutation)
            })
        {
            self.push_error("This repository backend does not support hunk operations.");
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
        let Some(bucket) = self.workspace.selected_change_bucket.get(&self.store) else {
            tracing::debug!("apply_hunk_operation: bail: no selected_change_bucket");
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
                operation != FileOperation::Stage,
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
                bucket,
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
            crate::editor::diff::render_doc::RenderRowKind::Added
                | crate::editor::diff::render_doc::RenderRowKind::Removed
                | crate::editor::diff::render_doc::RenderRowKind::Modified
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
        self.insert_line_selection_range(row, anchor, false);
    }

    fn set_line_selection_range(&mut self, row: usize, anchor: usize) {
        self.insert_line_selection_range(row, anchor, true);
    }

    fn insert_line_selection_range(&mut self, row: usize, anchor: usize, clear_first: bool) {
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
        // Staging only selects changed lines; in PR review mode a comment can anchor
        // to any line (incl. context), like GitHub.
        let review = self.pull_request_review_enabled();
        self.editor.line_selection.update(&self.store, |ls| {
            if clear_first {
                ls.clear();
            }
            for line in &lines {
                use crate::editor::diff::render_doc::RenderRowKind;
                let kind = line.row_kind();
                if !kind.is_body() || line.hunk_index < 0 {
                    continue;
                }
                if !review
                    && !matches!(
                        kind,
                        RenderRowKind::Added | RenderRowKind::Removed | RenderRowKind::Modified
                    )
                {
                    continue;
                }
                let hunk_id = line.hunk_index as u32;
                if line.old_line_index >= 0 {
                    ls.entries
                        .insert(crate::editor::diff::state::LineSelectionKey {
                            file_path: None,
                            hunk_id,
                            side: carbon::DiffSide::Old,
                            source_index: line.old_line_index as u32,
                        });
                }
                if line.new_line_index >= 0 {
                    ls.entries
                        .insert(crate::editor::diff::state::LineSelectionKey {
                            file_path: None,
                            hunk_id,
                            side: carbon::DiffSide::New,
                            source_index: line.new_line_index as u32,
                        });
                }
            }
            ls.last_toggled_row = Some(row);
        });
    }

    fn toggle_current_line_selection(&mut self) {
        let Some(row) = self.current_render_line_index_from_hover() else {
            self.push_error("Move the row cursor to a changed line before selecting lines.");
            return;
        };
        self.toggle_line_selection(row, false);
    }

    fn toggle_current_line_selection_range(&mut self) {
        let Some(row) = self.current_render_line_index_from_hover() else {
            self.push_error("Move the row cursor to a changed line before selecting lines.");
            return;
        };
        let anchor = self
            .editor
            .line_selection
            .with(&self.store, |ls| ls.last_toggled_row);
        if let Some(anchor) = anchor {
            self.toggle_line_selection_range(row, anchor);
        } else {
            self.toggle_line_selection(row, false);
        }
    }

    fn apply_line_selection_operation(&mut self, operation: FileOperation) -> Vec<Effect> {
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
        let Some(bucket) = self.workspace.selected_change_bucket.get(&self.store) else {
            return Vec::new();
        };
        let reverse = operation != FileOperation::Stage;

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
                    bucket,
                    operation,
                })
                .into()
            })
            .collect()
    }

    fn scroll_viewport_lines(&mut self, delta_lines: i32) -> Vec<Effect> {
        let step_px = 20_i32;
        let delta_px = delta_lines.saturating_mul(step_px);
        self.scroll_viewport_px(delta_px)
    }

    fn scroll_active_overlay_list_px(&mut self, delta_px: i32) {
        match self.overlays_top() {
            Some(
                OverlaySurface::RepoPicker
                | OverlaySurface::RefPicker
                | OverlaySurface::ThemePicker
                | OverlaySurface::FontPicker,
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

    fn scroll_viewport_px(&mut self, delta_px: i32) -> Vec<Effect> {
        if !self.settings.continuous_scroll {
            let current = self.editor.scroll_top_px.get(&self.store);
            let max = self.editor_max_scroll_top_px();
            let next = apply_scroll_delta_px(current, delta_px, max);
            self.editor.scroll_top_px.set(&self.store, next);
            return Vec::new();
        }

        if delta_px == 0 {
            return Vec::new();
        }

        let current = self.workspace.global_scroll_top_px.get(&self.store);
        let target = apply_scroll_delta_px(current, delta_px, self.global_max_scroll_top_px());
        self.scroll_viewport_to_global(target)
    }

    fn clear_file_scroll_layout(&mut self) {
        self.workspace
            .file_content_heights
            .set(&self.store, Vec::new());
        self.workspace
            .file_scroll_total_height_px
            .set(&self.store, 0);
        self.workspace
            .pending_file_content_heights
            .set(&self.store, HashMap::new());
        self.workspace
            .file_scroll_recompute_pending
            .set(&self.store, false);
        self.workspace
            .viewport_scrollbar_drag
            .set(&self.store, None);
        self.virtual_diff_document.clear();
        self.virtual_scroll.clear();
        self.last_virtual_scroll_top_px = None;
    }

    fn reset_file_scroll_layout(&mut self) {
        self.workspace
            .file_content_heights
            .set(&self.store, Vec::new());
        self.workspace
            .pending_file_content_heights
            .set(&self.store, HashMap::new());
        self.workspace
            .file_scroll_recompute_pending
            .set(&self.store, false);
        self.workspace
            .viewport_scrollbar_drag
            .set(&self.store, None);
        self.virtual_scroll.clear();
        self.last_virtual_scroll_top_px = None;
        self.recompute_file_scroll_total_height_px();
    }

    pub fn recompute_file_scroll_total_height_px(&mut self) {
        let count = self.workspace_file_count();
        let source = self.workspace.source.get(&self.store);
        let generation = self.workspace_render_generation();
        if self
            .virtual_diff_document
            .sync_identity(source, generation, count)
        {
            self.virtual_scroll.clear();
            self.last_virtual_scroll_top_px = None;
        }
        self.workspace
            .file_content_heights
            .update(&self.store, |heights| {
                if heights.len() > count {
                    heights.truncate(count);
                }
            });

        let heights = (0..count)
            .map(|index| self.file_scroll_height_px(index).max(1))
            .collect::<Vec<_>>();
        self.virtual_diff_document.rebuild_heights(heights);
        let total = self.virtual_diff_document.total_u32();
        self.workspace
            .file_scroll_total_height_px
            .set(&self.store, total);
    }

    fn update_file_scroll_heights(&mut self, old_heights: Vec<(usize, u32)>) {
        let count = self.workspace_file_count();
        if self.virtual_diff_document.len() != count {
            self.recompute_file_scroll_total_height_px();
            return;
        }

        let mut total = self.workspace.file_scroll_total_height_px.get(&self.store);
        for (index, old_height) in old_heights {
            if index >= count {
                continue;
            }
            let new_height = self.file_scroll_height_px(index).max(1);
            total = total.saturating_sub(old_height).saturating_add(new_height);
            self.virtual_diff_document.update_height(index, new_height);
        }
        self.workspace
            .file_scroll_total_height_px
            .set(&self.store, total);
    }

    pub fn update_file_content_height_px(&mut self, index: usize, height: u32) -> bool {
        let count = self.workspace_file_count();
        if index >= count || height == 0 {
            return false;
        }
        if self.settings.continuous_scroll
            && self
                .workspace
                .viewport_scrollbar_drag
                .get(&self.store)
                .is_some()
        {
            self.workspace
                .pending_file_content_heights
                .update(&self.store, |pending| {
                    pending.insert(index, height);
                });
            return false;
        }
        if self.virtual_diff_document.len() != count {
            self.recompute_file_scroll_total_height_px();
        }

        let old_slot_height = self.file_scroll_height_px(index);
        let old_total = self.total_diff_height_px();
        let anchor = self
            .settings
            .continuous_scroll
            .then(|| self.current_or_derived_viewport_anchor())
            .flatten();
        let row_count = self.workspace_file_row_count(index);
        let mut recorded_changed = false;
        self.workspace
            .file_content_heights
            .update(&self.store, |heights| {
                if heights.len() < count {
                    heights.resize(count, None);
                }
                if heights[index] != Some(height) {
                    heights[index] = Some(height);
                    recorded_changed = true;
                }
            });

        let mut calibration_initialized = false;
        if let Some(rows) = row_count
            && rows > 0
        {
            let sample_q16 = (u64::from(height) << 16) / u64::from(rows);
            let prev = self.workspace.measured_px_per_row_q16.get(&self.store);
            let next = if prev == 0 {
                calibration_initialized = true;
                sample_q16 as u32
            } else {
                (((u64::from(prev) * 7) + sample_q16) / 8) as u32
            };
            self.workspace
                .measured_px_per_row_q16
                .set(&self.store, next);
        }

        if calibration_initialized {
            self.recompute_file_scroll_total_height_px();
        }

        if recorded_changed {
            let new_slot_height = self.file_scroll_height_px(index);
            let slot_height_changed = new_slot_height != old_slot_height;
            if calibration_initialized {
                self.workspace
                    .file_scroll_total_height_px
                    .set(&self.store, self.virtual_diff_document.total_u32());
            } else {
                let next_total = old_total
                    .saturating_sub(old_slot_height)
                    .saturating_add(new_slot_height);
                self.workspace
                    .file_scroll_total_height_px
                    .set(&self.store, next_total);
                self.virtual_diff_document
                    .update_height(index, new_slot_height.max(1));
            }

            if self.settings.continuous_scroll
                && slot_height_changed
                && let Some(anchor) = anchor
            {
                self.rebase_viewport_anchor(anchor);
            }
        }

        recorded_changed && old_slot_height != self.file_scroll_height_px(index)
    }

    pub fn update_virtual_diff_item_height_px(
        &mut self,
        item_id: VirtualDiffItemId,
        height: u32,
    ) -> bool {
        if item_id.kind != VirtualDiffItemKind::File
            || item_id.source != self.workspace.source.get(&self.store)
            || item_id.generation != self.workspace_render_generation()
        {
            return false;
        }
        self.update_file_content_height_px(item_id.index, height)
    }

    pub fn virtual_stream_item(
        &self,
        file_index: usize,
        kind: VirtualDiffItemKind,
        ordinal: u32,
        stable_key: u64,
        sort_key: u64,
        measured_height_px: Option<u32>,
    ) -> VirtualDiffStreamItem {
        VirtualDiffStreamItem::new(
            VirtualDiffItemId::new(
                self.workspace.source.get(&self.store),
                self.workspace_render_generation(),
                kind,
                file_index,
                ordinal,
                stable_key,
            ),
            sort_key,
            measured_height_px.unwrap_or_else(|| estimated_virtual_item_height_px(kind)),
            measured_height_px,
        )
    }

    fn virtual_stream_items_for_viewport_doc(
        &self,
        source: WorkspaceSource,
        generation: u64,
        slots: &[ViewportSlotKey],
        doc: &RenderDoc,
    ) -> Vec<VirtualDiffStreamItem> {
        let mut items = Vec::new();
        let mut slot_pos = None::<usize>;
        let mut local_ordinal = 0_u32;

        for (line_index, line) in doc.lines.iter().enumerate() {
            if line.row_kind() == RenderRowKind::FileHeader {
                slot_pos = Some(slot_pos.map_or(0, |pos| pos.saturating_add(1)));
                local_ordinal = 0;
            }

            let Some(slot) = slot_pos.and_then(|pos| slots.get(pos)) else {
                continue;
            };
            let Some(kind) = virtual_stream_item_kind(slot, line) else {
                continue;
            };
            let ordinal = match kind {
                VirtualDiffItemKind::FileHeader => 0,
                VirtualDiffItemKind::Hunk if line.hunk_index >= 0 => line.hunk_index as u32,
                _ => local_ordinal,
            };

            items.push(VirtualDiffStreamItem::new(
                VirtualDiffItemId::new(
                    source,
                    generation,
                    kind,
                    slot.index,
                    ordinal,
                    virtual_row_stable_key(line, ordinal),
                ),
                virtual_row_sort_key(line_index),
                estimated_virtual_item_height_px(kind),
                None,
            ));
            local_ordinal = local_ordinal.saturating_add(1);
        }

        items
    }

    fn file_scroll_height_px(&self, index: usize) -> u32 {
        self.workspace
            .file_content_heights
            .with(&self.store, |heights| heights.get(index).copied().flatten())
            .unwrap_or_else(|| self.estimated_file_height_px(index))
    }

    fn viewport_file_scroll_height_px(&self, index: usize) -> u32 {
        if let Some(height) = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| {
                drag.as_ref()
                    .and_then(|drag| drag.file_heights_px.get(index).copied())
            })
        {
            return height;
        }
        self.file_scroll_height_px(index)
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

    fn reconcile_selected_file_index_from_path(&mut self) -> Option<usize> {
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

    pub fn estimated_file_height_px(&self, index: usize) -> u32 {
        const BASELINE_ROWS: u32 = 8;
        let row_height_q16 = {
            let cal = self.workspace.measured_px_per_row_q16.get(&self.store);
            if cal == 0 { 24_u32 << 16 } else { cal }
        };
        let row_height_px =
            |rows: u32| ((u64::from(rows) * u64::from(row_height_q16)) >> 16) as u32;

        if matches!(
            self.workspace.source.get(&self.store),
            WorkspaceSource::Compare | WorkspaceSource::TextCompare
        ) && let Some(rows) = self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .and_then(|output| output.carbon.files.get(index))
                .map(estimated_carbon_file_rows_with_overhead)
        }) {
            return row_height_px(rows);
        }

        let line_count = match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare | WorkspaceSource::TextCompare => {
                if index < self.workspace_file_count() {
                    let meta = self.file_list_entry_meta(index);
                    meta.additions.saturating_add(meta.deletions).max(1) as u32 + BASELINE_ROWS
                } else {
                    BASELINE_ROWS
                }
            }
            WorkspaceSource::Status => BASELINE_ROWS,
            WorkspaceSource::None => BASELINE_ROWS,
        };
        row_height_px(line_count)
    }

    fn workspace_file_row_count(&self, index: usize) -> Option<u32> {
        if !matches!(
            self.workspace.source.get(&self.store),
            WorkspaceSource::Compare | WorkspaceSource::TextCompare
        ) {
            return None;
        }
        self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .and_then(|output| output.carbon.files.get(index))
                .map(estimated_carbon_file_rows_with_overhead)
        })
    }

    pub fn total_diff_height_px(&self) -> u32 {
        if let Some(total) = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| {
                drag.as_ref().map(|drag| drag.metrics.content_height_px)
            })
        {
            return total;
        }
        let cached = self.workspace.file_scroll_total_height_px.get(&self.store);
        if cached > 0 || self.workspace_file_count() == 0 {
            return cached;
        }

        self.virtual_diff_document.total_u32()
    }

    pub fn file_start_offset_px(&self, index: usize) -> u32 {
        if self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.is_none())
            && self.virtual_diff_document.len() == self.workspace_file_count()
        {
            return self.virtual_diff_document.prefix_u32(index);
        }
        let mut total: u32 = 0;
        for slot in 0..index.min(self.workspace_file_count()) {
            total = total.saturating_add(self.viewport_file_scroll_height_px(slot));
        }
        total
    }

    pub fn global_max_scroll_top_px(&self) -> u32 {
        if let Some(max) = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| {
                drag.as_ref().map(|drag| drag.metrics.max_scroll_top_px)
            })
        {
            return max;
        }
        let viewport = self.editor.viewport_height_px.get(&self.store);
        self.total_diff_height_px().saturating_sub(viewport.max(1))
    }

    fn viewport_anchor_bias_for_global(&self, scroll_top_px: u32) -> ViewportAnchorBias {
        let max = self.global_max_scroll_top_px();
        if max > 0 && scroll_top_px.saturating_add(CONTINUOUS_BOTTOM_ANCHOR_TOLERANCE_PX) >= max {
            ViewportAnchorBias::FollowEnd
        } else {
            ViewportAnchorBias::PreserveTop
        }
    }

    fn viewport_anchor_for_file_offset(
        &self,
        index: usize,
        local_offset_px: u32,
        bias: ViewportAnchorBias,
    ) -> Option<ViewportAnchor> {
        let item_id = self.virtual_diff_document.item_id(index)?;
        Some(ViewportAnchor {
            item_id,
            intra_item_offset_px: local_offset_px,
            bias,
        })
    }

    fn viewport_anchor_for_global(
        &self,
        scroll_top_px: u32,
        bias: ViewportAnchorBias,
    ) -> Option<ViewportAnchor> {
        let target_px = match bias {
            ViewportAnchorBias::PreserveBottom => {
                scroll_top_px.saturating_add(self.editor.viewport_height_px.get(&self.store).max(1))
            }
            ViewportAnchorBias::PreserveTop | ViewportAnchorBias::FollowEnd => scroll_top_px,
        };
        let (index, local_offset_px) = self.locate_global_scroll_px(target_px)?;
        self.viewport_anchor_for_file_offset(index, local_offset_px, bias)
    }

    fn current_or_derived_viewport_anchor(&self) -> Option<ViewportAnchor> {
        if let Some(anchor) = self.virtual_scroll.anchor
            && self.virtual_diff_document.anchor_is_current(anchor)
        {
            return Some(anchor);
        }
        let scroll_top_px = self.workspace.global_scroll_top_px.get(&self.store);
        let bias = self.viewport_anchor_bias_for_global(scroll_top_px);
        self.viewport_anchor_for_global(scroll_top_px, bias)
    }

    fn scroll_top_for_viewport_anchor(&self, anchor: ViewportAnchor) -> Option<u32> {
        if !self.virtual_diff_document.anchor_is_current(anchor) {
            return None;
        }
        if anchor.bias == ViewportAnchorBias::FollowEnd {
            return Some(self.global_max_scroll_top_px());
        }

        let index = anchor.item_id.index;
        let item_height = self
            .viewport_file_scroll_height_px(index)
            .max(self.virtual_diff_document.height_at(index))
            .max(1);
        let local_offset = anchor
            .intra_item_offset_px
            .min(item_height.saturating_sub(1));
        let item_top = self.file_start_offset_px(index);
        let target = match anchor.bias {
            ViewportAnchorBias::PreserveTop => item_top.saturating_add(local_offset),
            ViewportAnchorBias::PreserveBottom => item_top
                .saturating_add(local_offset)
                .saturating_sub(self.editor.viewport_height_px.get(&self.store).max(1)),
            ViewportAnchorBias::FollowEnd => unreachable!(),
        };
        Some(target.min(self.global_max_scroll_top_px()))
    }

    fn set_viewport_anchor(&mut self, anchor: ViewportAnchor) {
        if let Some(scroll_top_px) = self.scroll_top_for_viewport_anchor(anchor) {
            self.workspace
                .global_scroll_top_px
                .set(&self.store, scroll_top_px);
            self.virtual_scroll.set_anchor(anchor);
        } else {
            self.virtual_scroll.clear();
            self.clamp_global_scroll_top_px();
        }
    }

    fn set_viewport_anchor_for_global(&mut self, scroll_top_px: u32, bias: ViewportAnchorBias) {
        if let Some(anchor) = self.viewport_anchor_for_global(scroll_top_px, bias) {
            self.set_viewport_anchor(anchor);
        } else {
            self.virtual_scroll.clear();
            self.workspace.global_scroll_top_px.set(&self.store, 0);
        }
    }

    fn rebase_viewport_anchor(&mut self, anchor: ViewportAnchor) {
        self.set_viewport_anchor(anchor);
    }

    fn clamp_global_scroll_top_px(&mut self) {
        if let Some(anchor) = self.virtual_scroll.anchor
            && let Some(scroll_top_px) = self.scroll_top_for_viewport_anchor(anchor)
        {
            self.workspace
                .global_scroll_top_px
                .set(&self.store, scroll_top_px);
            return;
        }
        let max = self.global_max_scroll_top_px();
        let current = self.workspace.global_scroll_top_px.get(&self.store);
        self.workspace
            .global_scroll_top_px
            .set(&self.store, current.min(max));
    }

    fn locate_global_scroll_px(&self, target_px: u32) -> Option<(usize, u32)> {
        let count = self.workspace_file_count();
        if count == 0 {
            return None;
        }
        if self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.is_none())
            && self.virtual_diff_document.len() == count
        {
            return self.virtual_diff_document.locate(target_px);
        }
        let mut prior: u32 = 0;
        for index in 0..count {
            let height = self.viewport_file_scroll_height_px(index).max(1);
            let next_prior = prior.saturating_add(height);
            if target_px < next_prior || index + 1 == count {
                return Some((index, target_px.saturating_sub(prior)));
            }
            prior = next_prior;
        }
        Some((count - 1, 0))
    }

    fn scroll_viewport_to_global(&mut self, target_px: u32) -> Vec<Effect> {
        if self.virtual_diff_document.len() != self.workspace_file_count() {
            self.recompute_file_scroll_total_height_px();
        }
        let target_px = target_px.min(self.global_max_scroll_top_px());
        let bias = self.viewport_anchor_bias_for_global(target_px);
        self.set_viewport_anchor_for_global(target_px, bias);
        let target_px = self.workspace.global_scroll_top_px.get(&self.store);
        let Some((target_index, local_offset)) = self.locate_global_scroll_px(target_px) else {
            self.workspace.global_scroll_top_px.set(&self.store, 0);
            self.virtual_scroll.clear();
            return Vec::new();
        };
        self.workspace
            .global_scroll_top_px
            .set(&self.store, target_px);
        self.workspace
            .viewport_scrollbar_drag
            .update(&self.store, |drag| {
                if let Some(drag) = drag.as_mut() {
                    drag.metrics.scroll_top_px = target_px.min(drag.metrics.max_scroll_top_px);
                }
            });

        let dragging_scrollbar = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.is_some());
        let mut effects = if dragging_scrollbar {
            Vec::new()
        } else if self.active_file_matches_workspace_file(target_index) {
            Vec::new()
        } else {
            self.select_file_inner(target_index, true)
        };

        let local_max = self.editor_max_scroll_top_px();
        self.editor
            .scroll_top_px
            .set(&self.store, local_offset.min(local_max));
        if !dragging_scrollbar {
            effects.extend(self.request_active_file_syntax_effect());
        }
        effects
    }

    pub fn global_scroll_position_px(&self) -> u32 {
        self.workspace.global_scroll_top_px.get(&self.store)
    }

    pub fn continuous_viewport_scrollbar_metrics(&self) -> ViewportScrollbarMetrics {
        if let Some(metrics) = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.as_ref().map(|drag| drag.metrics))
        {
            return metrics;
        }
        let viewport_height_px = self.editor.viewport_height_px.get(&self.store);
        let content_height_px = self.total_diff_height_px();
        ViewportScrollbarMetrics {
            content_height_px,
            viewport_height_px,
            scroll_top_px: self.global_scroll_position_px(),
            max_scroll_top_px: content_height_px.saturating_sub(viewport_height_px.max(1)),
        }
    }

    pub fn begin_viewport_scrollbar_drag(
        &mut self,
        content_height_px: u32,
        viewport_height_px: u32,
        scroll_top_px: u32,
        max_scroll_top_px: u32,
    ) {
        if !self.settings.continuous_scroll {
            return;
        }
        let file_heights_px = (0..self.workspace_file_count())
            .map(|index| self.file_scroll_height_px(index).max(1))
            .collect();
        self.workspace.viewport_scrollbar_drag.set(
            &self.store,
            Some(ViewportScrollbarDragState {
                metrics: ViewportScrollbarMetrics {
                    content_height_px,
                    viewport_height_px,
                    scroll_top_px: scroll_top_px.min(max_scroll_top_px),
                    max_scroll_top_px,
                },
                file_heights_px,
            }),
        );
    }

    pub fn end_viewport_scrollbar_drag(&mut self) {
        self.workspace
            .viewport_scrollbar_drag
            .set(&self.store, None);
        self.apply_pending_file_scroll_updates();
    }

    fn apply_pending_file_scroll_updates(&mut self) {
        let pending_heights = self
            .workspace
            .pending_file_content_heights
            .with(&self.store, |pending| pending.clone());
        self.workspace
            .pending_file_content_heights
            .set(&self.store, HashMap::new());
        for (index, height) in pending_heights {
            self.update_file_content_height_px(index, height);
        }
        if self
            .workspace
            .file_scroll_recompute_pending
            .get(&self.store)
        {
            self.workspace
                .file_scroll_recompute_pending
                .set(&self.store, false);
            self.recompute_file_scroll_total_height_px();
            self.clamp_global_scroll_top_px();
        }
    }

    pub fn sync_editor_scroll_from_global(&mut self) -> Vec<Effect> {
        if !self.settings.continuous_scroll {
            return Vec::new();
        }
        self.clamp_global_scroll_top_px();
        let target = self.workspace.global_scroll_top_px.get(&self.store);
        let Some((_, local_offset)) = self.locate_global_scroll_px(target) else {
            self.workspace.global_scroll_top_px.set(&self.store, 0);
            self.virtual_scroll.clear();
            return Vec::new();
        };
        let max = self.editor_max_scroll_top_px();
        self.editor
            .scroll_top_px
            .set(&self.store, local_offset.min(max));
        Vec::new()
    }

    pub fn sync_global_scroll_from_editor(&mut self) {
        let Some(selected_index) = self.reconcile_selected_file_index_from_path() else {
            self.workspace.global_scroll_top_px.set(&self.store, 0);
            self.virtual_scroll.clear();
            return;
        };
        let start = self.file_start_offset_px(selected_index);
        let local = self.editor.scroll_top_px.get(&self.store);
        let target = start
            .saturating_add(local)
            .min(self.global_max_scroll_top_px());
        self.workspace.global_scroll_top_px.set(&self.store, target);
        if self.settings.continuous_scroll {
            if let Some(anchor) = self.viewport_anchor_for_file_offset(
                selected_index,
                local,
                self.viewport_anchor_bias_for_global(target),
            ) {
                self.virtual_scroll.set_anchor(anchor);
            } else {
                self.virtual_scroll.clear();
            }
        }
    }

    fn prefetch_compare_working_set(
        &mut self,
        render_start_index: usize,
        render_end_index: usize,
        direction: ScrollDirection,
        viewport_height_px: u32,
    ) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Compare {
            return Vec::new();
        }
        let count = self.workspace_file_count();
        if count == 0 {
            return Vec::new();
        }

        let forward_pages = if direction == ScrollDirection::Forward {
            COMPARE_WORKING_SET_PREFETCH_PAGES
        } else {
            COMPARE_WORKING_SET_TRAILING_PAGES
        };
        let backward_pages = if direction == ScrollDirection::Backward {
            COMPARE_WORKING_SET_PREFETCH_PAGES
        } else {
            COMPARE_WORKING_SET_TRAILING_PAGES
        };

        let mut effects = Vec::new();
        effects.extend(self.prefetch_compare_files_forward(
            render_end_index,
            viewport_height_px.saturating_mul(forward_pages).max(1),
        ));
        effects.extend(self.prefetch_compare_files_backward(
            render_start_index,
            viewport_height_px.saturating_mul(backward_pages).max(1),
        ));
        effects
    }

    fn prefetch_compare_files_forward(
        &mut self,
        start_index: usize,
        target_height: u32,
    ) -> Vec<Effect> {
        let count = self.workspace_file_count();
        let mut effects = Vec::new();
        let mut accumulated = 0_u32;
        let mut index = start_index;
        while index < count && accumulated < target_height {
            if let Some(path) = self.workspace_file_path_at(index) {
                effects.extend(self.ensure_compare_file_cached_for_viewport(
                    index,
                    &path,
                    CompareWorkPriority::Overscan,
                ));
            }
            accumulated =
                accumulated.saturating_add(self.viewport_file_scroll_height_px(index).max(1));
            index += 1;
        }
        effects
    }

    fn prefetch_compare_files_backward(
        &mut self,
        start_index: usize,
        target_height: u32,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        let mut accumulated = 0_u32;
        let mut index = start_index;
        while index > 0 && accumulated < target_height {
            index -= 1;
            if let Some(path) = self.workspace_file_path_at(index) {
                effects.extend(self.ensure_compare_file_cached_for_viewport(
                    index,
                    &path,
                    CompareWorkPriority::Overscan,
                ));
            }
            accumulated =
                accumulated.saturating_add(self.viewport_file_scroll_height_px(index).max(1));
        }
        effects
    }

    pub fn build_continuous_viewport_document(
        &mut self,
    ) -> (Option<ViewportDocument>, Vec<Effect>) {
        if !self.settings.continuous_scroll {
            return (None, Vec::new());
        }
        if self.virtual_diff_document.len() != self.workspace_file_count() {
            self.recompute_file_scroll_total_height_px();
        }
        self.clamp_global_scroll_top_px();
        let scroll_top_px = self.workspace.global_scroll_top_px.get(&self.store);
        let scroll_direction = match self.last_virtual_scroll_top_px {
            Some(previous) if scroll_top_px < previous => ScrollDirection::Backward,
            _ => ScrollDirection::Forward,
        };
        self.last_virtual_scroll_top_px = Some(scroll_top_px);
        let Some((anchor_index, _)) = self.locate_global_scroll_px(scroll_top_px) else {
            return (None, Vec::new());
        };

        let source = self.workspace.source.get(&self.store);
        if source == WorkspaceSource::None {
            return (None, Vec::new());
        }
        let dragging_scrollbar = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.is_some());

        let count = self.workspace_file_count();
        let viewport = self.editor.viewport_height_px.get(&self.store).max(1);
        let follow_end = self.virtual_scroll.anchor.is_some_and(|anchor| {
            anchor.bias == ViewportAnchorBias::FollowEnd
                && self.virtual_diff_document.anchor_is_current(anchor)
        }) || self.viewport_anchor_bias_for_global(scroll_top_px)
            == ViewportAnchorBias::FollowEnd;
        let (start_index, start_offset, local_top, target_height) = if follow_end {
            let mut start_index = count.saturating_sub(1);
            let mut tail_height = self.viewport_file_scroll_height_px(start_index).max(1);
            let target_tail_height = viewport.saturating_mul(2).max(viewport);
            while start_index > 0 && tail_height < target_tail_height {
                start_index -= 1;
                tail_height = tail_height
                    .saturating_add(self.viewport_file_scroll_height_px(start_index).max(1));
            }
            (
                start_index,
                self.file_start_offset_px(start_index),
                tail_height.saturating_sub(viewport),
                tail_height.max(1),
            )
        } else {
            let mut start_index = anchor_index;
            let mut before_viewport_px = 0_u32;
            while start_index > 0 && before_viewport_px < viewport {
                start_index -= 1;
                before_viewport_px = before_viewport_px
                    .saturating_add(self.viewport_file_scroll_height_px(start_index).max(1));
            }
            let start_offset = self.file_start_offset_px(start_index);
            let local_top = self
                .workspace
                .global_scroll_top_px
                .get(&self.store)
                .saturating_sub(start_offset);
            let target_height = local_top
                .saturating_add(viewport)
                .saturating_add(viewport / 2)
                .max(1);
            (start_index, start_offset, local_top, target_height)
        };

        let mut effects = Vec::new();
        let mut slot_keys = Vec::new();
        let mut slot_loading = Vec::new();
        let mut accumulated = 0_u32;
        let mut index = start_index;
        while index < count && (slot_keys.is_empty() || accumulated < target_height) {
            let path = self
                .workspace_file_path_at(index)
                .unwrap_or_else(|| format!("File {}", index + 1));
            let slot_key = match source {
                WorkspaceSource::Compare | WorkspaceSource::TextCompare => {
                    effects.extend(self.ensure_compare_file_cached_for_viewport(
                        index,
                        &path,
                        CompareWorkPriority::VisibleViewportDiff,
                    ));
                    self.compare_slot_key_at(index, &path)
                }
                WorkspaceSource::Status => {
                    effects.extend(self.ensure_status_file_cached_for_viewport(index));
                    let file_change = self
                        .workspace
                        .status_file_changes
                        .with(&self.store, |changes| changes.get(index).cloned());
                    file_change.as_ref().map_or_else(
                        || {
                            self.loading_slot_key(
                                WorkspaceSource::Status,
                                index,
                                &path,
                                String::new(),
                                String::new(),
                            )
                        },
                        |change| self.status_slot_key_at(index, change),
                    )
                }
                WorkspaceSource::None => self.loading_slot_key(
                    WorkspaceSource::None,
                    index,
                    &path,
                    String::new(),
                    String::new(),
                ),
            };
            let slot_height = self.viewport_file_scroll_height_px(index).max(1);
            if let Some(window) = self.viewport_slot_syntax_window(
                &slot_key,
                accumulated,
                slot_height,
                local_top,
                viewport,
            ) {
                effects.extend(self.request_viewport_slot_syntax_window(&slot_key, window));
            }
            let slot_is_loading = matches!(&slot_key.kind, ViewportSlotKind::Loading);
            if !slot_is_loading {
                self.touch_viewport_slot(&slot_key);
            }
            slot_loading.push(slot_is_loading);
            slot_keys.push(slot_key);
            accumulated = accumulated.saturating_add(slot_height);
            index += 1;
        }
        let render_end_index = index;
        self.protect_working_set_slots(&slot_keys);
        self.trim_file_working_set();
        effects.extend(self.prefetch_compare_working_set(
            start_index,
            render_end_index,
            scroll_direction,
            viewport,
        ));

        let key = ViewportDocumentKey {
            source,
            generation: self.workspace_render_generation(),
            start_index,
            slots: slot_keys,
        };
        let doc = if let Some(cache) = self.viewport_document_cache.as_ref()
            && cache.key == key
        {
            cache.doc.clone()
        } else {
            let mut doc = RenderDoc::default();
            let loading_message = if dragging_scrollbar {
                ""
            } else {
                "Loading diff..."
            };
            for slot in &key.slots {
                self.append_viewport_slot_doc(&mut doc, slot, loading_message);
            }
            let doc = Arc::new(doc);
            self.viewport_document_cache = Some(ViewportDocumentCache {
                key: key.clone(),
                doc: doc.clone(),
            });
            doc
        };
        let slot_indices = key.slots.iter().map(|slot| slot.index).collect();
        let slot_item_ids = key
            .slots
            .iter()
            .map(|slot| {
                self.virtual_diff_document
                    .item_id(slot.index)
                    .unwrap_or_else(|| {
                        VirtualDiffItemId::file(
                            source,
                            self.workspace_render_generation(),
                            slot.index,
                        )
                    })
            })
            .collect();
        let stream_items = self.virtual_stream_items_for_viewport_doc(
            source,
            self.workspace_render_generation(),
            &key.slots,
            doc.as_ref(),
        );

        (
            Some(ViewportDocument {
                doc,
                mode: ViewportDocumentMode::Continuous,
                generation: self.workspace_render_generation(),
                start_index,
                start_offset_px: start_offset,
                scroll_top_px: local_top,
                slot_indices,
                slot_item_ids,
                stream_items,
                slot_loading,
                path: String::new(),
            }),
            effects,
        )
    }

    fn scroll_viewport_pages(&mut self, delta_pages: i32) -> Vec<Effect> {
        let viewport = self.editor.viewport_height_px.get(&self.store);
        let page_px = ((viewport as f32) * 0.85).round().max(1.0) as i32;
        let delta_px = delta_pages.saturating_mul(page_px);
        if self.settings.continuous_scroll {
            return self.scroll_viewport_px(delta_px);
        }
        let current = self.editor.scroll_top_px.get(&self.store);
        let max = self.editor_max_scroll_top_px();
        let next = apply_scroll_delta_px(current, delta_px, max);
        self.editor.scroll_top_px.set(&self.store, next);
        Vec::new()
    }

    fn scroll_viewport_half_page(&mut self, direction: i32) -> Vec<Effect> {
        let viewport = self.editor.viewport_height_px.get(&self.store);
        let half_px = ((viewport as f32) * 0.5).round().max(1.0) as i32;
        let delta_px = direction.saturating_mul(half_px);
        if self.settings.continuous_scroll {
            return self.scroll_viewport_px(delta_px);
        }
        let current = self.editor.scroll_top_px.get(&self.store);
        let max = self.editor_max_scroll_top_px();
        let next = apply_scroll_delta_px(current, delta_px, max);
        self.editor.scroll_top_px.set(&self.store, next);
        Vec::new()
    }

    fn request_active_file_syntax_effect(&mut self) -> Option<Effect> {
        if !self.syntax_request_budget_available() {
            return None;
        }
        let repo_path = self.compare.repo_path.get(&self.store)?;
        let window = self.desired_syntax_window()?;
        let generation = self.active_syntax_generation();
        let syntax_epoch = self.syntax_requests.epoch();
        let mut request = None;
        let request_id = self.syntax_requests.next_request_id();
        let mut active_to_cache = None;

        self.workspace.active_file.update(&self.store, |active| {
            let Some(active) = active.as_mut() else {
                return;
            };
            if let Some(next_request) = request_syntax_for_active_file(
                active,
                repo_path,
                generation,
                syntax_epoch,
                window,
                request_id,
            ) {
                active_to_cache = Some(active.clone());
                request = Some(next_request);
            }
        });
        if let Some(active_file) = active_to_cache {
            self.cache_active_file(active_file);
        }

        request.map(|request| {
            self.track_syntax_request(&request);
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

    fn navigate_to_file(&mut self, forward: bool) -> Vec<Effect> {
        let Some(current) = self.reconcile_selected_file_index_from_path() else {
            return Vec::new();
        };
        let count = self.workspace_file_count();
        if count == 0 {
            return Vec::new();
        }
        let target = if forward {
            current.saturating_add(1).min(count.saturating_sub(1))
        } else {
            current.saturating_sub(1)
        };
        if target == current {
            return Vec::new();
        }

        if self.settings.continuous_scroll {
            return self.select_file(target, true);
        }

        self.select_file(target, true)
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
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.remotes)
            })
        {
            self.push_error("This repository backend does not support remotes.");
            return Vec::new();
        }
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
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.remotes)
            })
        {
            self.push_error("This repository backend does not support remotes.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before fetching.");
            return Vec::new();
        };
        let remotes = self.repository.refs.with(&self.store, |refs| {
            remote_names_from_refs(refs).into_iter().collect::<Vec<_>>()
        });
        if remotes.is_empty() {
            self.push_error("No remotes are configured for this repository.");
            return Vec::new();
        }
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

    fn start_publish_default(&mut self) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.remotes)
            })
        {
            self.push_error("This repository backend does not support publishing.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before publishing.");
            return Vec::new();
        };
        let toast_id = self.push_progress_toast(&format!(
            "{}\u{2026}",
            self.vcs_ui_profile().publish_command_label()
        ));
        vec![
            RepositoryEffect::PublishDefault(PublishRequest {
                repo_path,
                action: None,
                toast_id,
            })
            .into(),
        ]
    }

    fn start_open_publish_menu(&mut self) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.remotes)
            })
        {
            self.push_error("This repository backend does not support publishing.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before publishing.");
            return Vec::new();
        };
        self.push_overlay(OverlaySurface::PublishMenu, None);
        vec![
            RepositoryEffect::LoadPublishPlan(PublishPlanRequest {
                repo_path,
                toast_id: None,
            })
            .into(),
        ]
    }

    fn start_publish_action(&mut self, action: PublishAction) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before publishing.");
            return Vec::new();
        };
        if self.overlays_top() == Some(OverlaySurface::PublishMenu) {
            self.pop_overlay();
        }
        let toast_id = self.push_progress_toast(&format!("{}\u{2026}", action.label));
        vec![
            RepositoryEffect::PublishDefault(PublishRequest {
                repo_path,
                action: Some(action),
                toast_id,
            })
            .into(),
        ]
    }

    fn start_push_current_branch(&mut self, force_with_lease: bool) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.remotes)
            })
        {
            self.push_error("This repository backend does not support push.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before pushing.");
            return Vec::new();
        };
        let Some(branch_ref) = self
            .repository
            .refs
            .with(&self.store, |refs| active_publish_ref(refs))
        else {
            self.push_error("No active branch or bookmark to push.");
            return Vec::new();
        };
        let branch = branch_ref.name;
        let (remote, refspec) = match branch_ref.upstream.as_deref().and_then(upstream_pair) {
            Some((remote, upstream_branch)) => (
                remote,
                format!("refs/heads/{branch}:refs/heads/{upstream_branch}"),
            ),
            None => {
                // No upstream configured yet — default to `origin/<branch>`.
                let remotes = self.repository.refs.with(&self.store, |refs| {
                    remote_names_from_refs(refs).into_iter().collect::<Vec<_>>()
                });
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
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.pull_fast_forward)
            })
        {
            self.push_error("This repository backend does not support fast-forward pull.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before pulling.");
            return Vec::new();
        };
        let Some(branch_ref) = self
            .repository
            .refs
            .with(&self.store, |refs| active_publish_ref(refs))
        else {
            self.push_error("No active branch or bookmark to pull into.");
            return Vec::new();
        };
        let branch = branch_ref.name;
        let (remote, upstream_branch) = match branch_ref.upstream.as_deref().and_then(upstream_pair)
        {
            Some(pair) => pair,
            None => {
                self.push_error(&format!(
                    "No upstream configured for {branch}. Push once to set one."
                ));
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
        use crate::editor::diff::state::MatchSide;

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
        self.editor.hovered_render_line_index.set(&self.store, None);
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
        self.editor.text_selection.set(&self.store, None);
        self.context_menu.close();
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

    fn move_editor_row_cursor(&mut self, delta: i32) {
        let Some(start) = self.editor.visible_row_start.get(&self.store) else {
            return;
        };
        let Some(end) = self.editor.visible_row_end.get(&self.store) else {
            return;
        };
        if start >= end {
            return;
        }
        let max = end.saturating_sub(1);
        let Some(current) = self
            .editor
            .hovered_row
            .get(&self.store)
            .filter(|row| *row >= start && *row <= max)
        else {
            self.editor
                .hovered_row
                .set(&self.store, Some(if delta < 0 { max } else { start }));
            return;
        };
        let next = if delta < 0 {
            current
                .saturating_sub(delta.unsigned_abs() as usize)
                .max(start)
        } else {
            current.saturating_add(delta as usize).min(max)
        };
        self.editor.hovered_row.set(&self.store, Some(next));
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

fn estimated_carbon_file_rows_with_overhead(file: &carbon::FileDiff) -> u32 {
    if file.is_binary {
        return 4;
    }
    estimated_carbon_file_rows(file).saturating_add(1).max(1)
}

fn estimated_carbon_file_rows(file: &carbon::FileDiff) -> u32 {
    if file.hunks.is_empty() {
        return file.additions.saturating_add(file.deletions).max(1);
    }

    let mut rows = 0_u32;
    for (hunk_index, hunk) in file.hunks.iter().enumerate() {
        if !file.is_partial {
            let gap_len = if hunk_index == 0 {
                hunk.old_start_index().min(hunk.new_start_index())
            } else {
                let prev = &file.hunks[hunk_index - 1];
                hunk.old_start_index()
                    .saturating_sub(prev.old_end_index())
                    .min(hunk.new_start_index().saturating_sub(prev.new_end_index()))
            };
            rows = rows.saturating_add((gap_len > 0) as u32);
        }

        rows = rows.saturating_add(1);
        for block in file.hunk_blocks(hunk) {
            rows = rows.saturating_add(match block.kind {
                carbon::BlockKind::Context => block.old.len.min(block.new.len),
                carbon::BlockKind::Change => block.old.len.saturating_add(block.new.len),
            });
        }

        if !file.is_partial && hunk_index + 1 == file.hunks.len() {
            let old_end = file
                .old_text
                .as_ref()
                .map(|text| text.line_count())
                .unwrap_or_else(|| hunk.old_end_index());
            let new_end = file
                .new_text
                .as_ref()
                .map(|text| text.line_count())
                .unwrap_or_else(|| hunk.new_end_index());
            let gap_len = old_end
                .saturating_sub(hunk.old_end_index())
                .min(new_end.saturating_sub(hunk.new_end_index()));
            rows = rows.saturating_add((gap_len > 0) as u32);
        }
    }
    rows
}

fn compare_summary_file_entry(summary: &CompareFileSummary) -> FileListEntry {
    FileListEntry {
        path: summary.paths.display_path_ref(),
    }
}

fn compare_output_file_entry_meta(
    output: &CompareOutput,
    index: usize,
) -> Option<FileListEntryMeta> {
    if let Some(summary) = output.file_summaries.get(index) {
        let (additions, deletions) = summary.fallback_stats();
        return Some(FileListEntryMeta {
            status: carbon_list_status(summary.status),
            additions,
            deletions,
            is_binary: summary.is_binary,
        });
    }
    output.carbon.files.get(index).map(carbon_file_entry_meta)
}

fn carbon_file_entry_meta(file: &carbon::FileDiff) -> FileListEntryMeta {
    let (additions, deletions) = carbon_file_stats(file);
    FileListEntryMeta {
        status: carbon_list_status(file.status),
        additions,
        deletions,
        is_binary: file.is_binary,
    }
}

fn compare_output_summary_is_deferred(output: &CompareOutput, index: usize) -> bool {
    if let Some(summary) = output.file_summaries.get(index) {
        return summary.is_partial;
    }
    output
        .carbon
        .files
        .get(index)
        .is_some_and(|file| file.is_partial && file.hunks.is_empty())
}

fn compare_output_deferred_summary(
    output: &CompareOutput,
    index: usize,
) -> Option<CompareFileSummary> {
    if let Some(summary) = output.file_summaries.get(index) {
        return summary.is_partial.then(|| summary.clone());
    }
    output
        .carbon
        .files
        .get(index)
        .filter(|file| file.is_partial && file.hunks.is_empty())
        .map(CompareFileSummary::from_file)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CompareStatsSnapshot {
    hydrated_total: (i32, i32),
    deferred_count: usize,
}

fn compare_output_stats_snapshot(output: &CompareOutput) -> CompareStatsSnapshot {
    let mut snapshot = CompareStatsSnapshot::default();
    output.for_each_summary(|_, summary| {
        if summary.stats_deferred {
            snapshot.deferred_count = snapshot.deferred_count.saturating_add(1);
        } else {
            let stats = summary.fallback_stats();
            snapshot.hydrated_total = (
                snapshot.hydrated_total.0.saturating_add(stats.0),
                snapshot.hydrated_total.1.saturating_add(stats.1),
            );
        }
    });
    snapshot
}

fn compare_output_has_deferred_stats(output: &CompareOutput) -> bool {
    if output.file_summaries.is_empty() {
        output.carbon.files.iter().any(|file| file.stats_deferred)
    } else {
        output
            .file_summaries
            .iter()
            .any(|summary| summary.stats_deferred)
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

fn carbon_list_status(status: carbon::FileStatus) -> FileListStatus {
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

fn build_status_file_entries(changes: &[FileChange]) -> Vec<FileListEntry> {
    changes.iter().map(FileListEntry::from).collect()
}

fn active_publish_ref(refs: &[VcsRef]) -> Option<VcsRef> {
    refs.iter()
        .find(|reference| {
            reference.active && matches!(reference.kind, RefKind::Branch | RefKind::Bookmark)
        })
        .cloned()
}

fn upstream_pair(upstream: &str) -> Option<(String, String)> {
    upstream
        .split_once('/')
        .map(|(remote, branch)| (remote.to_owned(), branch.to_owned()))
}

fn remote_names_from_refs(refs: &[VcsRef]) -> std::collections::BTreeSet<String> {
    let mut remotes = std::collections::BTreeSet::new();
    for reference in refs {
        if let Some((remote, _)) = reference
            .upstream
            .as_deref()
            .and_then(|upstream| upstream.split_once('/'))
        {
            remotes.insert(remote.to_owned());
        }
        if matches!(
            reference.kind,
            RefKind::RemoteBranch | RefKind::RemoteBookmark
        ) && let Some((remote, _)) = reference.name.split_once('/')
        {
            remotes.insert(remote.to_owned());
        }
    }
    remotes
}

fn status_section_count(changes: &[FileChange]) -> usize {
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

fn status_section_count_before(changes: &[FileChange], len: usize) -> usize {
    status_section_count(&changes[..len.min(changes.len())])
}

fn overlay_name(surface: OverlaySurface) -> &'static str {
    match surface {
        OverlaySurface::RepoPicker => "repo-picker",
        OverlaySurface::RefPicker => "ref-picker",
        OverlaySurface::CommandPalette => "command-palette",
        OverlaySurface::Confirmation => "confirmation",
        OverlaySurface::GitHubAuthModal => "github-auth-modal",
        OverlaySurface::AccountMenu => "account-menu",
        OverlaySurface::KeyboardShortcuts => "keyboard-shortcuts",
        OverlaySurface::ThemePicker => "theme-picker",
        OverlaySurface::FontPicker => "font-picker",
        OverlaySurface::CompareMenu => "compare-menu",
        OverlaySurface::PublishMenu => "publish-menu",
    }
}

fn font_picker_entry(
    entry: &FontFamilyEntry,
    selected_family: &str,
    highlights: Vec<(usize, usize)>,
) -> PickerEntry {
    let source = entry.source.label();
    let detail = if entry.family == selected_family {
        format!("Selected - {source}")
    } else {
        source.to_owned()
    };
    PickerEntry {
        label: entry.label.clone(),
        detail,
        value: entry.family.clone(),
        highlights,
        label_style: PickerLabelStyle::Default,
        icon: Some(if entry.monospaced {
            lucide::TERMINAL
        } else {
            lucide::FILE
        }),
        section_header: false,
    }
}

pub fn workspace_mode_name(mode: WorkspaceMode) -> &'static str {
    match mode {
        WorkspaceMode::Empty => "empty",
        WorkspaceMode::Loading => "loading",
        WorkspaceMode::Ready => "ready",
    }
}

impl From<&FileChange> for FileListEntry {
    fn from(value: &FileChange) -> Self {
        Self {
            path: ComparePath::from(value.path.as_str()),
        }
    }
}

fn status_file_entry_meta(change: &FileChange) -> FileListEntryMeta {
    FileListEntryMeta {
        status: file_change_list_status(change.status, change.bucket),
        additions: 0,
        deletions: 0,
        is_binary: matches!(change.status, FileChangeStatus::Binary),
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

fn path_looks_like_repository(path: &Path) -> bool {
    path.join(".git").exists() || path.join(".jj").exists()
}

fn normalize_repository_open_path(path: PathBuf) -> PathBuf {
    crate::core::vcs::discovery::discover_repository(&path)
        .ok()
        .flatten()
        .map(|location| location.workspace_root)
        .unwrap_or(path)
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
    use std::sync::Arc;

    use clap::Parser;

    use super::{
        ActiveFile, ActiveFileLoading, AppState, AsyncStatus, CarbonStyleOverlays,
        CardTextSelection, CompareField, FILE_HEIGHT_SPARSE_MIN_COUNT, FileHeightIndex,
        FileListEntry, FocusTarget, OverlayEntry, OverlaySurface, PickerItem, PickerLabelStyle,
        PreparedActiveFile, SidebarMode, SidebarTab, TextCompareLanguage, TextCompareView,
        ViewportAnchorBias, VirtualDiffItemKind, WorkspaceMode, WorkspaceSource,
        prepare_active_file, vcs_compare_request,
    };
    use crate::core::compare::{
        CompareFileSummary, CompareMode, CompareOutput, LayoutMode, RendererKind,
    };
    use crate::core::text::TokenBuffer;
    use crate::core::vcs::model::{
        ChangeBucket, ChangeFlags, FileChange, FileChangeStatus, JjOperation, RefKind,
        RepoCapabilities, RepoLocation, RevisionId, VcsChange, VcsKind, VcsOperation,
        VcsOperationLogEntry, VcsRef,
    };
    use crate::editor::EditorMode;
    use crate::editor::diff::render_doc::{RenderDoc, build_render_doc_from_carbon};
    use crate::effects::{
        AiEffect, CompareEffect, CompareWorkPriority, Effect, GitHubEffect, RepositoryEffect,
        SettingsEffect, SyntaxEffect,
    };
    use crate::events::{
        AppEvent, CompareEvent, CompareFileFinished, CompareFileStat, CompareFileStatsReady,
        CompareStatsReady, GitHubEvent, RepositoryEvent, TextCompareFinished,
    };
    use crate::platform::persistence::Settings;
    use crate::platform::startup::{Args, StartupOptions};

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

    #[test]
    fn new_text_compare_enters_text_workspace_with_left_focus() {
        let mut state = AppState::default();

        state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);

        assert_eq!(
            state.workspace.source.get(&state.store),
            WorkspaceSource::TextCompare
        );
        assert_eq!(state.text_compare.view, TextCompareView::Edit);
        assert_eq!(state.text_compare.left_editor.mode(), EditorMode::CodeInput);
        assert_eq!(
            state.text_compare.right_editor.mode(),
            EditorMode::CodeInput
        );
        assert_eq!(state.text_compare.language, TextCompareLanguage::Auto);
        assert_eq!(state.text_compare.path_hint, "text.txt");
        assert_eq!(
            state.focus.get(&state.store),
            Some(FocusTarget::TextCompareLeft)
        );
    }

    #[test]
    fn text_compare_paste_routes_to_focused_side() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);

        state.apply_action(crate::actions::TextEditAction::Paste("left".to_owned()));
        state.apply_action(crate::actions::AppAction::SetFocus(Some(
            FocusTarget::TextCompareRight,
        )));
        state.apply_action(crate::actions::TextEditAction::Paste("right".to_owned()));

        assert_eq!(state.text_compare.left_editor.text(), "left");
        assert_eq!(state.text_compare.right_editor.text(), "right");
        assert_eq!(state.text_compare.left_editor.line_count(), 1);
        assert_eq!(state.text_compare.right_editor.line_count(), 1);
    }

    #[test]
    fn text_compare_auto_language_detects_pasted_rust() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);

        state.apply_action(crate::actions::TextEditAction::Paste(
            "pub fn main() {\n    println!(\"hi\");\n}\n".to_owned(),
        ));

        assert_eq!(
            state.text_compare.detected_language,
            Some(TextCompareLanguage::Rust)
        );
        assert_eq!(state.text_compare.path_hint, "scratch.rs");
    }

    #[test]
    fn text_compare_auto_language_detects_pasted_typescript() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);

        state.apply_action(crate::actions::TextEditAction::Paste(
            "const answer: number = 42;\nexport { answer };\n".to_owned(),
        ));

        assert_eq!(
            state.text_compare.detected_language,
            Some(TextCompareLanguage::TypeScript)
        );
        assert_eq!(state.text_compare.path_hint, "scratch.ts");
    }

    #[test]
    fn text_compare_language_override_sets_compare_path() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);

        state.apply_action(crate::actions::TextCompareAction::SetLanguage(
            TextCompareLanguage::TypeScript,
        ));
        state.apply_action(crate::actions::TextEditAction::Paste(
            "pub fn main() {}\n".to_owned(),
        ));
        let effects = state.apply_action(crate::actions::TextCompareAction::CompareNow);
        let request_path = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Compare(CompareEffect::RunText(task)) => {
                    Some(task.request.display_path.as_str())
                }
                _ => None,
            })
            .unwrap();

        assert_eq!(state.text_compare.language, TextCompareLanguage::TypeScript);
        assert_eq!(state.text_compare.path_hint, "scratch.ts");
        assert_eq!(request_path, "scratch.ts");
    }

    #[test]
    fn text_compare_swap_sides_preserves_text_and_marks_stale() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);
        state.text_compare.left_editor.set_text("old");
        state.text_compare.right_editor.set_text("new");
        let generation = state.text_compare.generation;

        state.apply_action(crate::actions::TextCompareAction::SwapSides);

        assert_eq!(state.text_compare.left_editor.text(), "new");
        assert_eq!(state.text_compare.right_editor.text(), "old");
        assert!(state.text_compare.generation > generation);
        assert!(state.text_compare_is_stale());
    }

    #[test]
    fn stale_text_compare_finished_event_is_ignored() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);
        let effects = state.apply_action(crate::actions::TextCompareAction::CompareNow);
        let generation = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Compare(CompareEffect::RunText(task)) => Some(task.generation),
                _ => None,
            })
            .unwrap();
        state.apply_action(crate::actions::TextEditAction::Paste("newer".to_owned()));

        state.apply_event(AppEvent::from(CompareEvent::TextCompareFinished(
            TextCompareFinished {
                generation,
                display_path: "text.txt".to_owned(),
                renderer: RendererKind::Builtin,
                layout: LayoutMode::Unified,
                output: CompareOutput::default(),
            },
        )));

        assert!(state.workspace.compare_output.get(&state.store).is_none());
        assert_eq!(state.text_compare.view, TextCompareView::Edit);
    }

    #[test]
    fn text_compare_finished_installs_diff_view() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);
        let generation = state.text_compare.generation.saturating_add(1);
        state.text_compare.generation = generation;
        let output = crate::core::compare::compare_text(
            "old\n",
            "new\n",
            "text.txt",
            RendererKind::Builtin,
            LayoutMode::Unified,
        )
        .unwrap();

        state.apply_event(AppEvent::from(CompareEvent::TextCompareFinished(
            TextCompareFinished {
                generation,
                display_path: "text.txt".to_owned(),
                renderer: RendererKind::Builtin,
                layout: LayoutMode::Unified,
                output,
            },
        )));

        assert_eq!(state.text_compare.view, TextCompareView::Diff);
        assert_eq!(
            state.workspace.source.get(&state.store),
            WorkspaceSource::TextCompare
        );
        assert!(state.workspace.active_file.get(&state.store).is_some());
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
        let (left_ref, right_ref) =
            crate::ui::vcs::profile(None).status_compare_refs(ChangeBucket::Unstaged);

        state.compare.repo_path.set(&state.store, Some(repo_path));
        state
            .repository
            .capabilities
            .set(&state.store, Some(RepoCapabilities::git()));
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
                path: path.as_str().into(),
            }],
        );
        state.workspace.status_file_changes.set(
            &state.store,
            vec![FileChange {
                path: path.clone(),
                old_path: None,
                status: FileChangeStatus::Modified,
                bucket: ChangeBucket::Unstaged,
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
            .selected_change_bucket
            .set(&state.store, Some(ChangeBucket::Unstaged));
        state.workspace.active_file.set(
            &state.store,
            Some(ActiveFile {
                index: 0,
                path,
                carbon_file: Arc::new(carbon_file.clone()),
                carbon_expansion,
                carbon_overlays: CarbonStyleOverlays::default(),
                render_doc: Arc::new(render_doc),
                token_buffer,
                left_ref,
                right_ref,
                file_line_count: None,
                old_file_lines: None,
                file_lines: None,
                syntax_pending: Vec::new(),
                syntax_covered: Vec::new(),
                last_used_tick: 0,
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
                path: file.path().into(),
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
                },
                FileListEntry {
                    path: "b.rs".into(),
                },
                FileListEntry {
                    path: "c.rs".into(),
                },
                FileListEntry {
                    path: "d.rs".into(),
                },
                FileListEntry {
                    path: "e.rs".into(),
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
    fn file_height_index_keeps_uniform_large_lists_sparse() {
        let mut index = FileHeightIndex::default();
        index.rebuild(vec![192; FILE_HEIGHT_SPARSE_MIN_COUNT + 1]);

        assert_eq!(index.len(), FILE_HEIGHT_SPARSE_MIN_COUNT + 1);
        assert_eq!(
            index.total_u32(),
            ((FILE_HEIGHT_SPARSE_MIN_COUNT + 1) as u32) * 192
        );
        assert!(matches!(index, FileHeightIndex::Sparse { .. }));
        assert_eq!(index.locate(192 * 7 + 12), Some((7, 12)));
    }

    #[test]
    fn sparse_file_height_index_updates_prefix_and_locate() {
        let mut index = FileHeightIndex::default();
        index.rebuild(vec![100; FILE_HEIGHT_SPARSE_MIN_COUNT + 2]);
        index.update(3, 250);
        index.update(7, 40);

        assert_eq!(index.prefix_u32(4), 550);
        assert_eq!(index.prefix_u32(8), 890);
        assert_eq!(index.locate(549), Some((3, 249)));
        assert_eq!(index.locate(550), Some((4, 0)));
        assert_eq!(index.locate(849), Some((6, 99)));
        assert_eq!(index.locate(850), Some((7, 0)));
        assert_eq!(index.locate(889), Some((7, 39)));
        assert_eq!(index.locate(890), Some((8, 0)));
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
    fn next_file_action_selects_adjacent_file_in_single_file_mode() {
        let mut state =
            loaded_state_with_files(&["src/ui/state/mod.rs", "src/ui/state/text_edit.rs"]);
        state.apply_action(crate::actions::FileListAction::SelectFile(0));

        state.apply_action(crate::actions::EditorAction::GoToNextFile);
        state.sync_editor_scroll_from_global();

        assert_eq!(
            state.workspace.selected_file_index.get(&state.store),
            Some(1)
        );
        assert_eq!(
            state
                .workspace
                .selected_file_path
                .get(&state.store)
                .as_deref(),
            Some("src/ui/state/text_edit.rs")
        );
        assert_eq!(
            state
                .workspace
                .active_file
                .get(&state.store)
                .as_ref()
                .map(|file| file.path.as_str()),
            Some("src/ui/state/text_edit.rs")
        );
        assert_eq!(state.workspace.global_scroll_top_px.get(&state.store), 0);
    }

    #[test]
    fn next_file_action_selects_next_file_when_tail_is_short() {
        let mut state =
            loaded_state_with_files(&["src/ui/state/mod.rs", "src/ui/state/text_edit.rs"]);
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 10_000);
        state.apply_action(crate::actions::FileListAction::SelectFile(0));

        state.apply_action(crate::actions::EditorAction::GoToNextFile);

        assert_eq!(
            state.workspace.selected_file_index.get(&state.store),
            Some(1)
        );
        assert_eq!(
            state
                .workspace
                .selected_file_path
                .get(&state.store)
                .as_deref(),
            Some("src/ui/state/text_edit.rs")
        );
        assert_eq!(
            state
                .workspace
                .active_file
                .get(&state.store)
                .as_ref()
                .map(|file| file.path.as_str()),
            Some("src/ui/state/text_edit.rs")
        );
    }

    #[test]
    fn continuous_scroll_keeps_short_tail_at_natural_bottom() {
        let state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.editor.viewport_height_px.set(&state.store, 10_000);

        assert_eq!(state.global_max_scroll_top_px(), 0);
    }

    #[test]
    fn continuous_scroll_first_height_measurement_keeps_total_cache_in_sync_with_index() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.settings.continuous_scroll = true;
        state.recompute_file_scroll_total_height_px();

        assert_eq!(state.workspace.measured_px_per_row_q16.get(&state.store), 0);

        assert!(state.update_file_content_height_px(0, 1_200));

        assert_eq!(
            state
                .workspace
                .file_scroll_total_height_px
                .get(&state.store),
            state.virtual_diff_document.total_u32()
        );
    }

    #[test]
    fn continuous_scroll_keeps_bottom_anchor_when_visible_file_height_grows() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 100);
        state
            .workspace
            .file_content_heights
            .set(&state.store, vec![Some(200), Some(200), Some(200)]);
        state.recompute_file_scroll_total_height_px();

        let old_max = state.global_max_scroll_top_px();
        assert_eq!(old_max, 500);
        state
            .workspace
            .global_scroll_top_px
            .set(&state.store, old_max);

        assert!(state.update_file_content_height_px(2, 350));

        assert_eq!(state.global_max_scroll_top_px(), 650);
        assert_eq!(
            state.workspace.global_scroll_top_px.get(&state.store),
            state.global_max_scroll_top_px()
        );
    }

    #[test]
    fn continuous_scroll_follow_end_anchor_is_explicit_after_scrolling_to_bottom() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 100);
        state
            .workspace
            .file_content_heights
            .set(&state.store, vec![Some(200), Some(200), Some(200)]);
        state.recompute_file_scroll_total_height_px();

        let old_max = state.global_max_scroll_top_px();
        state.scroll_viewport_to_global(old_max);

        let anchor = state.virtual_scroll.anchor.expect("bottom anchor");
        assert_eq!(anchor.bias, ViewportAnchorBias::FollowEnd);

        assert!(state.update_file_content_height_px(2, 350));

        assert_eq!(
            state.workspace.global_scroll_top_px.get(&state.store),
            state.global_max_scroll_top_px()
        );
    }

    #[test]
    fn continuous_scroll_preserves_top_anchor_when_prior_file_height_changes() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 100);
        state
            .workspace
            .file_content_heights
            .set(&state.store, vec![Some(200), Some(200), Some(200)]);
        state.recompute_file_scroll_total_height_px();

        state.scroll_viewport_to_global(250);
        let anchor = state.virtual_scroll.anchor.expect("top anchor");
        assert_eq!(anchor.item_id.index, 1);
        assert_eq!(anchor.intra_item_offset_px, 50);
        assert_eq!(anchor.bias, ViewportAnchorBias::PreserveTop);

        assert!(state.update_file_content_height_px(0, 300));

        assert_eq!(state.workspace.global_scroll_top_px.get(&state.store), 350);
        let anchor = state.virtual_scroll.anchor.expect("rebased anchor");
        assert_eq!(anchor.item_id.index, 1);
        assert_eq!(anchor.intra_item_offset_px, 50);
    }

    #[test]
    fn continuous_scroll_preserves_bottom_anchor_when_prior_file_height_changes() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 100);
        state
            .workspace
            .file_content_heights
            .set(&state.store, vec![Some(200), Some(200), Some(200)]);
        state.recompute_file_scroll_total_height_px();

        state.set_viewport_anchor_for_global(350, ViewportAnchorBias::PreserveBottom);
        let anchor = state.virtual_scroll.anchor.expect("bottom-edge anchor");
        assert_eq!(anchor.item_id.index, 2);
        assert_eq!(anchor.intra_item_offset_px, 50);

        assert!(state.update_file_content_height_px(0, 300));

        assert_eq!(state.workspace.global_scroll_top_px.get(&state.store), 450);
        let anchor = state.virtual_scroll.anchor.expect("rebased anchor");
        assert_eq!(anchor.bias, ViewportAnchorBias::PreserveBottom);
        assert_eq!(anchor.item_id.index, 2);
        assert_eq!(anchor.intra_item_offset_px, 50);
    }

    #[test]
    fn continuous_scroll_keeps_bottom_anchor_after_pending_scrollbar_drag_height_update() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 100);
        state
            .workspace
            .file_content_heights
            .set(&state.store, vec![Some(200), Some(200), Some(200)]);
        state.recompute_file_scroll_total_height_px();

        let old_max = state.global_max_scroll_top_px();
        assert_eq!(old_max, 500);
        state
            .workspace
            .global_scroll_top_px
            .set(&state.store, old_max);
        state.begin_viewport_scrollbar_drag(600, 100, old_max, old_max);

        assert!(!state.update_file_content_height_px(2, 350));
        state.end_viewport_scrollbar_drag();

        assert_eq!(state.global_max_scroll_top_px(), 650);
        assert_eq!(
            state.workspace.global_scroll_top_px.get(&state.store),
            state.global_max_scroll_top_px()
        );
    }

    #[test]
    fn continuous_scroll_does_not_treat_zero_max_as_bottom_anchor() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 1_000);
        state
            .workspace
            .file_content_heights
            .set(&state.store, vec![Some(200), Some(200), Some(200)]);
        state.recompute_file_scroll_total_height_px();

        assert_eq!(state.global_max_scroll_top_px(), 0);
        assert!(state.update_file_content_height_px(2, 700));

        assert_eq!(state.global_max_scroll_top_px(), 100);
        assert_eq!(state.workspace.global_scroll_top_px.get(&state.store), 0);
    }

    #[test]
    fn virtual_diff_document_keeps_large_compare_ranges_sparse_and_anchorable() {
        let count = FILE_HEIGHT_SPARSE_MIN_COUNT + 32;
        let summaries = (0..count)
            .map(|index| {
                let path = format!("kernel/file_{index}.c");
                CompareFileSummary::from_paths_status(
                    Some(&path),
                    Some(&path),
                    carbon::FileStatus::Modified,
                    true,
                )
            })
            .collect::<Vec<_>>();
        let mut state = AppState::default();
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 900);
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                file_summaries: summaries,
                ..CompareOutput::default()
            }),
        );
        state.recompute_file_scroll_total_height_px();

        assert!(matches!(
            state.virtual_diff_document.height_index,
            FileHeightIndex::Sparse { .. }
        ));

        let target = state.global_max_scroll_top_px() / 2;
        state.scroll_viewport_to_global(target);
        let anchor = state.virtual_scroll.anchor.expect("compare anchor");

        assert_eq!(anchor.item_id.source, WorkspaceSource::Compare);
        assert_eq!(
            anchor.item_id.generation,
            state.workspace_render_generation()
        );
        assert!(anchor.item_id.index < count);
    }

    #[test]
    fn virtual_diff_document_rejects_stale_measurement_item_ids() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs"]);
        state.settings.continuous_scroll = true;
        state.recompute_file_scroll_total_height_px();
        let item_id = state.virtual_diff_document.item_id(1).expect("item id");

        assert!(state.update_virtual_diff_item_height_px(item_id, 300));
        state.workspace.compare_generation.set(&state.store, 1);

        assert!(!state.update_virtual_diff_item_height_px(item_id, 500));
        assert_eq!(
            state
                .workspace
                .file_content_heights
                .with(&state.store, |heights| heights.get(1).copied().flatten()),
            Some(300)
        );
    }

    #[test]
    fn continuous_compare_count_keeps_sidebar_files_when_output_is_partially_hydrated() {
        let mut state = AppState::default();
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 10_000);
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                carbon: carbon::DiffDocument {
                    files: vec![carbon_context_file(0, "a.rs", "loaded")],
                },
                ..CompareOutput::default()
            }),
        );
        state.workspace.files.set(
            &state.store,
            vec![
                FileListEntry {
                    path: "a.rs".into(),
                },
                FileListEntry {
                    path: "b.rs".into(),
                },
                FileListEntry {
                    path: "c.rs".into(),
                },
            ],
        );

        assert_eq!(state.workspace_file_count(), 3);

        let (doc, _effects) = state.build_continuous_viewport_document();
        let doc = doc.expect("viewport doc");

        assert_eq!(doc.slot_indices, vec![0, 1, 2]);
        assert_eq!(doc.slot_loading, vec![false, true, true]);
    }

    #[test]
    fn continuous_viewport_document_exposes_virtual_stream_rows() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs"]);
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 900);
        state
            .cache_compare_file_from_output(0, "a.rs")
            .expect("cached first file");
        state
            .cache_compare_file_from_output(1, "b.rs")
            .expect("cached second file");

        let (doc, _effects) = state.build_continuous_viewport_document();
        let doc = doc.expect("viewport doc");

        assert!(
            doc.stream_items
                .iter()
                .any(|item| item.id.kind == VirtualDiffItemKind::FileHeader)
        );
        assert!(
            doc.stream_items
                .iter()
                .any(|item| item.id.kind == VirtualDiffItemKind::Hunk)
        );
        assert!(
            doc.stream_items
                .iter()
                .any(|item| item.id.kind == VirtualDiffItemKind::DiffRow)
        );
        assert!(
            doc.stream_items
                .windows(2)
                .all(|items| items[0].sort_key <= items[1].sort_key)
        );
        assert!(
            doc.stream_items
                .iter()
                .all(|item| item.estimated_height_px > 0)
        );
    }

    #[test]
    fn continuous_viewport_document_backfills_before_tail_file() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 500);
        state
            .workspace
            .file_content_heights
            .set(&state.store, vec![Some(800), Some(800), Some(800)]);
        state.recompute_file_scroll_total_height_px();
        state
            .workspace
            .global_scroll_top_px
            .set(&state.store, 1_700);

        let (doc, _effects) = state.build_continuous_viewport_document();
        let doc = doc.expect("viewport doc");

        assert_eq!(doc.slot_indices, vec![1, 2]);
        assert_eq!(doc.start_index, 1);
        assert_eq!(doc.start_offset_px, 800);
        assert_eq!(doc.scroll_top_px, 900);
    }

    #[test]
    fn continuous_viewport_document_follow_end_builds_from_tail() {
        let mut state =
            loaded_state_with_files(&["a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs", "tail.rs"]);
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 500);
        state.workspace.file_content_heights.set(
            &state.store,
            vec![
                Some(1_000),
                Some(1_000),
                Some(1_000),
                Some(1_000),
                Some(1_000),
                Some(1_000),
                Some(100),
            ],
        );
        state.recompute_file_scroll_total_height_px();

        state.scroll_viewport_to_global(state.global_max_scroll_top_px());

        let (doc, _effects) = state.build_continuous_viewport_document();
        let doc = doc.expect("viewport doc");

        assert_eq!(doc.slot_indices, vec![5, 6]);
        assert_eq!(doc.start_index, 5);
        assert_eq!(doc.start_offset_px, 5_000);
        assert_eq!(doc.scroll_top_px, 600);
    }

    #[test]
    fn next_file_action_resolves_current_file_from_selected_path() {
        let mut state = loaded_state_with_files(&[
            "src/core/compare/backends/git_diff.rs",
            "src/core/compare/mod.rs",
            "src/core/compare/service.rs",
            "src/core/compare/stats.rs",
            "src/core/frecency.rs",
            "src/ui/state/mod.rs",
            "src/ui/state/text_edit.rs",
            "src/ui/toolbar.rs",
        ]);
        state.settings.continuous_scroll = true;
        state
            .workspace
            .selected_file_index
            .set(&state.store, Some(0));
        state
            .workspace
            .selected_file_path
            .set(&state.store, Some("src/ui/state/mod.rs".to_owned()));

        state.apply_action(crate::actions::EditorAction::GoToNextFile);

        assert_eq!(
            state.workspace.selected_file_index.get(&state.store),
            Some(6)
        );
        assert_eq!(
            state
                .workspace
                .selected_file_path
                .get(&state.store)
                .as_deref(),
            Some("src/ui/state/text_edit.rs")
        );
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
                path: "src/lib.rs".into(),
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
                path: "src/lib.rs".into(),
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
        state
            .workspace
            .compare_output
            .update(&state.store, |output| {
                let file = &mut output.as_mut().expect("compare output").carbon.files[0];
                file.additions = 1_500;
            });
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
                priority: CompareWorkPriority::InteractiveSelectedFile,
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
                        && task.request.deferred_file.as_ref().is_some_and(|file| file.is_partial)
            )
        }));
        assert_eq!(
            state.workspace.active_file_loading.get(&state.store),
            Some(ActiveFileLoading {
                index: 0,
                path: "src/kernel.c".to_owned(),
                priority: CompareWorkPriority::InteractiveSelectedFile,
            })
        );
        assert!(state.workspace.active_file.get(&state.store).is_none());
    }

    #[test]
    fn scrollbar_drag_loads_visible_compare_files_without_selecting_them() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state.settings.continuous_scroll = true;
        state.editor.viewport_height_px.set(&state.store, 240);
        state
            .compare
            .repo_path
            .set(&state.store, Some(PathBuf::from("/repo")));
        state
            .workspace
            .compare_output
            .update(&state.store, |output| {
                for file in &mut output.as_mut().expect("compare output").carbon.files {
                    file.is_partial = true;
                    file.hunks.clear();
                    file.blocks.clear();
                }
            });
        state.begin_viewport_scrollbar_drag(900, 240, 300, 660);

        let (_doc, effects) = state.build_continuous_viewport_document();

        assert!(
            effects
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
    }

    #[test]
    fn overscan_prefetch_does_not_enqueue_syntax_work() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
        state
            .compare
            .repo_path
            .set(&state.store, Some(PathBuf::from("/repo")));

        let effects = state.prefetch_compare_files_forward(0, 1_000);

        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Syntax(SyntaxEffect::LoadFileSyntax(_)))),
            "overscan should warm file diffs without adding syntax windows"
        );
        state.workspace.file_cache.with(&state.store, |files| {
            assert!(files.values().all(|file| file.syntax_pending.is_empty()));
        });
    }

    #[test]
    fn offscreen_viewport_slots_do_not_enqueue_syntax_work() {
        let mut state = loaded_state_with_files(&["a.rs"]);
        state
            .cache_compare_file_from_output(0, "a.rs")
            .expect("cached file");
        let key = state.compare_slot_key_at(0, "a.rs");

        let window = state.viewport_slot_syntax_window(&key, 1_000, 120, 0, 240);

        assert_eq!(window, None);
    }

    #[test]
    fn syntax_budget_counts_inflight_requests_after_cache_eviction() {
        let mut state = loaded_state_with_files(&["a.rs"]);
        state
            .compare
            .repo_path
            .set(&state.store, Some(PathBuf::from("/repo")));
        state
            .cache_compare_file_from_output(0, "a.rs")
            .expect("cached file");
        let key = state.compare_slot_key_at(0, "a.rs");

        let effect = state.request_viewport_slot_syntax_window(
            &key,
            crate::core::syntax::annotator::SyntaxRowWindow { start: 0, end: 32 },
        );

        assert!(matches!(
            effect,
            Some(Effect::Syntax(SyntaxEffect::LoadFileSyntax(_)))
        ));
        state.workspace.file_cache.update(&state.store, |files| {
            files.clear();
        });
        assert_eq!(state.syntax_pending_window_count(), 0);
        assert_eq!(state.syntax_requests.inflight_len(), 1);
    }

    #[test]
    fn syntax_epoch_invalidation_clears_attached_pending_windows() {
        let mut state = loaded_state_with_files(&["a.rs", "b.rs"]);
        state
            .cache_compare_file_from_output(0, "a.rs")
            .expect("cached file");
        state
            .cache_compare_file_from_output(1, "b.rs")
            .expect("cached file");
        let active = state
            .workspace
            .file_cache
            .with(&state.store, |files| files.get(&0).cloned())
            .expect("cached active");
        state.workspace.active_file.set(&state.store, Some(active));
        let pending = super::SyntaxPendingWindow {
            request_id: 1,
            window: crate::core::syntax::annotator::SyntaxRowWindow { start: 0, end: 32 },
        };
        state.workspace.active_file.update(&state.store, |active| {
            active
                .as_mut()
                .expect("active file")
                .syntax_pending
                .push(pending);
        });
        state.workspace.file_cache.update(&state.store, |files| {
            for file in files.values_mut() {
                file.syntax_pending.push(pending);
            }
        });
        state.syntax_requests.insert_inflight(0, 1);

        let effect = state.invalidate_syntax_epoch_effect();

        assert!(matches!(
            effect,
            Effect::Syntax(SyntaxEffect::SetFileSyntaxEpoch { .. })
        ));
        assert_eq!(state.syntax_pending_window_count(), 0);
        assert_eq!(state.syntax_requests.inflight_len(), 0);
    }

    #[test]
    fn context_expansion_invalidates_existing_syntax_windows() {
        let mut state = status_state_with_two_hunks();
        let stale_window = crate::core::syntax::annotator::SyntaxRowWindow { start: 0, end: 8 };

        state.workspace.active_file.update(&state.store, |active| {
            let active = active.as_mut().expect("active file");
            active.syntax_pending.push(super::SyntaxPendingWindow {
                request_id: 7,
                window: stale_window,
            });
            active.syntax_covered.push(stale_window);
            let range = active
                .token_buffer
                .append(&[crate::core::text::DiffTokenSpan {
                    offset: 0,
                    length: 2,
                    kind: Default::default(),
                    intensity: Default::default(),
                }]);
            active
                .carbon_overlays
                .insert_syntax(0, carbon::DiffSide::Old, 0, range);
        });

        state.apply_context_expansion(
            crate::events::ContextDirection::All,
            0,
            0,
            Arc::new((0..12).map(|index| format!("old {index}")).collect()),
            Arc::new((0..12).map(|index| format!("new {index}")).collect()),
        );

        state.workspace.active_file.with(&state.store, |active| {
            let active = active.as_ref().expect("active file");
            assert!(active.syntax_pending.is_empty());
            assert!(active.syntax_covered.is_empty());
            assert_eq!(active.token_buffer.len(), 0);
        });
    }

    #[test]
    fn context_expansion_retires_old_syntax_epoch_before_requeue() {
        let mut state = status_state_with_two_hunks();
        state.workspace.active_file.update(&state.store, |active| {
            let active = active.as_mut().expect("active file");
            active.old_file_lines = Some(Arc::new(
                (0..12).map(|index| format!("old {index}")).collect(),
            ));
            active.file_lines = Some(Arc::new(
                (0..12).map(|index| format!("new {index}")).collect(),
            ));
        });
        state.workspace.compare_generation.set(&state.store, 1);
        for request_id in 0..super::MAX_PENDING_SYNTAX_WINDOWS as u64 {
            state.syntax_requests.insert_inflight(0, request_id);
        }

        let effects = state.dispatch_context_expansion(0, crate::events::ContextDirection::All, 0);

        assert!(matches!(
            effects.first(),
            Some(Effect::Syntax(SyntaxEffect::SetFileSyntaxEpoch { .. }))
        ));
        assert!(effects.iter().any(|effect| {
            matches!(
                effect,
                Effect::Syntax(SyntaxEffect::LoadFileSyntax(task))
                    if task.request.syntax_epoch == state.syntax_requests.epoch()
            )
        }));
        assert_eq!(state.syntax_requests.inflight_len(), 1);
    }

    #[test]
    fn syntax_pack_install_retires_old_epoch_before_refresh() {
        let mut state = status_state_with_two_hunks();
        for request_id in 0..super::MAX_PENDING_SYNTAX_WINDOWS as u64 {
            state.syntax_requests.insert_inflight(0, request_id);
        }

        let effects = state.handle_syntax_packs_installed(&["rust".to_owned()]);

        assert!(matches!(
            effects.first(),
            Some(Effect::Syntax(SyntaxEffect::SetFileSyntaxEpoch { .. }))
        ));
        assert!(effects.iter().any(|effect| {
            matches!(
                effect,
                Effect::Syntax(SyntaxEffect::LoadFileSyntax(task))
                    if task.request.syntax_epoch == state.syntax_requests.epoch()
            )
        }));
        assert_eq!(state.syntax_requests.inflight_len(), 1);
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
                priority: CompareWorkPriority::InteractiveSelectedFile,
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
                    render_doc: Arc::new(RenderDoc::default()),
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
                priority: CompareWorkPriority::InteractiveSelectedFile,
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
                label_style: PickerLabelStyle::Default,
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
    fn stage_hunk_without_partial_hunk_capability_is_ignored() {
        let mut state = status_state_with_two_hunks();
        let mut capabilities = RepoCapabilities::git();
        capabilities.staging_area = false;
        capabilities.partial_hunk_mutation = false;
        state
            .repository
            .capabilities
            .set(&state.store, Some(capabilities));

        let effects = state.apply_action(crate::actions::RepositoryAction::StageHunkAt(0));

        assert!(effects.is_empty());
    }

    #[test]
    fn status_operation_failure_clears_the_pending_flag() {
        let mut state = status_state_with_two_hunks();
        let _ = state.apply_action(crate::actions::RepositoryAction::StageHunkAt(0));
        assert!(state.workspace.status_operation_pending.get(&state.store));

        let _ = state.apply_event(AppEvent::from(RepositoryEvent::FileOperationFailed {
            path: PathBuf::from("/repo"),
            message: "patch failed".to_owned(),
        }));

        assert!(!state.workspace.status_operation_pending.get(&state.store));
    }

    #[test]
    fn ref_picker_rebuilds_matches_while_typing_and_keeps_raw_git_revisions_selectable() {
        let mut state = AppState::default();
        state.repository.refs.set(
            &state.store,
            vec![VcsRef {
                name: "main".to_owned(),
                kind: RefKind::Branch,
                target: RevisionId::git("0000000000000000000000000000000000000000"),
                active: true,
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
    fn ref_picker_uses_jj_refs_and_change_ids_without_git_workdir() {
        let mut state = AppState::default();
        let working_commit = "3e2d7a6e55221e519e3efb86e4f8fbb324980427".to_owned();
        let change_id = "xxyzvpwmsuxytmqltlzwzqpylvlqqyso".to_owned();

        state.repository.location.set(
            &state.store,
            Some(RepoLocation {
                kind: VcsKind::JJ,
                profile: crate::core::vcs::model::VCS_PROFILE_JJ,
                workspace_root: PathBuf::from("/repo"),
                store_root: Some(PathBuf::from("/repo/.jj")),
            }),
        );
        state.repository.refs.set(
            &state.store,
            vec![
                VcsRef {
                    name: "@".to_owned(),
                    kind: RefKind::WorkingCopy,
                    target: RevisionId {
                        backend: VcsKind::JJ,
                        id: working_commit.clone(),
                    },
                    active: true,
                    upstream: None,
                    ahead_behind: None,
                },
                VcsRef {
                    name: "main".to_owned(),
                    kind: RefKind::Bookmark,
                    target: RevisionId {
                        backend: VcsKind::JJ,
                        id: "a4c9f6e8b1d24036a78610a332e12ca25e97c315".to_owned(),
                    },
                    active: false,
                    upstream: None,
                    ahead_behind: None,
                },
            ],
        );
        state.repository.changes.set(
            &state.store,
            vec![VcsChange {
                revision: RevisionId {
                    backend: VcsKind::JJ,
                    id: working_commit,
                },
                change_id: Some(change_id.clone()),
                short_change_id: Some("xsvsonvs".to_owned()),
                short_change_id_prefix_len: Some(2),
                short_revision: "3e2d7a6e5522".to_owned(),
                summary: "Working copy".to_owned(),
                author_name: "ro".to_owned(),
                timestamp: 0,
                flags: ChangeFlags {
                    current: true,
                    working_copy: true,
                    ..ChangeFlags::default()
                },
            }],
        );

        state.open_ref_picker(CompareField::Left);

        state.overlays.picker.entries.with(&state.store, |entries| {
            assert!(!entries.iter().any(|entry| entry.value == "@workdir"));

            let working_copy = entries
                .iter()
                .find(|entry| entry.value == "@")
                .expect("working copy ref");
            assert_eq!(
                working_copy.detail,
                "Working copy change \u{2022} current / xsvsonvs 3e2d7a6e5522"
            );

            let bookmark = entries
                .iter()
                .find(|entry| entry.value == "main")
                .expect("bookmark ref");
            assert_eq!(bookmark.detail, "Bookmark");

            let change = entries
                .iter()
                .find(|entry| entry.value == change_id)
                .expect("change id entry");
            assert_eq!(change.label, "xsvsonvs");
            assert!(change.highlights.is_empty());
            assert_eq!(
                change.label_style(),
                PickerLabelStyle::JjChangeId {
                    prefix_len: 2,
                    working_copy: true,
                }
            );
        });
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
    fn command_palette_surfaces_jj_operations_for_jj_repositories() {
        let mut state = AppState::default();
        state.repository.location.set(
            &state.store,
            Some(RepoLocation {
                kind: VcsKind::JJ,
                profile: crate::core::vcs::model::VCS_PROFILE_JJ,
                workspace_root: PathBuf::from("/repo"),
                store_root: Some(PathBuf::from("/repo/.jj")),
            }),
        );
        state
            .overlays
            .command_palette
            .query
            .set(&state.store, "jj".to_owned());

        state.rebuild_command_palette();

        let entries = state
            .overlays
            .command_palette
            .entries
            .with(&state.store, |entries| entries.clone());
        for operation in JjOperation::ALL {
            let label = format!("jj: {}", operation.label());
            let entry = entries
                .iter()
                .find(|entry| entry.label == label)
                .unwrap_or_else(|| panic!("missing {label} command"));
            assert!(matches!(
                entry.kind,
                super::PaletteEntryKind::Command(super::PaletteCommand::RunOperation(
                    VcsOperation::Jj(found)
                )) if found == operation
            ));
        }

        let mut state = AppState::default();
        state
            .overlays
            .command_palette
            .query
            .set(&state.store, "jj".to_owned());
        state.rebuild_command_palette();
        let has_jj_operation =
            state
                .overlays
                .command_palette
                .entries
                .with(&state.store, |entries| {
                    entries.iter().any(|entry| {
                        JjOperation::ALL
                            .into_iter()
                            .any(|operation| entry.label == format!("jj: {}", operation.label()))
                    })
                });
        assert!(!has_jj_operation);
    }

    #[test]
    fn jj_operation_action_emits_repository_effect() {
        let mut state = AppState::default();
        let repo_path = PathBuf::from("/repo");
        let operation = VcsOperation::Jj(JjOperation::NewChange);
        state
            .compare
            .repo_path
            .set(&state.store, Some(repo_path.clone()));
        state.repository.location.set(
            &state.store,
            Some(RepoLocation {
                kind: VcsKind::JJ,
                profile: crate::core::vcs::model::VCS_PROFILE_JJ,
                workspace_root: repo_path.clone(),
                store_root: Some(repo_path.join(".jj")),
            }),
        );

        let effects = state.apply_action(crate::actions::RepositoryAction::RunOperation(
            operation.clone(),
        ));

        let [Effect::Repository(RepositoryEffect::RunOperation(request))] = effects.as_slice()
        else {
            panic!("expected RunOperation effect, got {effects:?}");
        };
        assert_eq!(request.repo_path, repo_path);
        assert_eq!(request.operation, operation);
    }

    #[test]
    fn destructive_jj_palette_operation_requires_confirmation() {
        let mut state = AppState::default();
        let repo_path = PathBuf::from("/repo");
        let operation = VcsOperation::Jj(JjOperation::AbandonChange);
        state
            .compare
            .repo_path
            .set(&state.store, Some(repo_path.clone()));
        state.repository.location.set(
            &state.store,
            Some(RepoLocation {
                kind: VcsKind::JJ,
                profile: crate::core::vcs::model::VCS_PROFILE_JJ,
                workspace_root: repo_path.clone(),
                store_root: Some(repo_path.join(".jj")),
            }),
        );
        state
            .overlays
            .command_palette
            .query
            .set(&state.store, "abandon".to_owned());
        state.overlays.stack.update(&state.store, |stack| {
            stack.push(OverlayEntry {
                surface: OverlaySurface::CommandPalette,
                focus_return: None,
            });
        });
        state.rebuild_command_palette();

        let effects = state.apply_action(crate::actions::OverlayAction::ConfirmOverlaySelection);

        assert!(effects.is_empty());
        assert_eq!(state.overlays_top(), Some(OverlaySurface::Confirmation));
        assert_eq!(
            state.overlays.confirmation.action.get(&state.store),
            Some(crate::actions::RepositoryAction::RunOperation(operation.clone()).into())
        );

        let effects = state.apply_action(crate::actions::OverlayAction::ConfirmOverlaySelection);

        let [Effect::Repository(RepositoryEffect::RunOperation(request))] = effects.as_slice()
        else {
            panic!("expected RunOperation effect, got {effects:?}");
        };
        assert_eq!(request.repo_path, repo_path);
        assert_eq!(request.operation, operation);
        assert_eq!(state.overlays_top(), None);
    }

    #[test]
    fn command_palette_surfaces_jj_rebase_destinations() {
        let mut state = AppState::default();
        state.repository.location.set(
            &state.store,
            Some(RepoLocation {
                kind: VcsKind::JJ,
                profile: crate::core::vcs::model::VCS_PROFILE_JJ,
                workspace_root: PathBuf::from("/repo"),
                store_root: Some(PathBuf::from("/repo/.jj")),
            }),
        );
        state.repository.refs.set(
            &state.store,
            vec![
                VcsRef {
                    name: "@".to_owned(),
                    kind: RefKind::WorkingCopy,
                    target: RevisionId {
                        backend: VcsKind::JJ,
                        id: "current".to_owned(),
                    },
                    active: true,
                    upstream: None,
                    ahead_behind: None,
                },
                VcsRef {
                    name: "main".to_owned(),
                    kind: RefKind::Bookmark,
                    target: RevisionId {
                        backend: VcsKind::JJ,
                        id: "main-revision".to_owned(),
                    },
                    active: false,
                    upstream: None,
                    ahead_behind: None,
                },
            ],
        );
        state
            .overlays
            .command_palette
            .query
            .set(&state.store, "rebase main".to_owned());

        state.rebuild_command_palette();

        let entry = state
            .overlays
            .command_palette
            .entries
            .with(&state.store, |entries| entries.first().cloned())
            .expect("rebase entry");
        assert_eq!(entry.label, "jj: Rebase @ Onto main");
        assert!(matches!(
            entry.kind,
            super::PaletteEntryKind::Command(super::PaletteCommand::RunOperation(
                VcsOperation::JjRebaseCurrentChangeOnto { ref destination }
            )) if destination == "main"
        ));
    }

    #[test]
    fn command_palette_surfaces_jj_editable_changes() {
        let mut state = AppState::default();
        state.repository.location.set(
            &state.store,
            Some(RepoLocation {
                kind: VcsKind::JJ,
                profile: crate::core::vcs::model::VCS_PROFILE_JJ,
                workspace_root: PathBuf::from("/repo"),
                store_root: Some(PathBuf::from("/repo/.jj")),
            }),
        );
        state.repository.changes.set(
            &state.store,
            vec![
                VcsChange {
                    revision: RevisionId {
                        backend: VcsKind::JJ,
                        id: "current-revision".to_owned(),
                    },
                    change_id: Some("current-change".to_owned()),
                    short_change_id: Some("cur".to_owned()),
                    short_change_id_prefix_len: Some(3),
                    short_revision: "currev".to_owned(),
                    summary: "current".to_owned(),
                    author_name: "ro".to_owned(),
                    timestamp: 0,
                    flags: ChangeFlags {
                        current: true,
                        working_copy: true,
                        ..ChangeFlags::default()
                    },
                },
                VcsChange {
                    revision: RevisionId {
                        backend: VcsKind::JJ,
                        id: "target-revision".to_owned(),
                    },
                    change_id: Some("target-change".to_owned()),
                    short_change_id: Some("tgt".to_owned()),
                    short_change_id_prefix_len: Some(3),
                    short_revision: "tgt123".to_owned(),
                    summary: "target change".to_owned(),
                    author_name: "ro".to_owned(),
                    timestamp: 0,
                    flags: ChangeFlags::default(),
                },
            ],
        );
        state
            .overlays
            .command_palette
            .query
            .set(&state.store, "edit tgt".to_owned());

        state.rebuild_command_palette();

        let entry = state
            .overlays
            .command_palette
            .entries
            .with(&state.store, |entries| entries.first().cloned())
            .expect("edit entry");
        assert_eq!(entry.label, "jj: Edit tgt");
        assert!(matches!(
            entry.kind,
            super::PaletteEntryKind::Command(super::PaletteCommand::RunOperation(
                VcsOperation::JjEditRevision {
                    ref revision,
                    ref label
                }
            )) if revision == "target-revision" && label == "tgt"
        ));
    }

    #[test]
    fn command_palette_surfaces_jj_operation_log_restore_targets() {
        let mut state = AppState::default();
        state.repository.location.set(
            &state.store,
            Some(RepoLocation {
                kind: VcsKind::JJ,
                profile: crate::core::vcs::model::VCS_PROFILE_JJ,
                workspace_root: PathBuf::from("/repo"),
                store_root: Some(PathBuf::from("/repo/.jj")),
            }),
        );
        state.repository.operation_log.set(
            &state.store,
            vec![
                VcsOperationLogEntry {
                    operation_id: "current-operation".to_owned(),
                    short_operation_id: "current".to_owned(),
                    user: "ro".to_owned(),
                    time: "later".to_owned(),
                    description: "snapshot working copy".to_owned(),
                },
                VcsOperationLogEntry {
                    operation_id: "target-operation".to_owned(),
                    short_operation_id: "target".to_owned(),
                    user: "ro".to_owned(),
                    time: "earlier".to_owned(),
                    description: "describe change".to_owned(),
                },
            ],
        );
        state
            .overlays
            .command_palette
            .query
            .set(&state.store, "restore target".to_owned());

        state.rebuild_command_palette();

        let entries = state
            .overlays
            .command_palette
            .entries
            .with(&state.store, |entries| entries.clone());
        assert!(
            !entries
                .iter()
                .any(|entry| entry.label == "jj: Restore Operation current")
        );
        let entry = entries
            .iter()
            .find(|entry| entry.label == "jj: Restore Operation target")
            .expect("restore entry");
        assert_eq!(entry.detail, "describe change - ro - earlier");
        assert!(matches!(
            entry.kind,
            super::PaletteEntryKind::Command(super::PaletteCommand::RunOperation(
                VcsOperation::JjRestoreOperation {
                    ref operation_id,
                    ref label
                }
            )) if operation_id == "target-operation" && label == "target"
        ));
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
    fn card_text_selection_slices_normalized_range() {
        let body = "the quick brown fox".to_owned();
        // Forward selection.
        let mut sel = CardTextSelection::new(7, body.clone(), 4);
        sel.focus = 9;
        assert_eq!(sel.normalized(), (4, 9));
        assert_eq!(sel.selected_text().as_deref(), Some("quick"));
        assert!(!sel.is_collapsed());

        // Reversed drag yields the same substring.
        let mut rev = CardTextSelection::new(7, body.clone(), 9);
        rev.focus = 4;
        assert_eq!(rev.normalized(), (4, 9));
        assert_eq!(rev.selected_text().as_deref(), Some("quick"));

        // Collapsed selection copies nothing.
        let collapsed = CardTextSelection::new(7, body.clone(), 4);
        assert!(collapsed.is_collapsed());
        assert_eq!(collapsed.selected_text(), None);

        // Out-of-range anchor is clamped at construction (no panic / no copy).
        let clamped = CardTextSelection::new(7, body, 999);
        assert!(clamped.is_collapsed());
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
        use crate::core::forge::github::PullRequestInfo;
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
    fn kickoff_with_prior_state_reveals_loading_immediately() {
        let mut state = compare_ready_state();
        // Simulate a previously loaded compare (files present).
        state.workspace.files.set(
            &state.store,
            vec![FileListEntry {
                path: "old.rs".into(),
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
            progress.reveal_at_ms, 10_000,
            "compare loading should be visible immediately"
        );
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
    fn open_repository_resets_stale_compare_refs_before_snapshot() {
        let mut state = AppState::default();
        state.compare.left_ref.set(&state.store, "@-".to_owned());
        state.compare.right_ref.set(&state.store, "@".to_owned());
        state.compare.mode.set(&state.store, CompareMode::TwoDot);

        let path = PathBuf::from("/tmp/git-repo");
        let effects = state.open_repository(path.clone());

        assert_eq!(state.compare.left_ref.get(&state.store), "");
        assert_eq!(state.compare.right_ref.get(&state.store), "");
        assert_eq!(state.compare.mode.get(&state.store), CompareMode::default());
        let saved = effects.iter().find_map(|effect| match effect {
            Effect::Settings(SettingsEffect::SaveSettings(settings)) => {
                settings.last_compare.as_ref()
            }
            _ => None,
        });
        let saved = saved.expect("open_repository should persist settings");
        assert_eq!(saved.repo_path.as_ref(), Some(&path));
        assert_eq!(saved.left_ref, "");
        assert_eq!(saved.right_ref, "");
    }

    #[test]
    fn git_snapshot_after_jj_refs_uses_git_defaults() {
        let mut state = AppState::default();
        state.compare.left_ref.set(&state.store, "@-".to_owned());
        state.compare.right_ref.set(&state.store, "@".to_owned());
        state.compare.mode.set(&state.store, CompareMode::TwoDot);

        let path = PathBuf::from("/tmp/git-repo");
        let _ = state.open_repository(path.clone());
        state.apply_event(AppEvent::from(RepositoryEvent::RepositorySnapshotReady(
            crate::events::RepositorySnapshot::from_vcs_snapshot(
                crate::core::vcs::model::VcsSnapshot {
                    location: RepoLocation {
                        kind: VcsKind::GIT,
                        profile: crate::core::vcs::model::VCS_PROFILE_GIT,
                        workspace_root: path,
                        store_root: None,
                    },
                    reason: RepositorySyncReason::Open,
                    change_kind: None,
                    capabilities: RepoCapabilities::git(),
                    refs: Vec::new(),
                    changes: Vec::new(),
                    operation_log: Vec::new(),
                    file_changes: Vec::new(),
                },
            ),
        )));

        let (left, right, mode) =
            crate::ui::vcs::profile(state.repository.location.get(&state.store).as_ref())
                .default_compare();
        assert_eq!(state.compare.left_ref.get(&state.store), left);
        assert_eq!(state.compare.right_ref.get(&state.store), right);
        assert_eq!(state.compare.mode.get(&state.store), mode);
    }

    #[test]
    fn large_compare_stats_stream_offscreen_background_rows_after_visible_rows() {
        let state = compare_ready_state();
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state
            .compare
            .repo_path
            .set(&state.store, Some(PathBuf::from("/repo")));
        state.file_list.row_height.set(&state.store, 36.0);
        state.file_list.gap.set(&state.store, 4.0);
        state.file_list.viewport_height.set(&state.store, 80.0);

        let summaries = (0..=super::COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT)
            .map(|index| {
                let path = format!("src/file-{index}.rs");
                let mut summary = CompareFileSummary::from_paths_status(
                    Some(&path),
                    Some(&path),
                    carbon::FileStatus::Modified,
                    true,
                );
                if index < 128 {
                    summary.stats_deferred = false;
                }
                summary
            })
            .collect();
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                file_summaries: summaries,
                ..CompareOutput::default()
            }),
        );

        let effect = state
            .next_compare_stats_hydration_effect()
            .expect("huge compares should keep streaming offscreen stats");

        match effect {
            Effect::Compare(CompareEffect::LoadFileStats(task)) => {
                assert_eq!(task.request.priority, CompareWorkPriority::Warmup);
                assert_eq!(task.request.files.first().map(|item| item.index), Some(128));
            }
            other => panic!("expected LoadFileStats effect, got {other:?}"),
        }
    }

    #[test]
    fn large_compare_still_loads_exact_total_stats() {
        let mut state = compare_ready_state();
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state
            .compare
            .repo_path
            .set(&state.store, Some(PathBuf::from("/repo")));

        let summaries = (0..=super::COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT)
            .map(|index| {
                let path = format!("src/file-{index}.rs");
                CompareFileSummary::from_paths_status(
                    Some(&path),
                    Some(&path),
                    carbon::FileStatus::Modified,
                    true,
                )
            })
            .collect();
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                file_summaries: summaries,
                ..CompareOutput::default()
            }),
        );

        let effect = state
            .start_compare_total_stats_if_needed()
            .expect("large deferred compares should request one bounded total-stats job");

        match effect {
            Effect::Compare(CompareEffect::LoadStats(task)) => {
                assert_eq!(task.request.priority, CompareWorkPriority::TotalStats);
                assert!(
                    state
                        .workspace
                        .compare_total_stats_loading
                        .get(&state.store)
                );
            }
            other => panic!("expected LoadStats effect, got {other:?}"),
        }
    }

    #[test]
    fn filtered_compare_stats_hydrates_filtered_visible_raw_indices() {
        let state = compare_ready_state();
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.file_list.row_height.set(&state.store, 36.0);
        state.file_list.gap.set(&state.store, 4.0);
        state.file_list.viewport_height.set(&state.store, 80.0);
        state
            .file_list
            .filter
            .set(&state.store, "target-only".to_owned());

        let summaries = (0..50)
            .map(|index| {
                let path = if index == 40 {
                    "src/target-only.rs".to_owned()
                } else {
                    format!("src/file-{index}.rs")
                };
                CompareFileSummary::from_paths_status(
                    Some(&path),
                    Some(&path),
                    carbon::FileStatus::Modified,
                    true,
                )
            })
            .collect();
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                file_summaries: summaries,
                ..CompareOutput::default()
            }),
        );

        let items = state.visible_compare_stats_hydration_items();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].index, 40);
    }

    #[test]
    fn tree_compare_stats_hydrates_visible_tree_file_indices() {
        let state = compare_ready_state();
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state
            .file_list
            .mode
            .set(&state.store, SidebarMode::TreeView);
        state
            .file_list
            .expanded_folders
            .set(&state.store, ["a".to_owned()].into_iter().collect());
        state.file_list.row_height.set(&state.store, 36.0);
        state.file_list.gap.set(&state.store, 4.0);
        state.file_list.viewport_height.set(&state.store, 80.0);

        let summaries = (0..50)
            .map(|index| {
                let path = if index == 40 {
                    "a/target-visible.rs".to_owned()
                } else {
                    format!("z/file-{index}.rs")
                };
                CompareFileSummary::from_paths_status(
                    Some(&path),
                    Some(&path),
                    carbon::FileStatus::Modified,
                    true,
                )
            })
            .collect();
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                file_summaries: summaries,
                ..CompareOutput::default()
            }),
        );

        let items = state.visible_compare_stats_hydration_items();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].index, 40);
    }

    #[test]
    fn loaded_compare_stats_update_sidebar_meta() {
        let mut state = compare_ready_state();
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state
            .file_list
            .mode
            .set(&state.store, SidebarMode::TreeView);
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                file_summaries: vec![CompareFileSummary::from_paths_status(
                    None,
                    Some("arch/arm64/boot/dts/mediatek/mt8183-kukui-jacuzzi-kenzo.dts"),
                    carbon::FileStatus::Added,
                    true,
                )],
                ..CompareOutput::default()
            }),
        );

        let effects = state.handle_compare_file_stats_ready(CompareFileStatsReady {
            generation: state.workspace.compare_generation.get(&state.store),
            stats: vec![CompareFileStat {
                index: 0,
                path: "arch/arm64/boot/dts/mediatek/mt8183-kukui-jacuzzi-kenzo.dts".to_owned(),
                additions: 13,
                deletions: 0,
            }],
            request_complete: false,
        });

        assert!(effects.is_empty());
        let meta = state.file_list_entry_meta(0);
        assert_eq!(meta.additions, 13);
        assert_eq!(meta.deletions, 0);
        assert!(
            !state.workspace.compare_output.with(&state.store, |output| {
                output
                    .as_ref()
                    .and_then(|output| output.file_summaries.first())
                    .is_none_or(|summary| summary.stats_deferred)
            }),
            "loaded stats must clear the deferred marker used by sidebar rows",
        );
    }

    #[test]
    fn expanding_tree_folder_starts_visible_stats_hydration() {
        let mut state = compare_ready_state();
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state
            .compare
            .repo_path
            .set(&state.store, Some(PathBuf::from("/repo")));
        state
            .file_list
            .mode
            .set(&state.store, SidebarMode::TreeView);
        state.file_list.row_height.set(&state.store, 36.0);
        state.file_list.gap.set(&state.store, 4.0);
        state.file_list.viewport_height.set(&state.store, 80.0);

        let summaries = (0..=super::COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT)
            .map(|index| {
                let path = if index == 40 {
                    "a/target-visible.rs".to_owned()
                } else {
                    format!("z/file-{index}.rs")
                };
                CompareFileSummary::from_paths_status(
                    Some(&path),
                    Some(&path),
                    carbon::FileStatus::Modified,
                    true,
                )
            })
            .collect();
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                file_summaries: summaries,
                ..CompareOutput::default()
            }),
        );
        state.set_compare_stats_hydration(super::CompareStatsHydrationState::Running);

        let effects =
            state.apply_action(crate::actions::FileListAction::ToggleFolder("a".to_owned()));

        assert!(effects.iter().any(|effect| {
            matches!(
                effect,
                Effect::Compare(CompareEffect::LoadFileStats(task))
                    if task.request.priority == CompareWorkPriority::VisibleSidebarStats
                        && task.request.files.iter().any(|item| item.index == 40)
            )
        }));
    }

    #[test]
    fn compare_stats_ready_drains_history_when_hydration_has_no_visible_work() {
        let mut state = compare_ready_state();
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.file_list.tab.set(&state.store, SidebarTab::Commits);
        state.workspace.compare_history_pending.set(
            &state.store,
            Some(crate::effects::CompareHistoryRequest {
                repo_path: PathBuf::from("/repo"),
                left_ref: "v5.0".to_owned(),
                right_ref: "v5.1".to_owned(),
            }),
        );
        let summaries = (0..=super::COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT)
            .map(|index| {
                let path = format!("src/file-{index}.rs");
                CompareFileSummary::from_paths_status(
                    Some(&path),
                    Some(&path),
                    carbon::FileStatus::Modified,
                    true,
                )
            })
            .collect();
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                file_summaries: summaries,
                ..CompareOutput::default()
            }),
        );

        let effects = state.handle_compare_stats_ready(CompareStatsReady {
            generation: state.workspace.compare_generation.get(&state.store),
            additions: 0,
            deletions: 0,
        });

        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Compare(CompareEffect::LoadHistory(_))))
        );
        assert!(
            state
                .workspace
                .compare_history_pending
                .get(&state.store)
                .is_none()
        );
    }

    #[test]
    fn compare_file_stats_failure_does_not_retry_same_chunk() {
        let mut state = compare_ready_state();
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.set_compare_stats_hydration(super::CompareStatsHydrationState::Running);
        state.workspace.compare_history_pending.set(
            &state.store,
            Some(crate::effects::CompareHistoryRequest {
                repo_path: PathBuf::from("/repo"),
                left_ref: "v5.0".to_owned(),
                right_ref: "v5.1".to_owned(),
            }),
        );
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                file_summaries: vec![CompareFileSummary::from_paths_status(
                    Some("src/file.rs"),
                    Some("src/file.rs"),
                    carbon::FileStatus::Modified,
                    true,
                )],
                ..CompareOutput::default()
            }),
        );

        let effects = state.apply_event(AppEvent::from(CompareEvent::CompareFileStatsFailed {
            generation: state.workspace.compare_generation.get(&state.store),
            message: "backend failed".to_owned(),
        }));

        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Compare(CompareEffect::LoadFileStats(_)))),
            "failed stats hydration should not immediately retry the same deferred chunk"
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Compare(CompareEffect::LoadHistory(_))))
        );
        assert!(
            state
                .workspace
                .compare_history_pending
                .get(&state.store)
                .is_none()
        );
        assert!(state.compare_stats_hydration_failed());
    }

    #[test]
    fn repository_snapshot_ready_clears_repo_open_progress() {
        let mut state = AppState::default();
        let path = PathBuf::from("/tmp/linux");
        let _ = state.open_repository(path.clone());
        assert!(state.compare_progress.with(&state.store, |p| p.is_some()));

        state.apply_event(AppEvent::from(RepositoryEvent::RepositorySnapshotReady(
            crate::events::RepositorySnapshot::from_vcs_snapshot(
                crate::core::vcs::model::VcsSnapshot {
                    location: RepoLocation {
                        kind: VcsKind::GIT,
                        profile: crate::core::vcs::model::VCS_PROFILE_GIT,
                        workspace_root: path,
                        store_root: None,
                    },
                    reason: RepositorySyncReason::Open,
                    change_kind: None,
                    capabilities: RepoCapabilities::git(),
                    refs: Vec::new(),
                    changes: Vec::new(),
                    operation_log: Vec::new(),
                    file_changes: Vec::new(),
                },
            ),
        )));

        assert!(
            state.compare_progress.with(&state.store, |p| p.is_none()),
            "snapshot-ready must tear down the repo-open progress panel"
        );
    }

    #[test]
    fn kickoff_without_prior_state_reveals_loading_immediately() {
        let mut state = compare_ready_state();
        state.clock_ms = 5_000;

        let _ = state.kickoff_compare();
        let progress = state
            .compare_progress
            .with(&state.store, |p| p.clone())
            .expect("progress populated");
        assert_eq!(progress.started_at_ms, 5_000);
        assert_eq!(
            progress.reveal_at_ms, 5_000,
            "compare loading should be visible immediately"
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
                request: vcs_compare_request(
                    CompareMode::TwoDot,
                    "v5.0".to_owned(),
                    "v5.1".to_owned(),
                    LayoutMode::Unified,
                    RendererKind::Builtin,
                ),
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
                request: vcs_compare_request(
                    CompareMode::TwoDot,
                    "v5.0".to_owned(),
                    "v5.1".to_owned(),
                    LayoutMode::Unified,
                    RendererKind::Builtin,
                ),
                resolved_left: "deadbeef".to_owned(),
                resolved_right: "cafefeed".to_owned(),
                output,
                range_commits: Vec::new(),
            },
        )));

        // Small files load synchronously, so progress is already cleared by the
        // time handle_compare_finished returns. We at least know the workspace
        // is Ready and the compare file view is populated from CompareOutput.
        assert_eq!(state.workspace_mode.get(&state.store), WorkspaceMode::Ready,);
        assert_eq!(state.workspace_file_count(), 3);
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
