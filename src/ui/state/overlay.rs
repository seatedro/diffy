use crate::actions::OverlayAction;
use crate::effects::Effect;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: OverlayAction) -> Vec<Effect> {
    state.apply_overlay_action(action)
}

impl AppState {
    pub(super) fn apply_overlay_action(&mut self, action: OverlayAction) -> Vec<Effect> {
        use OverlayAction::*;
        match action {
            OpenRepoPicker => {
                self.open_repo_picker();
                Vec::new()
            }
            OpenRefPicker(field) => self.open_ref_picker(field),
            OpenCommandPalette => self.open_command_palette(),
            OpenGitHubAuthModal => {
                self.push_overlay(
                    OverlaySurface::GitHubAuthModal,
                    Some(FocusTarget::AuthPrimaryAction),
                );
                Vec::new()
            }
            CloseOverlay => {
                if self.overlays_top() == Some(OverlaySurface::RefPicker) {
                    return self.cancel_ref_picker();
                }
                self.pop_overlay();
                Vec::new()
            }
            MoveOverlaySelection(delta) => {
                self.move_overlay_selection(delta);
                Vec::new()
            }
            ConfirmOverlaySelection => self.confirm_overlay_selection(),
            TabCompletePickerDir => {
                self.tab_complete_picker_dir();
                Vec::new()
            }
            SelectOverlayEntry(index) => {
                self.select_overlay_entry(index);
                self.confirm_overlay_selection()
            }
            HoverOverlayEntry(Some(index)) => {
                self.overlays
                    .picker
                    .hovered_index
                    .set(&self.store, Some(index));
                self.select_overlay_entry(index);
                Vec::new()
            }
            HoverOverlayEntry(None) => {
                self.overlays.picker.hovered_index.set(&self.store, None);
                Vec::new()
            }
            ScrollActiveOverlayListPx(delta_px) => {
                self.scroll_active_overlay_list_px(delta_px);
                Vec::new()
            }
            ShowKeyboardShortcuts => {
                self.clear_overlays();
                self.ui.app_view.set(&self.store, AppView::Settings);
                self.ui
                    .settings_section
                    .set(&self.store, SettingsSection::Keymaps);
                Vec::new()
            }
        }
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

pub(super) fn palette_command_available(
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

pub(super) fn vcs_operation_available_for_location(
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

pub(super) fn operation_log_entry_detail(entry: &VcsOperationLogEntry) -> String {
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
        // The bottom-most entry recorded the focus from before any overlay
        // opened; restore it so focus never dangles on a dismissed surface.
        let mut focus_return: Option<Option<FocusTarget>> = None;
        self.overlays.stack.update(&self.store, |stack| {
            focus_return = stack.first().map(|entry| entry.focus_return);
            stack.clear();
        });
        self.reset_picker();
        self.reset_command_palette();
        self.reset_confirmation();
        if let Some(target) = focus_return {
            self.set_focus(target);
        }
    }
}
pub(super) fn overlay_name(surface: OverlaySurface) -> &'static str {
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

pub(super) fn font_picker_entry(
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

pub(super) fn highlight_ranges_from_match_indices(
    text: &str,
    indices_rev: &[usize],
) -> Vec<(usize, usize)> {
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

pub(super) fn highlight_ranges_for_prefix_match(
    text: &str,
    indices_rev: &[usize],
) -> Vec<(usize, usize)> {
    let prefix_indices: Vec<usize> = indices_rev
        .iter()
        .copied()
        .filter(|&idx| idx < text.len())
        .collect();
    highlight_ranges_from_match_indices(text, &prefix_indices)
}

pub(super) fn highlight_ranges_for_visible_match(
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

pub(super) fn query_looks_like_path(query: &str) -> bool {
    query.starts_with('/')
        || query.starts_with("~/")
        || query.starts_with("./")
        || (query.len() >= 2 && query.as_bytes()[1] == b':')
}

pub(super) fn path_looks_like_repository(path: &Path) -> bool {
    path.join(".git").exists() || path.join(".jj").exists()
}

pub(super) fn normalize_repository_open_path(path: PathBuf) -> PathBuf {
    crate::core::vcs::discovery::discover_repository(&path)
        .ok()
        .flatten()
        .map(|location| location.workspace_root)
        .unwrap_or(path)
}

pub(super) fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") || path == "~" {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_owned()
}

pub(super) fn split_browse_query(expanded: &str) -> (String, &str) {
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

impl AppState {
    pub fn active_overlay_name(&self) -> Option<&'static str> {
        self.overlays_active_name()
    }

    pub(super) fn open_repo_picker(&mut self) {
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

    pub(super) fn open_theme_picker(&mut self) {
        let scale = self.ui_scale_factor();
        self.ui
            .theme_preview_original
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

    pub(super) fn build_theme_entries_grouped(&self) -> Vec<PickerEntry> {
        use crate::core::themes::ThemeVariant;

        let original = self
            .ui
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

        let variant_of = |index: usize| {
            self.theme_variants
                .get(index)
                .copied()
                .unwrap_or(ThemeVariant::Dark)
        };
        let mut ordered: Vec<usize> = Vec::with_capacity(self.theme_names.len());
        for group in [ThemeVariant::Dual, ThemeVariant::Dark, ThemeVariant::Light] {
            ordered.extend((0..self.theme_names.len()).filter(|&index| variant_of(index) == group));
        }

        build_sectioned_rows(
            &ordered,
            |index| Some(variant_of(index)),
            |variant| {
                make_header(match variant {
                    ThemeVariant::Dual => "Dark & Light",
                    ThemeVariant::Dark => "Dark",
                    ThemeVariant::Light => "Light",
                })
            },
            |index| self.theme_names.get(index).map(make_entry),
        )
    }

    pub(super) fn rebuild_theme_picker(&mut self) {
        let query = self
            .overlays
            .picker
            .query
            .with(&self.store, |q| q.trim().to_owned());
        let original = self
            .ui
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

    pub(super) fn open_font_picker(&mut self, role: FontRole) {
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

    pub(super) fn rebuild_font_picker(&mut self) {
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

    pub(super) fn font_picker_role(&self) -> Option<FontRole> {
        match self.overlays.picker.kind.get(&self.store) {
            PickerKind::UiFont => Some(FontRole::Ui),
            PickerKind::MonoFont => Some(FontRole::Mono),
            _ => None,
        }
    }

    pub(super) fn selected_font_family(&self, role: FontRole) -> String {
        match role {
            FontRole::Ui => {
                crate::fonts::normalize_font_selection(role, &self.settings.fonts.ui_family)
            }
            FontRole::Mono => {
                crate::fonts::normalize_font_selection(role, &self.settings.fonts.mono_family)
            }
        }
    }

    pub(super) fn open_ref_picker(&mut self, field: CompareField) -> Vec<Effect> {
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

    pub(super) fn open_command_palette(&mut self) -> Vec<Effect> {
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

    pub(super) fn push_overlay(
        &mut self,
        surface: OverlaySurface,
        focus_target: Option<FocusTarget>,
    ) {
        if self.overlays_top() == Some(surface) {
            self.set_focus(focus_target);
            return;
        }
        let focus_return = self.ui.focus.get(&self.store);
        self.overlays.stack.update(&self.store, |stack| {
            stack.push(OverlayEntry {
                surface,
                focus_return,
            });
        });
        self.set_focus(focus_target);
    }

    pub(super) fn pop_overlay(&mut self) {
        let mut popped: Option<OverlayEntry> = None;
        self.overlays.stack.update(&self.store, |stack| {
            popped = stack.pop();
        });
        let Some(entry) = popped else {
            return;
        };
        match entry.surface {
            OverlaySurface::ThemePicker => {
                let original = self.ui.theme_preview_original.get(&self.store);
                self.ui.theme_preview_original.set(&self.store, None);
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

    pub(super) fn open_confirmation(
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
        // Let push_overlay snapshot the current focus as the restore target
        // before it moves focus off the field; closing the confirmation then
        // returns focus (and IME state) to wherever the user was.
        self.push_overlay(OverlaySurface::Confirmation, None);
    }

    pub(super) fn move_overlay_selection(&mut self, delta: i32) {
        match self.overlays_top() {
            Some(OverlaySurface::ThemePicker) => {
                let current = self.overlays.picker.selected_index.get(&self.store);
                let (idx, len, value) = self.overlays.picker.entries.with(&self.store, |entries| {
                    let len = entries.len();
                    let idx = step_selection(current, delta, len, |i| entries[i].section_header);
                    let value = idx.and_then(|idx| {
                        entries
                            .get(idx)
                            .filter(|e| !e.section_header)
                            .map(|e| e.value.clone())
                    });
                    (idx, len, value)
                });
                let Some(idx) = idx else {
                    return;
                };
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
                    let idx = step_selection(current, delta, len, |i| entries[i].section_header);
                    (idx, len)
                });
                let Some(idx) = idx else {
                    return;
                };
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
                let current = self
                    .overlays
                    .command_palette
                    .selected_index
                    .get(&self.store);
                // Palette entries have no section headers; an empty palette
                // still pins the selection to row zero.
                let idx = step_selection(current, delta, entry_count, |_| false).unwrap_or(0);
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

    pub(super) fn select_overlay_entry(&mut self, index: usize) {
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

    pub(super) fn confirm_overlay_selection(&mut self) -> Vec<Effect> {
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
                self.ui.theme_preview_original.set(&self.store, None);
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

    pub(super) fn confirm_font_picker(&mut self) -> Vec<Effect> {
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

    pub(super) fn confirm_repo_picker(&mut self) -> Vec<Effect> {
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

    pub(super) fn tab_complete_picker_dir(&mut self) {
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

    pub(super) fn navigate_picker_to_dir(&mut self, path: &Path) {
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

    pub(super) fn confirm_ref_picker(&mut self, field: CompareField) -> Vec<Effect> {
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

    pub(super) fn commit_ref_picker(&mut self) -> Vec<Effect> {
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

    pub(super) fn cancel_ref_picker(&mut self) -> Vec<Effect> {
        let left = self.overlays.ref_picker.original_left.get(&self.store);
        let right = self.overlays.ref_picker.original_right.get(&self.store);
        self.compare.left_ref.set(&self.store, left);
        self.compare.right_ref.set(&self.store, right);
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        self.pop_overlay();
        Vec::new()
    }

    pub(super) fn set_active_ref_field(&mut self, field: CompareField) -> Vec<Effect> {
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

    pub(super) fn swap_draft_refs(&mut self) -> Vec<Effect> {
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

    pub(super) fn apply_compare_preset(&mut self, preset: &str) -> Vec<Effect> {
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

    pub(super) fn confirm_command_palette(&mut self) -> Vec<Effect> {
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

    pub(super) fn confirm_pr_entry(&mut self, key: PrKey) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            self.push_error("Open a repository before loading a pull request.");
            return Vec::new();
        };
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
            Some(PrPeekDiff::Loading) => {
                self.github
                    .pull_request
                    .pending_confirm
                    .set(&self.store, Some(key.clone()));
                self.push_info(&format!("Preparing PR #{}\u{2026}", key.2));
                Vec::new()
            }
            Some(PrPeekDiff::Idle) | None => {
                self.github
                    .pull_request
                    .pending_confirm
                    .set(&self.store, Some(key.clone()));
                self.github
                    .pull_request
                    .status
                    .set(&self.store, AsyncStatus::Loading);
                self.github.pull_request.cache.update(&self.store, |c| {
                    let entry = c.entry(key.clone()).or_insert_with(|| PrCacheEntry {
                        meta: PrPeekMeta::Loading,
                        diff: PrPeekDiff::Idle,
                        last_peek_ms: self.clock_ms,
                    });
                    entry.diff = PrPeekDiff::Loading;
                });
                self.push_info(&format!("Preparing PR #{}\u{2026}", key.2));
                vec![
                    GitHubEffect::LoadPullRequest {
                        url: format!("https://github.com/{}/{}/pull/{}", key.0, key.1, key.2),
                        repo_path,
                        github_token: self.github_access_token.clone(),
                    }
                    .into(),
                ]
            }
            Some(PrPeekDiff::Failed(message)) => {
                self.push_error(&message);
                Vec::new()
            }
        }
    }

    pub(super) fn confirm_or_run_vcs_operation(&mut self, operation: VcsOperation) -> Vec<Effect> {
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

    pub(super) fn rebuild_repo_picker(&mut self) {
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

    pub(super) fn rebuild_repo_picker_recent(&mut self, query: &str) {
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

    pub(super) fn rebuild_repo_picker_browse(&mut self, query: &str) {
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

    pub(super) fn rebuild_ref_picker(&mut self, field: CompareField) -> Vec<Effect> {
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

    pub(super) fn rebuild_command_palette_if_open(&mut self) -> Vec<Effect> {
        if self.overlays_top() == Some(OverlaySurface::CommandPalette) {
            self.rebuild_command_palette()
        } else {
            Vec::new()
        }
    }

    pub(super) fn rebuild_command_palette(&mut self) -> Vec<Effect> {
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
        match self.ui.update.get(&self.store) {
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

    pub(super) fn scroll_active_overlay_list_px(&mut self, delta_px: i32) {
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
}
