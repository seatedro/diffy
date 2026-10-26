use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use quark_components::{
    CommandPalette, ContextMenuOutcome, ContextMenuState, PALETTE_INPUT, PaletteEvent,
    PaletteOutcome,
};
use quark_ui::FocusId;
use quark_ui::element::{Binding, ScrollHandle};
use quark_ui::text_input::{TextEditCommand, TextEditOutcome, TextField};

use super::keymap;
use super::launcher::{Launcher, LauncherChoice};
use super::palette::{self, PaletteCommand, PaletteSource};
use super::picker::{self, PickerCandidate, PickerChoice, PickerKind, PickerState};
use super::platform::{self, PlatformRequest, ids};
use super::ref_picker::{RefPicker, is_working_tree};
use super::repo_picker::{RepoEntry, RepoView};
use super::toasts::ToastHost;
use crate::app::text_targets::{LAUNCHER, PICKER, SETTINGS, SETTINGS_SEARCH, TextCx, TextHost};
use crate::app::viewport::outline;
use crate::contracts::commands::{
    CommandDescriptor, CommandsAction, CommandsEffect, CommandsEvent, ConfirmationRequest,
    ContextMenuEntry, FileListRequest, LauncherFolder, RefCatalogRequest, RefPickerAction,
    RefPickerRequest, SettingsSection, SettingsSnapshot, ToastKind, ToastUpdate,
};
use crate::contracts::mutations::MutationsAction;
use crate::contracts::navigation::{
    CompareField, NavigationAction, RepositorySyncReason, SidebarTab,
};
use crate::contracts::review::{PullsAction, ReviewAction};
use crate::contracts::shared::{JobId, SharedContext};
use crate::contracts::shell::{
    Action, Controller, Intent, LogicalFocus, Reduction, ShellAction, ShellEffect,
};
use crate::contracts::viewport::files::{FileAt, FilesAction, head_revision, revision_label};
use crate::contracts::viewport::pipeline::{DiffSizeRequest, PipelineAction};
use crate::contracts::viewport::symbols::SymbolsAction;
use crate::core::compare::LayoutMode;
use crate::core::frecency::{FrecencyStore, recent_repo_paths, repo_last_opened};
use crate::core::keymap::{KeymapOverride, ShortcutCommand};
use crate::core::ref_catalog::{RefCatalog, RefEntryKind, WORKING_TREE_LABEL};
use crate::core::repo_discovery::RepoScan;
use crate::core::theme_types::ThemeMode;
use crate::core::tips;
use crate::core::vcs::git::{WORKDIR_REF, is_full_hex_oid};
use crate::core::vcs::model::{RevisionId, VcsKind};
use crate::fonts::{FontFamilyEntry, FontRole, bundled_font_entries, normalize_font_selection};
use crate::platform::persistence::Settings;

pub const CONFIRM_CANCEL: FocusId = FocusId::from_key("diffy.confirm.cancel");
pub const CONFIRM_ACCEPT: FocusId = FocusId::from_key("diffy.confirm.accept");

pub const MIN_UI_SCALE_PCT: u16 = 70;
pub const MAX_UI_SCALE_PCT: u16 = 180;
pub const UI_SCALE_STEP_PCT: u16 = 10;
const FILE_LIST_CACHE: usize = 4;
pub const DEFAULT_UI_SCALE_PCT: u16 = 100;
const RECENT_REPOSITORIES: usize = 12;
const RECENT_COMMANDS: usize = 8;
const PROJECT_URL: &str = "https://github.com/seatedro/diffy";
const QUIT_BINDING: &str = "mod+q";
const REF_FRECENCY_PREFIX: &str = "ref:";
// Toast job ids are shared with other streams' counters; keep clones clear of them.
const CLONE_JOBS: u64 = 1 << 48;

const TIP_DELAY_MS: u64 = 8_000;

#[derive(Debug, Clone, Copy)]
enum TipSession {
    Pending,
    Showing(&'static tips::Tip),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    Palette,
    Picker,
    Confirmation,
    ContextMenu,
    RefPicker,
    Shortcuts,
}

#[derive(Debug, Clone, Copy)]
struct Layer {
    kind: LayerKind,
    return_focus: Option<FocusId>,
}

#[derive(Debug, Clone)]
pub struct SettingsPage {
    pub section: SettingsSection,
    pub capture: Option<ShortcutCommand>,
    pub notice: Option<String>,
    pub scroll: ScrollHandle,
    pub search: TextField,
    return_focus: Option<FocusId>,
}

#[derive(Debug, Clone)]
struct FontCatalog {
    ui: &'static [FontFamilyEntry],
    mono: &'static [FontFamilyEntry],
}

pub struct CommandsController {
    settings: Rc<SettingsSnapshot>,
    layers: Vec<Layer>,
    view_focus: Option<FocusId>,
    palette: CommandPalette<PaletteSource>,
    palette_source: PaletteSource,
    picker: Option<PickerState>,
    ref_picker: Option<RefPicker>,
    ref_generation: u64,
    ref_catalog: Option<(usize, Arc<RefCatalog>)>,
    confirmation: Option<ConfirmationRequest>,
    context_menu: ContextMenuState,
    context_entries: Vec<Action>,
    settings_page: Option<SettingsPage>,
    toasts: ToastHost,
    external: Rc<[CommandDescriptor]>,
    fonts: Option<FontCatalog>,
    font_catalog_requested: bool,
    frecency: Option<FrecencyStore>,
    repo_scan: Option<Arc<RepoScan>>,
    repo_scanning: bool,
    repo_picker_current: Option<PathBuf>,
    launcher: Option<Launcher>,
    clones: Vec<(JobId, String)>,
    next_clone: u64,
    file_picker: Option<FileListKey>,
    file_lists: Vec<FileList>,
    file_pending: Option<FileListKey>,
    file_generation: u64,
    go_prefix: bool,
    tip: Cell<TipSession>,
    tip_since: Cell<Option<u64>>,
    shortcuts_reviewing: bool,
    shortcuts_scroll: ScrollHandle,
    go_since: Cell<Option<u64>>,
    row_stride: f32,
    outbox: RefCell<Vec<PlatformRequest>>,
    activation_click: Cell<bool>,
}

impl std::fmt::Debug for CommandsController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandsController")
            .field("settings", &self.settings.revision)
            .field("layers", &self.layers)
            .field("settings_page", &self.settings_page)
            .finish_non_exhaustive()
    }
}

impl Default for CommandsController {
    fn default() -> Self {
        let mut palette = CommandPalette::new()
            .placeholder("Search commands, files and repos, or paste a PR link");
        palette.register(palette::PullRequests);
        palette.register(palette::Commands);
        palette.register(palette::Files);
        palette.register(palette::Repositories);
        Self {
            settings: Rc::default(),
            layers: Vec::new(),
            view_focus: None,
            palette,
            palette_source: PaletteSource::default(),
            picker: None,
            ref_picker: None,
            ref_generation: 0,
            ref_catalog: None,
            confirmation: None,
            context_menu: ContextMenuState::default(),
            context_entries: Vec::new(),
            settings_page: None,
            toasts: ToastHost::default(),
            external: Rc::from(Vec::new()),
            fonts: None,
            font_catalog_requested: false,
            frecency: None,
            repo_scan: None,
            repo_scanning: false,
            repo_picker_current: None,
            launcher: Some(Launcher::default()),
            clones: Vec::new(),
            next_clone: 0,
            file_picker: None,
            file_lists: Vec::new(),
            file_pending: None,
            file_generation: 0,
            go_prefix: false,
            tip: Cell::new(TipSession::Pending),
            tip_since: Cell::new(None),
            shortcuts_reviewing: false,
            shortcuts_scroll: ScrollHandle::new(),
            go_since: Cell::new(None),
            row_stride: 0.0,
            outbox: RefCell::new(Vec::new()),
            activation_click: Cell::new(false),
        }
    }
}

impl CommandsController {
    pub fn settings(&self) -> &Rc<SettingsSnapshot> {
        &self.settings
    }

    pub fn palette(&self) -> &CommandPalette<PaletteSource> {
        &self.palette
    }

    pub fn picker(&self) -> Option<&PickerState> {
        self.picker.as_ref()
    }

    pub(super) fn observe_window_focus(&self, focused: bool) {
        self.activation_click
            .set(focused && !self.layers.is_empty());
    }

    pub(super) fn observe_key(&self) {
        self.activation_click.set(false);
    }

    pub fn editing_ref(&self) -> Option<CompareField> {
        self.ref_picker.as_ref().map(RefPicker::side)
    }

    pub(super) fn ref_picker(&self) -> Option<&RefPicker> {
        self.ref_picker.as_ref()
    }

    pub(super) fn ref_picker_mut(&mut self) -> Option<&mut RefPicker> {
        self.ref_picker.as_mut()
    }

    pub(super) fn picker_mut(&mut self) -> Option<&mut PickerState> {
        self.picker.as_mut()
    }

    pub fn confirmation(&self) -> Option<&ConfirmationRequest> {
        self.confirmation.as_ref()
    }

    pub fn settings_page(&self) -> Option<&SettingsPage> {
        self.settings_page.as_ref()
    }

    pub fn toasts(&self) -> &ToastHost {
        &self.toasts
    }

    pub(super) fn toasts_mut(&mut self) -> &mut ToastHost {
        &mut self.toasts
    }

    pub(super) fn context_menu_mut(&mut self) -> &mut ContextMenuState {
        &mut self.context_menu
    }

    pub fn take_platform(&self) -> Vec<PlatformRequest> {
        self.outbox.take()
    }

    pub fn top_layer(&self) -> Option<LayerKind> {
        self.layers.last().map(|layer| layer.kind)
    }

    pub fn has_layer(&self, kind: LayerKind) -> bool {
        self.layers.iter().any(|layer| layer.kind == kind)
    }

    pub fn settings_open(&self) -> bool {
        self.settings_page.is_some()
    }

    pub fn go_pending(&self) -> bool {
        self.go_prefix
    }

    pub(super) fn go_since(&self, now_ms: u64) -> u64 {
        let since = self.go_since.get().unwrap_or(now_ms);
        self.go_since.set(Some(since));
        since
    }

    fn leader(&mut self, binding: &str, cx: &SharedContext) -> Reduction {
        self.go_prefix = false;
        self.go_since.set(None);
        match keymap::leader(binding) {
            Some(leader) => {
                let mut reduction = self.run(leader.action(), cx);
                reduction.changed = true;
                reduction
            }
            None => Reduction::changed(),
        }
    }

    pub(super) fn capturing(&self) -> Option<ShortcutCommand> {
        self.settings_page.as_ref().and_then(|page| page.capture)
    }

    pub(super) fn repo_scan_status(&self) -> (Option<&RepoScan>, bool) {
        (self.repo_scan.as_deref(), self.repo_scanning)
    }

    pub fn launcher_active(&self) -> bool {
        self.launcher.is_some()
    }

    pub(super) fn launcher_mut(&mut self) -> Option<&mut Launcher> {
        self.launcher.as_mut()
    }

    pub fn observe_account(&mut self, signed_in: bool) {
        if let Some(launcher) = &mut self.launcher {
            launcher.set_signed_in(signed_in);
        }
    }

    fn launcher_recent(&self) -> Vec<PathBuf> {
        recent_repo_paths(self.frecency.as_ref(), RECENT_REPOSITORIES)
    }

    fn refresh_launcher(&mut self) -> Reduction {
        let recent = self.launcher_recent();
        let scan = self.repo_scan.clone();
        let Some(launcher) = &mut self.launcher else {
            return Reduction::default();
        };
        launcher.set_entries(recent, scan.as_deref());
        self.count_launcher_rows()
    }

    fn count_launcher_rows(&mut self) -> Reduction {
        let Some(launcher) = &mut self.launcher else {
            return Reduction::default();
        };
        let paths = launcher.take_count_requests();
        let reduction = Reduction::changed();
        if paths.is_empty() {
            reduction
        } else {
            reduction.effect(CommandsEffect::CountChanges(paths))
        }
    }

    fn confirm_launcher(&mut self, cx: &SharedContext) -> Reduction {
        let Some(launcher) = &self.launcher else {
            return Reduction::default();
        };
        let Some(choice) = launcher.selected_choice().cloned() else {
            return Reduction::default();
        };
        let signed_in = launcher.signed_in;
        match choice {
            LauncherChoice::Repository(path) | LauncherChoice::Path(path) => {
                self.request(PlatformRequest::Dispatch(
                    NavigationAction::OpenRepository(path).into(),
                ));
                Reduction::changed()
            }
            LauncherChoice::PullRequest { url } => {
                self.request(PlatformRequest::Dispatch(
                    ReviewAction::OpenPullRequest { reference: url }.into(),
                ));
                Reduction::changed()
            }
            LauncherChoice::PullRequests if signed_in => {
                self.request(PlatformRequest::Dispatch(
                    ReviewAction::Pulls(PullsAction::Open).into(),
                ));
                Reduction::changed()
            }
            LauncherChoice::PullRequests => {
                self.request(PlatformRequest::Dispatch(
                    ReviewAction::OpenAuthModal.into(),
                ));
                Reduction::changed()
            }
            LauncherChoice::OpenFolder => {
                self.reduce_action(CommandsAction::OpenRepositoryFolder, cx)
            }
            LauncherChoice::Clone(Some(source)) => Reduction::changed().effect(
                ShellEffect::OpenLauncherDialog(LauncherFolder::CloneInto {
                    url: source.url,
                    name: source.repo.repo,
                }),
            ),
            LauncherChoice::Clone(None) => {
                self.toasts.apply(ToastUpdate::info(
                    "Type owner/repo or a GitHub URL, then choose Clone",
                ));
                Reduction::changed()
            }
        }
    }

    fn launcher_folder(&mut self, folder: LauncherFolder, path: PathBuf) -> Reduction {
        match folder {
            LauncherFolder::CloneInto { url, name } => {
                self.next_clone += 1;
                let job = JobId(CLONE_JOBS | self.next_clone);
                self.toasts.apply(ToastUpdate::Progress {
                    id: job,
                    message: format!("Cloning {name}\u{2026}"),
                    completed: 0,
                    total: 0,
                });
                let dest = path.join(&name);
                self.clones.push((job, name));
                Reduction::changed().effect(CommandsEffect::CloneRepository { job, url, dest })
            }
        }
    }

    fn clone_name(&self, job: JobId) -> Option<&str> {
        self.clones
            .iter()
            .find(|(id, _)| *id == job)
            .map(|(_, name)| name.as_str())
    }

    pub(super) fn picker_browses_repos(&self) -> bool {
        self.picker
            .as_ref()
            .is_some_and(|picker| picker.repos.is_some() && picker.query.text().trim().is_empty())
    }

    pub(super) fn picker_completes_paths(&self) -> bool {
        self.picker
            .as_ref()
            .is_some_and(|picker| picker.kind == PickerKind::Repository)
    }

    pub(super) fn observe_view(&mut self, focus: Option<FocusId>, row_stride: f32) {
        if self.layers.is_empty() && self.settings_page.is_none() {
            self.view_focus = focus;
        }
        self.row_stride = row_stride;
    }

    pub fn commands(&self) -> Vec<CommandDescriptor> {
        let external = &self.external;
        let mut commands = builtin_commands(&self.settings.settings);
        commands.retain(|c| !external.iter().any(|e| e.id == c.id));
        for command in &mut commands {
            if command
                .shortcut
                .is_some_and(|s| external.iter().any(|e| e.shortcut == Some(s)))
            {
                command.shortcut = None;
            }
        }
        commands.extend(external.iter().cloned());
        commands
    }

    pub(super) fn command_by_id(&self, id: &str) -> Option<CommandDescriptor> {
        self.external
            .iter()
            .find(|command| command.id == id)
            .cloned()
            .or_else(|| {
                builtin_commands(&self.settings.settings)
                    .into_iter()
                    .find(|command| command.id == id)
            })
    }

    fn request(&self, request: PlatformRequest) {
        self.outbox.borrow_mut().push(request);
    }

    fn focus(&self, focus: Option<FocusId>) {
        self.request(PlatformRequest::Focus(focus));
    }

    fn open_shortcuts(&mut self, cx: &SharedContext) -> Reduction {
        if self.top_layer() == Some(LayerKind::Shortcuts) {
            return self.close_top();
        }
        let Some(return_focus) = self.admit(LayerKind::Shortcuts) else {
            return Reduction::default();
        };
        self.shortcuts_reviewing = cx.review.is_some();
        self.shortcuts_scroll.set_offset(0.0, 0.0);
        self.push_layer(LayerKind::Shortcuts, return_focus);
        Reduction::changed()
    }

    pub(super) fn shortcuts_reviewing(&self) -> bool {
        self.shortcuts_reviewing
    }

    pub(super) fn shortcuts_scroll(&self) -> &ScrollHandle {
        &self.shortcuts_scroll
    }

    fn current_focus(&self) -> Option<FocusId> {
        match self.layers.last() {
            Some(layer) => Some(layer_focus(layer.kind)),
            None if self.settings_page.is_some() => Some(SETTINGS),
            None => self.view_focus,
        }
    }

    pub fn modal_open(&self) -> bool {
        self.top_layer() == Some(LayerKind::Confirmation)
    }

    fn admit(&mut self, kind: LayerKind) -> Option<Option<FocusId>> {
        if self.modal_open() && kind != LayerKind::Confirmation {
            return None;
        }
        let mut return_focus = self.current_focus();
        while let Some(layer) = self.layers.pop() {
            self.clear_layer(layer.kind);
            return_focus = layer.return_focus;
        }
        Some(return_focus)
    }

    fn push_layer(&mut self, kind: LayerKind, return_focus: Option<FocusId>) {
        self.layers.push(Layer { kind, return_focus });
        self.focus(Some(layer_focus(kind)));
    }

    fn clear_layer(&mut self, kind: LayerKind) {
        match kind {
            LayerKind::Palette => {
                self.palette.close();
            }
            LayerKind::Picker => self.picker = None,
            LayerKind::RefPicker => self.ref_picker = None,
            LayerKind::Confirmation => self.confirmation = None,
            LayerKind::Shortcuts => {}
            LayerKind::ContextMenu => {
                self.context_menu.close();
                self.context_entries.clear();
            }
        }
    }

    fn dismiss_overlays(&mut self) -> Reduction {
        if self.modal_open() {
            return Reduction::default();
        }
        match self.top_layer() {
            Some(kind) => {
                self.pop_layer(kind);
                Reduction::changed()
            }
            None => Reduction::default(),
        }
    }

    fn pop_layer(&mut self, kind: LayerKind) -> bool {
        let Some(index) = self.layers.iter().rposition(|layer| layer.kind == kind) else {
            return false;
        };
        let layer = self.layers.remove(index);
        self.clear_layer(kind);
        if index == self.layers.len() {
            self.focus(layer.return_focus);
        }
        true
    }

    fn close_top(&mut self) -> Reduction {
        match self.top_layer() {
            Some(kind) => {
                self.pop_layer(kind);
                Reduction::changed()
            }
            None if self.settings_page.is_some() => self.close_settings(),
            None => Reduction::default(),
        }
    }

    fn run(&mut self, action: Action, cx: &SharedContext) -> Reduction {
        match action {
            Action::Commands(action) => self.reduce_action(action, cx),
            action => {
                self.request(PlatformRequest::Dispatch(action));
                Reduction::changed()
            }
        }
    }

    fn run_command(&mut self, id: &str, cx: &SharedContext) -> Reduction {
        let Some(command) = self.command_by_id(id) else {
            return Reduction::default();
        };
        if !command.enabled {
            return Reduction::default();
        }
        if let Some(store) = &self.frecency {
            store.record_access(&palette::frecency_key(command.id));
        }
        self.run(command.action, cx)
    }

    fn open_palette(&mut self, cx: &SharedContext) -> Reduction {
        let Some(return_focus) = self.admit(LayerKind::Palette) else {
            return Reduction::default();
        };
        let saved = &self.settings.settings.keymap_overrides;
        let (settings, reviewing) = (self.settings_open(), cx.review.is_some());
        self.palette_source.commands = self
            .commands()
            .into_iter()
            .filter(|descriptor| descriptor.id != ids::PALETTE)
            .map(|descriptor| PaletteCommand {
                binding: descriptor
                    .shortcut
                    .filter(|&s| keymap::works_on(s, settings, reviewing))
                    .and_then(|s| keymap::first_binding(saved, s))
                    .and_then(|b| b.parse::<Binding>().ok()),
                descriptor,
            })
            .collect();
        self.palette_source.files = cx
            .manifest
            .as_ref()
            .map(|manifest| {
                manifest
                    .files
                    .iter()
                    .map(|file| (file.key, file.paths.display_path().into_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let current = cx
            .repository
            .as_ref()
            .map(|repo| repo.snapshot.path.clone());
        self.palette_source.repositories =
            recent_repo_paths(self.frecency.as_ref(), RECENT_REPOSITORIES)
                .into_iter()
                .filter(|path| Some(path) != current.as_ref())
                .collect();
        if let Some(store) = &self.frecency {
            let recent: Vec<u64> = store
                .recent(64)
                .iter()
                .filter_map(|key| key.strip_prefix("cmd:"))
                .map(palette::command_key)
                .take(RECENT_COMMANDS)
                .collect();
            self.palette.set_recent(recent);
        }
        self.layers.push(Layer {
            kind: LayerKind::Palette,
            return_focus,
        });
        let focus = self.palette.open(&self.palette_source, return_focus);
        self.focus(Some(focus));
        Reduction::changed()
    }

    fn palette_outcome(
        &mut self,
        outcome: Option<PaletteOutcome>,
        cx: &SharedContext,
    ) -> Reduction {
        match outcome {
            None => Reduction::default(),
            Some(PaletteOutcome::Handled) => Reduction::changed(),
            Some(PaletteOutcome::Closed { .. }) => {
                self.pop_layer(LayerKind::Palette);
                Reduction::changed()
            }
            Some(PaletteOutcome::Run { action, .. }) => {
                self.pop_layer(LayerKind::Palette);
                let mut reduction = Reduction::changed();
                if let Some(action) = action.downcast_ref::<Action>() {
                    reduction.merge(self.run(action.clone(), cx));
                }
                reduction
            }
        }
    }

    fn open_picker(&mut self, kind: PickerKind, candidates: Vec<PickerCandidate>) -> Reduction {
        self.show_picker(PickerState::new(kind, candidates))
    }

    fn show_picker(&mut self, picker: PickerState) -> Reduction {
        if self.has_layer(LayerKind::Picker) {
            self.picker = Some(picker);
            self.focus(Some(PICKER));
            return Reduction::changed();
        }
        let Some(return_focus) = self.admit(LayerKind::Picker) else {
            return Reduction::default();
        };
        self.picker = Some(picker);
        self.push_layer(LayerKind::Picker, return_focus);
        Reduction::changed()
    }

    fn scan_repositories(&mut self) -> Reduction {
        self.repo_scanning = true;
        Reduction::changed().effect(CommandsEffect::ScanRepositories {
            options: Box::new(self.settings.settings.repo_discovery.clone()),
            known: recent_repo_paths(self.frecency.as_ref(), RECENT_REPOSITORIES),
        })
    }

    fn open_repo_picker(&mut self, anchored: bool, cx: &SharedContext) -> Reduction {
        let current = cx.repository.as_ref().map(|r| r.snapshot.path.clone());
        let mut recent: Vec<PathBuf> =
            recent_repo_paths(self.frecency.as_ref(), RECENT_REPOSITORIES);
        if let Some(current) = &current
            && !recent.contains(current)
        {
            recent.insert(0, current.clone());
        }
        let store = self.frecency.as_ref();
        let home = dirs::home_dir();
        let now = crate::core::ref_catalog::now_secs();
        let mut recent: Vec<(PathBuf, Option<i64>)> = recent
            .into_iter()
            .map(|path| {
                let opened = repo_last_opened(store, &path);
                (path, opened)
            })
            .collect();
        recent.sort_by_key(|(_, opened)| std::cmp::Reverse(*opened));
        let mut view = RepoView {
            recent: recent
                .into_iter()
                .map(|(path, opened)| {
                    RepoEntry::new(path, home.as_deref(), None, opened, now, current.as_deref())
                })
                .collect(),
            ..RepoView::default()
        };
        if let Some(repo) = cx.repository.as_ref() {
            view.set_worktrees(&repo.snapshot.worktrees, home.as_deref());
        }
        if let Some(scan) = &self.repo_scan {
            view.set_discovered(scan, home.as_deref(), now, current.as_deref());
        }
        let counts = view.worktree_paths();
        let mut reduction = self.scan_repositories();
        view.scanning = self.repo_scanning;
        self.repo_picker_current = current;
        reduction.merge(self.show_picker(PickerState::repositories(view, anchored)));
        if !counts.is_empty() {
            reduction = reduction.effect(CommandsEffect::CountChanges(counts));
        }
        reduction
    }

    fn open_file_picker(&mut self, prefix: &str, cx: &SharedContext) -> Reduction {
        let Some(key) = file_list_key(cx) else {
            return Reduction::default();
        };
        let mut state = PickerState::new(PickerKind::File, Vec::new());
        state.symbols = cx.symbols.clone();
        state.document = cx.document.clone();
        state.head = Some(key.revision.clone());
        if !prefix.is_empty() {
            state.query = TextField::new(prefix);
        }
        state.refilter();
        let mut reduction = self.show_picker(state);
        let Some(picker) = self.picker.as_mut() else {
            return reduction;
        };
        let cached = self.file_lists.iter().position(|list| list.key == key);
        let fresh = cached.is_some() && is_full_hex_oid(&key.revision.id);
        if let Some(index) = cached {
            let list = self.file_lists.remove(index);
            picker.set_files(list.files.clone());
            self.file_lists.push(list);
        }
        self.file_picker = Some(key.clone());
        if !fresh && let Some(effect) = self.list_files(key) {
            reduction = reduction.effect(effect);
        }
        reduction
    }

    fn prefetch_files(&mut self, cx: &SharedContext) -> Reduction {
        let Some(key) = file_list_key(cx) else {
            return Reduction::default();
        };
        if self.file_lists.iter().any(|list| list.key == key) {
            return Reduction::default();
        }
        match self.list_files(key) {
            Some(effect) => Reduction::default().effect(effect),
            None => Reduction::default(),
        }
    }

    fn list_files(&mut self, key: FileListKey) -> Option<CommandsEffect> {
        if self.file_pending.as_ref() == Some(&key) {
            return None;
        }
        self.file_generation += 1;
        self.file_pending = Some(key.clone());
        Some(CommandsEffect::ListFiles(Box::new(FileListRequest {
            generation: self.file_generation,
            repo_path: key.repo_path,
            revision: key.revision,
        })))
    }

    fn files_listed(
        &mut self,
        request: FileListRequest,
        files: Result<Arc<[String]>, String>,
    ) -> Reduction {
        let key = FileListKey {
            repo_path: request.repo_path,
            revision: request.revision,
        };
        if self.file_pending.as_ref() == Some(&key) {
            self.file_pending = None;
        }
        let showing = self.file_picker.as_ref() == Some(&key)
            && self
                .picker
                .as_ref()
                .is_some_and(|picker| picker.kind == PickerKind::File);
        let files = match files {
            Ok(files) => files,
            Err(message) if showing => {
                self.toasts.apply(ToastUpdate::error(format!(
                    "Couldn't list files: {message}"
                )));
                return Reduction::changed();
            }
            Err(_) => return Reduction::default(),
        };
        match self.file_lists.iter().position(|list| list.key == key) {
            Some(index) if self.file_lists[index].generation > request.generation => {
                return Reduction::default();
            }
            Some(index) if self.file_lists[index].files == files => {
                self.file_lists[index].generation = request.generation;
                return Reduction::default();
            }
            Some(index) => {
                self.file_lists.remove(index);
            }
            None => {}
        }
        self.file_lists.push(FileList {
            key,
            generation: request.generation,
            files: files.clone(),
        });
        if self.file_lists.len() > FILE_LIST_CACHE {
            self.file_lists.remove(0);
        }
        match self.picker.as_mut() {
            Some(picker) if showing => {
                picker.set_files(files);
                Reduction::changed()
            }
            _ => Reduction::default(),
        }
    }

    fn apply_repo_scan(&mut self) {
        let Some(scan) = self.repo_scan.clone() else {
            return;
        };
        let scanning = self.repo_scanning;
        let current = self.repo_picker_current.clone();
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        let Some(view) = picker.repos.as_mut() else {
            return;
        };
        let home = dirs::home_dir();
        let now = crate::core::ref_catalog::now_secs();
        view.set_discovered(&scan, home.as_deref(), now, current.as_deref());
        view.scanning = scanning;
        picker.refresh();
    }

    fn repo_group_key(&mut self, open: bool) -> Reduction {
        let Some(picker) = self.picker.as_mut() else {
            return Reduction::default();
        };
        let Some(row) = picker.rows.get(picker.selected) else {
            return Reduction::default();
        };
        let group = match (&row.choice, row.indent) {
            (PickerChoice::Group(group), _) => group.clone(),
            (PickerChoice::Repository(path), true) if !open => match &picker.repos {
                Some(view) => view
                    .discovered
                    .iter()
                    .find(|entry| &entry.path == path)
                    .map(|entry| entry.group.clone())
                    .unwrap_or_default(),
                None => return Reduction::default(),
            },
            _ => return Reduction::default(),
        };
        let Some(view) = picker.repos.as_mut() else {
            return Reduction::default();
        };
        view.set_open(&group, open);
        picker.refilter();
        if let Some(index) = picker
            .rows
            .iter()
            .position(|row| row.choice == PickerChoice::Group(group.clone()))
        {
            picker.selected = index;
        }
        let stride = self.row_stride;
        picker.reveal(stride);
        Reduction::changed()
    }

    fn open_ref_picker(&mut self, request: RefPickerRequest, cx: &SharedContext) -> Reduction {
        let Some(return_focus) = self.admit(LayerKind::RefPicker) else {
            return Reduction::default();
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        self.ref_picker = Some(RefPicker::new(request, now));
        self.push_layer(LayerKind::RefPicker, return_focus);
        let Some(repo) = cx.repository.as_ref() else {
            return Reduction::changed();
        };
        let identity = Arc::as_ptr(&repo.snapshot) as usize;
        if let Some((cached, catalog)) = &self.ref_catalog
            && *cached == identity
        {
            let catalog = catalog.clone();
            return self.install_ref_catalog(catalog);
        }
        self.ref_generation += 1;
        let recent = self
            .frecency
            .as_ref()
            .map(|store| {
                store
                    .recent(64)
                    .iter()
                    .filter_map(|key| key.strip_prefix(REF_FRECENCY_PREFIX))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(picker) = &mut self.ref_picker {
            picker.catalog_generation = self.ref_generation;
        }
        Reduction::changed().effect(CommandsEffect::LoadRefCatalog(Box::new(
            RefCatalogRequest {
                generation: self.ref_generation,
                repo_path: repo.snapshot.path.clone(),
                snapshot: repo.snapshot.clone(),
                recent,
                git: repo.snapshot.location.kind == VcsKind::GIT,
            },
        )))
    }

    fn install_ref_catalog(&mut self, catalog: Arc<RefCatalog>) -> Reduction {
        let Some(picker) = &mut self.ref_picker else {
            return Reduction::default();
        };
        picker.catalog = Some(catalog);
        self.filter_refs()
    }

    fn filter_refs(&mut self) -> Reduction {
        let Some(picker) = &mut self.ref_picker else {
            return Reduction::default();
        };
        let Some(catalog) = picker.catalog.clone() else {
            return Reduction::changed();
        };
        picker.query_dirty = false;
        self.ref_generation += 1;
        picker.filter_generation = self.ref_generation;
        Reduction::changed().effect(CommandsEffect::FilterRefs {
            generation: self.ref_generation,
            catalog,
            query: picker.query.text().to_owned(),
        })
    }

    fn request_diff_size(&mut self, cx: &SharedContext) -> Reduction {
        let Some(repo) = cx.repository.as_ref() else {
            return Reduction::default();
        };
        let Some(picker) = &mut self.ref_picker else {
            return Reduction::default();
        };
        let Some(entry) = picker.selected_entry() else {
            return Reduction::default();
        };
        if entry.kind == RefEntryKind::WorkingTree {
            return Reduction::default();
        }
        let chosen = entry.name.clone();
        let worktree = entry
            .worktree
            .as_ref()
            .filter(|w| w.dirty && picker.side() == CompareField::Right)
            .map(|w| w.path.clone());
        let other = match picker.other_name() {
            name if name.is_empty() || name == WORKING_TREE_LABEL => "HEAD".to_owned(),
            name => name.to_owned(),
        };
        let (left, right) = match picker.side() {
            CompareField::Left => (chosen, other),
            CompareField::Right => (other, chosen),
        };
        self.ref_generation += 1;
        picker.size_token = self.ref_generation;
        picker.size = None;
        Reduction::default().effect(CommandsEffect::LoadDiffSize(Box::new(DiffSizeRequest {
            token: self.ref_generation,
            repo_path: repo.snapshot.path.clone(),
            left,
            right,
            mode: picker.request.mode,
            worktree,
        })))
    }

    fn ref_picker_action(&mut self, action: RefPickerAction, cx: &SharedContext) -> Reduction {
        let Some(picker) = &mut self.ref_picker else {
            return Reduction::default();
        };
        let mut reduction = match action {
            RefPickerAction::ToggleSide => {
                let side = match picker.side() {
                    CompareField::Left => CompareField::Right,
                    CompareField::Right => CompareField::Left,
                };
                picker.switch_side(side);
                self.request_diff_size(cx)
            }
            RefPickerAction::SetSide(side) => {
                picker.switch_side(side);
                self.request_diff_size(cx)
            }
            RefPickerAction::Choose(row) => return self.choose_ref(Some(row), cx),
            RefPickerAction::SetMode(mode) => {
                picker.request.mode = mode;
                self.request(PlatformRequest::Dispatch(
                    NavigationAction::SetCompareMode(mode).into(),
                ));
                self.request_diff_size(cx)
            }
            RefPickerAction::SetEngine(renderer) => {
                picker.request.renderer = renderer;
                self.request(PlatformRequest::Dispatch(
                    PipelineAction::SetRenderer(renderer).into(),
                ));
                Reduction::default()
            }
            RefPickerAction::CopySha => match picker.selected_entry() {
                Some(entry) if !entry.oid.is_empty() => {
                    Reduction::default().effect(ShellEffect::SetClipboard(entry.oid.clone()))
                }
                _ => Reduction::default(),
            },
        };
        reduction.changed = true;
        reduction
    }

    fn choose_ref(&mut self, row: Option<usize>, cx: &SharedContext) -> Reduction {
        let Some(picker) = &self.ref_picker else {
            return Reduction::default();
        };
        let request = picker.request.clone();
        let choice = match row.and_then(|row| picker.entry(row)) {
            Some(entry) if is_working_tree(entry) => None,
            Some(entry) => Some(entry.name.clone()),
            None => match picker.query.text().trim() {
                "" => return Reduction::default(),
                typed => Some(typed.to_owned()),
            },
        };
        self.pop_layer(LayerKind::RefPicker);
        let Some(name) = choice else {
            self.request(PlatformRequest::Dispatch(
                NavigationAction::ShowWorkingTree.into(),
            ));
            return Reduction::changed();
        };
        if let Some(store) = &self.frecency {
            store.record_access(&format!("{REF_FRECENCY_PREFIX}{name}"));
        }
        let workdir = match cx.repository.as_ref() {
            Some(repo) if repo.snapshot.location.kind != VcsKind::GIT => "@",
            _ => WORKDIR_REF,
        };
        let mut refs = Vec::with_capacity(2);
        match (request.side, request.working_copy) {
            (CompareField::Left, true) => {
                refs.push((CompareField::Left, name));
                refs.push((CompareField::Right, workdir.to_owned()));
            }
            (CompareField::Right, true) => {
                refs.push((CompareField::Left, request.left));
                refs.push((CompareField::Right, name));
            }
            (field, false) => refs.push((field, name)),
        }
        for (field, value) in refs {
            self.request(PlatformRequest::Dispatch(
                NavigationAction::SetRef { field, value }.into(),
            ));
        }
        self.request(PlatformRequest::Dispatch(
            NavigationAction::StartCompare.into(),
        ));
        Reduction::changed()
    }

    fn open_theme_picker(&mut self) -> Reduction {
        let registry = crate::app::theme::registry();
        let current = &self.settings.settings.theme_name;
        let candidates = registry
            .names()
            .map(|name| {
                let detail = match registry.variant(name) {
                    crate::core::themes::ThemeVariant::Dual => "Light and dark",
                    crate::core::themes::ThemeVariant::Dark => "Dark",
                    crate::core::themes::ThemeVariant::Light => "Light",
                };
                PickerCandidate::new(name, PickerChoice::Theme(name.to_owned()))
                    .detail(detail)
                    .current(name == current)
            })
            .collect();
        self.open_picker(PickerKind::Theme, candidates)
    }

    fn font_entries(&self, role: FontRole) -> Vec<FontFamilyEntry> {
        match (&self.fonts, role) {
            (Some(catalog), FontRole::Ui) => catalog.ui.to_vec(),
            (Some(catalog), FontRole::Mono) => catalog.mono.to_vec(),
            (None, role) => bundled_font_entries(role),
        }
    }

    fn open_font_picker(&mut self, role: FontRole) -> Reduction {
        let current = self.settings.settings.fonts.family(role).to_owned();
        let candidates = picker::font_candidates(&self.font_entries(role), &current);
        let mut reduction = self.open_picker(PickerKind::Font(role), candidates);
        if self.fonts.is_none() && !self.font_catalog_requested {
            self.font_catalog_requested = true;
            reduction = reduction.effect(CommandsEffect::LoadFontCatalog);
        }
        reduction
    }

    fn confirm_picker(&mut self, cx: &SharedContext) -> Reduction {
        let Some(picker) = &self.picker else {
            return Reduction::default();
        };
        let kind = picker.kind;
        let Some(choice) = picker.selected_choice().cloned() else {
            return Reduction::default();
        };
        match choice {
            PickerChoice::Heading => return Reduction::default(),
            PickerChoice::Group(group) => {
                return self.reduce_action(CommandsAction::ToggleRepoGroup(group), cx);
            }
            _ => {}
        }
        self.pop_layer(LayerKind::Picker);
        let mut reduction = Reduction::changed();
        match (kind, choice) {
            (_, PickerChoice::Repository(path)) => {
                self.request(PlatformRequest::Dispatch(
                    NavigationAction::OpenRepository(path).into(),
                ));
            }
            (PickerKind::Ref(field), PickerChoice::Ref(value)) => {
                self.request(PlatformRequest::Dispatch(
                    NavigationAction::SetRef { field, value }.into(),
                ));
                self.request(PlatformRequest::Dispatch(
                    NavigationAction::StartCompare.into(),
                ));
            }
            (_, PickerChoice::Theme(name)) => {
                reduction.merge(self.reduce_action(CommandsAction::SetThemeName(name), cx));
            }
            (PickerKind::Font(FontRole::Ui), PickerChoice::Font(family)) => {
                reduction.merge(self.reduce_action(CommandsAction::SetUiFontFamily(family), cx));
            }
            (PickerKind::Font(FontRole::Mono), PickerChoice::Font(family)) => {
                reduction.merge(self.reduce_action(CommandsAction::SetMonoFontFamily(family), cx));
            }
            (_, PickerChoice::Action(action)) => reduction.merge(self.run(action, cx)),
            (_, PickerChoice::PullRequest(url)) => {
                self.request(PlatformRequest::Dispatch(
                    ReviewAction::OpenPullRequest { reference: url }.into(),
                ));
            }
            (_, PickerChoice::File(path)) => {
                if let Some(key) = self.file_picker.clone() {
                    self.request(PlatformRequest::Dispatch(
                        FilesAction::Open(FileAt {
                            path,
                            label: revision_label(&key.revision.id),
                            revision: key.revision,
                        })
                        .into(),
                    ));
                }
            }
            (
                _,
                PickerChoice::Ref(_)
                | PickerChoice::Font(_)
                | PickerChoice::Heading
                | PickerChoice::Group(_),
            ) => {}
        }
        reduction
    }

    fn open_settings(&mut self, section: Option<SettingsSection>) -> Reduction {
        if self.modal_open() {
            return Reduction::default();
        }
        while let Some(kind) = self.top_layer() {
            self.pop_layer(kind);
        }
        match &mut self.settings_page {
            Some(page) => {
                if let Some(section) = section
                    && page.section != section
                {
                    page.section = section;
                    page.scroll.set_offset(0.0, 0.0);
                }
                page.capture = None;
            }
            None => {
                self.settings_page = Some(SettingsPage {
                    section: section.unwrap_or_default(),
                    capture: None,
                    notice: None,
                    scroll: ScrollHandle::new(),
                    search: TextField::new(""),
                    return_focus: self.view_focus,
                });
            }
        }
        self.focus(Some(SETTINGS_SEARCH));
        self.prepare_section();
        Reduction::changed()
    }

    fn prepare_section(&self) {
        if self
            .settings_page
            .as_ref()
            .is_some_and(|page| page.section == SettingsSection::Clankers)
        {
            self.request(PlatformRequest::Dispatch(
                MutationsAction::ScanAiClis { force: false }.into(),
            ));
        }
    }

    fn close_settings(&mut self) -> Reduction {
        let Some(page) = self.settings_page.take() else {
            return Reduction::default();
        };
        self.focus(page.return_focus);
        Reduction::changed()
    }

    fn update_settings(&mut self, update: impl FnOnce(&mut Settings)) -> Reduction {
        let before = &self.settings.settings;
        let mut next = before.clone();
        update(&mut next);
        normalize(&mut next);
        if next == *before {
            return Reduction::default();
        }
        self.apply_platform_diff(before, &next);
        let snapshot = Rc::new(SettingsSnapshot {
            revision: self.settings.revision + 1,
            settings: next,
        });
        self.settings = snapshot.clone();
        Reduction::changed()
            .effect(CommandsEffect::SaveSettings(Box::new(
                snapshot.settings.clone(),
            )))
            .intent(Intent::SettingsChanged(snapshot))
    }

    fn apply_platform_diff(&self, before: &Settings, next: &Settings) {
        if before.theme_name != next.theme_name
            || before.theme_mode != next.theme_mode
            || before.ui_scale_pct != next.ui_scale_pct
        {
            self.request(PlatformRequest::Theme(Box::new(
                crate::app::theme::settings_theme(next),
            )));
        }
        if before.fonts != next.fonts {
            self.request(PlatformRequest::Fonts(next.fonts.clone()));
        }
        if before.keymap_overrides != next.keymap_overrides {
            self.install_keymap(next);
        } else if before.viewport.wrap_enabled != next.viewport.wrap_enabled
            || before.continuous_scroll != next.continuous_scroll
            || before.theme_mode != next.theme_mode
        {
            self.install_menus(next);
        }
    }

    fn install_keymap(&self, settings: &Settings) {
        let saved = &settings.keymap_overrides;
        self.request(PlatformRequest::KeyBindings(keymap::key_bindings(saved)));
        self.install_menus(settings);
    }

    fn install_menus(&self, settings: &Settings) {
        let saved = &settings.keymap_overrides;
        let commands = builtin_commands(settings);
        self.request(PlatformRequest::Menus(platform::menus(&commands, |c| {
            if c.id == ids::QUIT {
                return Some(QUIT_BINDING.to_owned());
            }
            c.shortcut.and_then(|s| keymap::first_binding(saved, s))
        })));
    }

    fn apply_keymap_binding(&mut self, command: ShortcutCommand, binding: String) -> Reduction {
        let binding = normalize_binding(&binding);
        let saved = self.settings.settings.keymap_overrides.clone();
        if let Some(entry) = keymap::conflict(&saved, command, &binding) {
            if let Some(page) = &mut self.settings_page {
                page.notice = Some(format!(
                    "{} is already used by {}",
                    quark_app::keymap::format_binding(&binding),
                    entry.description
                ));
            }
            return Reduction::changed();
        }
        if let Some(page) = &mut self.settings_page {
            page.capture = None;
            page.notice = None;
        }
        let mut reduction =
            self.update_settings(|s| set_override(&mut s.keymap_overrides, command, binding));
        reduction.changed = true;
        reduction
    }

    fn shortcut(&mut self, command: ShortcutCommand, cx: &SharedContext) -> Reduction {
        let mut noted = self.note_family(command);
        noted.merge(self.run_shortcut(command, cx));
        noted
    }

    fn run_shortcut(&mut self, command: ShortcutCommand, cx: &SharedContext) -> Reduction {
        if command == ShortcutCommand::GoPrefix {
            self.go_prefix = true;
            self.go_since.set(None);
            return Reduction::changed();
        }
        if command == ShortcutCommand::OpenRepository && self.launcher.is_some() {
            return self.reduce_action(CommandsAction::OpenRepositoryFolder, cx);
        }
        if let Some(external) = self.external.iter().find(|d| d.shortcut == Some(command)) {
            if !external.enabled {
                return Reduction::default();
            }
            let action = external.action.clone();
            return self.run(action, cx);
        }
        if let Some(action) = keymap::builtin_action(command) {
            return self.run(action, cx);
        }
        match command {
            ShortcutCommand::SettingsNextSection | ShortcutCommand::SettingsPreviousSection => {
                let delta = if command == ShortcutCommand::SettingsNextSection {
                    1
                } else {
                    -1
                };
                match &mut self.settings_page {
                    Some(page) => {
                        page.section = keymap::settings_section_step(page.section, delta);
                        page.capture = None;
                        Reduction::changed()
                    }
                    None => Reduction::default(),
                }
            }
            ShortcutCommand::ToggleFocus => {
                let target = if self.view_focus == Some(crate::app::text_targets::SIDEBAR) {
                    LogicalFocus::Viewport
                } else {
                    LogicalFocus::Sidebar
                };
                self.run(ShellAction::FocusLogical(Some(target)).into(), cx)
            }
            _ => Reduction::default(),
        }
    }

    fn open_context_menu(&mut self, entries: Vec<ContextMenuEntry>, x: i32, y: i32) -> Reduction {
        if entries.is_empty() {
            return Reduction::default();
        }
        let Some(return_focus) = self.admit(LayerKind::ContextMenu) else {
            return Reduction::default();
        };
        self.context_entries.clear();
        let mut items = Vec::with_capacity(entries.len());
        for entry in entries {
            match entry {
                ContextMenuEntry::Separator => {
                    items.push(quark_components::ContextMenuEntry::separator())
                }
                ContextMenuEntry::Item {
                    label,
                    action,
                    shortcut,
                    destructive,
                    disabled,
                } => {
                    let index = self.context_entries.len();
                    self.context_entries.push(*action);
                    let mut item = quark_components::ContextMenuEntry::item(
                        label,
                        Action::from(CommandsAction::SelectContextMenuEntry(index)),
                    )
                    .disabled_if(disabled);
                    if let Some(shortcut) = shortcut {
                        item = item.shortcut(shortcut);
                    }
                    if destructive {
                        item = item.destructive();
                    }
                    items.push(item);
                }
            }
        }
        self.context_menu.open(items, x as f32, y as f32);
        self.push_layer(LayerKind::ContextMenu, return_focus);
        Reduction::changed()
    }

    fn context_menu_key(&mut self, binding: &str, cx: &SharedContext) -> Reduction {
        let Ok(binding) = binding.parse::<Binding>() else {
            return Reduction::default();
        };
        match self.context_menu.handle_key(&binding) {
            None => {
                if binding.key == "escape" {
                    self.pop_layer(LayerKind::ContextMenu);
                    return Reduction::changed();
                }
                Reduction::default()
            }
            Some(ContextMenuOutcome::Handled) => Reduction::changed(),
            Some(ContextMenuOutcome::Closed) => {
                self.pop_layer(LayerKind::ContextMenu);
                Reduction::changed()
            }
            Some(ContextMenuOutcome::Activate(action)) => {
                let action = action.downcast_ref::<Action>().cloned();
                match action {
                    Some(action) => self.run(action, cx),
                    None => Reduction::changed(),
                }
            }
        }
    }

    fn startup(&mut self, snapshot: &Rc<SettingsSnapshot>, launcher: bool) -> Reduction {
        #[cfg(not(test))]
        {
            self.frecency = crate::core::frecency::open_default_store();
        }
        if launcher {
            self.focus(Some(LAUNCHER));
        }
        let settings = &snapshot.settings;
        self.request(PlatformRequest::Theme(Box::new(
            crate::app::theme::settings_theme(settings),
        )));
        self.request(PlatformRequest::Fonts(settings.fonts.clone()));
        self.install_keymap(settings);
        self.settings = snapshot.clone();
        let mut reduction = self.refresh_launcher();
        reduction.merge(self.scan_repositories());
        reduction
            .merge(self.update_settings(|s| s.tips.sessions = s.tips.sessions.saturating_add(1)));
        reduction
    }

    pub fn visible_tip(
        &self,
        context: tips::Context,
        now_ms: u64,
    ) -> Result<Option<&'static tips::Tip>, u64> {
        match self.tip.get() {
            TipSession::Showing(tip) => Ok(Some(tip)),
            TipSession::Done => Ok(None),
            TipSession::Pending => {
                let since = self.tip_since.get().unwrap_or(now_ms);
                self.tip_since.set(Some(since));
                let due = since + TIP_DELAY_MS;
                if now_ms < due {
                    return Err(due - now_ms);
                }
                let tip = tips::next_tip(&self.settings.settings.tips, context, tips::now_secs());
                self.tip
                    .set(tip.map_or(TipSession::Done, TipSession::Showing));
                Ok(tip)
            }
        }
    }

    pub fn hint_bar(&self) -> bool {
        tips::show_hint_bar(&self.settings.settings.tips)
    }

    fn note_family(&mut self, command: ShortcutCommand) -> Reduction {
        let Some(family) = tips::family_of(command) else {
            return Reduction::default();
        };
        let now = tips::now_secs();
        let mut reduction = Reduction::default();
        if let TipSession::Showing(tip) = self.tip.get()
            && tip.family == family
        {
            reduction.merge(self.dismiss_tip());
        }
        if tips::should_record_use(&self.settings.settings.tips, family, now) {
            reduction.merge(self.update_settings(|s| {
                s.tips.used.insert(family.key().to_owned(), now);
            }));
        }
        reduction
    }

    fn dismiss_tip(&mut self) -> Reduction {
        let TipSession::Showing(tip) = self.tip.replace(TipSession::Done) else {
            return Reduction::default();
        };
        let now = tips::now_secs();
        let mut reduction = self.update_settings(|s| {
            s.tips.shown.insert(tip.id.to_owned(), now);
        });
        reduction.changed = true;
        reduction
    }

    #[cfg(test)]
    pub fn set_frecency(&mut self, store: FrecencyStore) {
        self.frecency = Some(store);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileListKey {
    repo_path: PathBuf,
    revision: RevisionId,
}

#[derive(Debug)]
struct FileList {
    key: FileListKey,
    generation: u64,
    files: Arc<[String]>,
}

fn file_list_key(cx: &SharedContext) -> Option<FileListKey> {
    let (repo_path, revision) = head_revision(cx)?;
    Some(FileListKey {
        repo_path,
        revision,
    })
}

fn layer_focus(kind: LayerKind) -> FocusId {
    match kind {
        LayerKind::Palette => PALETTE_INPUT,
        LayerKind::Picker | LayerKind::RefPicker => PICKER,
        LayerKind::Confirmation => CONFIRM_CANCEL,
        LayerKind::ContextMenu => crate::app::text_targets::VIEWPORT,
        LayerKind::Shortcuts => crate::app::text_targets::SHORTCUTS,
    }
}

pub fn normalize(settings: &mut Settings) {
    settings.ui_scale_pct = settings
        .ui_scale_pct
        .clamp(MIN_UI_SCALE_PCT, MAX_UI_SCALE_PCT);
    settings.fonts = settings.fonts.normalized();
    if crate::app::theme::registry()
        .get(&settings.theme_name)
        .is_none()
    {
        settings.theme_name = crate::app::theme::DEFAULT_THEME.to_owned();
    }
    settings.wheel_scroll_lines = settings.wheel_scroll_lines.clamp(1, 10);
}

fn normalize_binding(binding: &str) -> String {
    let primary = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let mut parts: Vec<&str> = binding.split('+').collect();
    let key = parts.pop().unwrap_or_default();
    let mut out: Vec<&str> = parts
        .into_iter()
        .map(|part| if part == primary { "mod" } else { part })
        .collect();
    out.push(key);
    out.join("+")
}

fn set_override(overrides: &mut Vec<KeymapOverride>, command: ShortcutCommand, binding: String) {
    match overrides.iter_mut().find(|o| o.command == command) {
        Some(existing) => existing.binding = binding,
        None => overrides.push(KeymapOverride { command, binding }),
    }
}

fn step_scale(current: u16, delta: i32) -> u16 {
    (i32::from(current) + delta).clamp(i32::from(MIN_UI_SCALE_PCT), i32::from(MAX_UI_SCALE_PCT))
        as u16
}

fn command_detail(id: &str) -> Option<&'static str> {
    Some(match id {
        ids::OPEN_REPOSITORY => "Pick a repository or type a path",
        ids::GO_TO_FILE => "Open any file at the compared revision",
        ids::CLOSE => "Close the open file tab, or the window without tabs",
        ids::GO_TO_SYMBOL => "Jump to a definition in the open file",
        ids::GO_TO_WORKSPACE_SYMBOL => "Search every definition in the repository",
        ids::WORKING_TREE => "Compare HEAD with uncommitted changes",
        ids::REFRESH => "Reload refs, status and the current compare",
        "compare.base" => "Pick the ref to compare from",
        "compare.head" => "Pick the ref to compare to",
        "compare.swap" => "Exchange base and head",
        "compare.start" => "Run the compare with the chosen refs",
        "compare.menu" => "Mode, working tree and swap",
        "review.open_pull_request" => "Load a GitHub pull request by URL",
        ids::TOGGLE_SIDEBAR => "Show or hide the file list",
        ids::TOGGLE_FILE_TREE => "Switch between a flat list and folders",
        "view.smart_sort" => "List the most important changed files first",
        "view.expand_folders" => "Open every folder in the file tree",
        "view.collapse_folders" => "Close every folder in the file tree",
        "view.files_tab" => "Changed files in the sidebar",
        "view.history_tab" => "Commits in the sidebar",
        "focus.files" => "Move keyboard focus to the file list",
        "focus.diff" => "Move keyboard focus to the diff",
        ids::TOGGLE_WRAP => "Wrap long lines in the diff",
        "view.unified" => "Old and new lines in one column",
        "view.split" => "Old and new side by side",
        ids::TOGGLE_CONTINUOUS => "Show every file in one scrolling view",
        ids::TOGGLE_THEME_MODE => "Flip between light and dark",
        ids::THEME => "Browse every installed theme",
        "font.ui" => "Font for labels and menus",
        "font.mono" => "Font for diffs and editors",
        ids::SCALE_UP => "Make the interface larger",
        ids::SCALE_DOWN => "Make the interface smaller",
        ids::SCALE_RESET => "Interface at 100%",
        ids::SETTINGS => "Appearance, diff, keyboard and more",
        ids::KEYMAPS => "Every shortcut, rebindable",
        "updates.auto" => "Check for new versions in the background",
        ids::ABOUT => "Version and updates",
        ids::WEBSITE => "Open the project page",
        ids::QUIT => "Close every window and exit",
        _ => return None,
    })
}

pub fn builtin_commands(settings: &Settings) -> Vec<CommandDescriptor> {
    use CommandDescriptor as D;
    use ShortcutCommand as S;
    let other_mode = match settings.theme_mode {
        ThemeMode::Dark => "Switch to Light Mode",
        ThemeMode::Light | ThemeMode::System => "Switch to Dark Mode",
    };
    let name = crate::platform::startup::app_display_name();
    let mut commands = vec![
        D::new(
            ids::PALETTE,
            "Command Palette",
            CommandsAction::OpenCommandPalette,
        )
        .shortcut(S::OpenCommandPalette),
        D::new(
            ids::OPEN_REPOSITORY,
            "Open Repository…",
            CommandsAction::OpenRepoPicker { anchored: false },
        )
        .shortcut(S::OpenRepository),
        D::new(
            ids::GO_TO_FILE,
            "Go to File…",
            CommandsAction::OpenFilePicker,
        )
        .shortcut(S::GoToFile),
        D::new(ids::CLOSE, "Close", ShellAction::CloseTabOrWindow).shortcut(S::CloseTab),
        D::new(
            ids::GO_TO_SYMBOL,
            "Go to Symbol in File…",
            CommandsAction::OpenSymbolPicker { workspace: false },
        )
        .shortcut(S::GoToSymbol),
        D::new(
            ids::GO_TO_WORKSPACE_SYMBOL,
            "Go to Symbol in Workspace…",
            CommandsAction::OpenSymbolPicker { workspace: true },
        )
        .shortcut(S::GoToWorkspaceSymbol),
        D::new(
            ids::WORKING_TREE,
            "Show Working Tree",
            NavigationAction::ShowWorkingTree,
        ),
        D::new(ids::REFRESH, "Refresh", NavigationAction::RefreshRepository)
            .shortcut(S::RefreshView),
        D::new(
            "compare.base",
            "Choose Base Ref…",
            CommandsAction::OpenRefPicker(CompareField::Left),
        ),
        D::new(
            "compare.head",
            "Choose Head Ref…",
            CommandsAction::OpenRefPicker(CompareField::Right),
        ),
        D::new("compare.swap", "Swap Refs", NavigationAction::SwapRefs),
        D::new(
            "compare.uncommitted",
            "Toggle Uncommitted Changes",
            NavigationAction::ToggleCommittedOnly,
        ),
        D::new(
            "compare.start",
            "Start Compare",
            NavigationAction::StartCompare,
        ),
        D::new(
            "compare.menu",
            "Compare Options…",
            CommandsAction::OpenCompareMenu,
        )
        .shortcut(S::OpenCompareMenu),
        D::new(
            "review.open_pull_request",
            "Open Pull Request…",
            CommandsAction::OpenPullRequestPicker,
        ),
        D::new(
            ids::TOGGLE_SIDEBAR,
            "Toggle Sidebar",
            NavigationAction::ToggleSidebar,
        )
        .shortcut(S::ToggleSidebar),
        D::new(
            ids::TOGGLE_FILE_TREE,
            "Toggle File Tree",
            NavigationAction::ToggleSidebarMode,
        )
        .shortcut(S::ToggleFileTree),
        D::new(
            "view.smart_sort",
            "Toggle Smart Sort",
            NavigationAction::ToggleSmartSort,
        ),
        D::new(
            "view.expand_folders",
            "Expand All Folders",
            NavigationAction::ExpandAllFolders,
        )
        .shortcut(S::ExpandFolders),
        D::new(
            "view.collapse_folders",
            "Collapse All Folders",
            NavigationAction::CollapseAllFolders,
        )
        .shortcut(S::CollapseFolders),
        D::new(
            "view.files_tab",
            "Show Files",
            NavigationAction::SetSidebarTab(SidebarTab::Files),
        )
        .shortcut(S::SidebarFiles),
        D::new(
            "view.history_tab",
            "Show History",
            NavigationAction::SetSidebarTab(SidebarTab::Commits),
        )
        .shortcut(S::SidebarCommits),
        D::new(
            "focus.files",
            "Focus File List",
            ShellAction::FocusLogical(Some(LogicalFocus::Sidebar)),
        )
        .shortcut(S::FocusFileList),
        D::new(
            "focus.diff",
            "Focus Diff",
            ShellAction::FocusLogical(Some(LogicalFocus::Viewport)),
        )
        .shortcut(S::FocusEditor),
        D::new(
            ids::TOGGLE_WRAP,
            if settings.viewport.wrap_enabled {
                "Disable Line Wrapping"
            } else {
                "Enable Line Wrapping"
            },
            CommandsAction::ToggleWrap,
        )
        .shortcut(S::ToggleWrap),
        D::new(
            "view.unified",
            "Unified Diff View",
            CommandsAction::SetLayout(LayoutMode::Unified),
        )
        .shortcut(S::UnifiedView)
        .enabled(settings.viewport.layout != LayoutMode::Unified),
        D::new(
            "view.split",
            "Split Diff View",
            CommandsAction::SetLayout(LayoutMode::Split),
        )
        .shortcut(S::SplitView)
        .enabled(settings.viewport.layout != LayoutMode::Split),
        D::new(
            ids::TOGGLE_CONTINUOUS,
            if settings.continuous_scroll {
                "Disable Continuous Scroll"
            } else {
                "Enable Continuous Scroll"
            },
            CommandsAction::ToggleContinuousScroll,
        ),
        D::new(
            ids::TOGGLE_THEME_MODE,
            other_mode,
            CommandsAction::ToggleThemeMode,
        ),
        D::new(ids::THEME, "Change Theme…", CommandsAction::OpenThemePicker),
        D::new(
            "font.ui",
            "Change UI Font…",
            CommandsAction::OpenUiFontPicker,
        ),
        D::new(
            "font.mono",
            "Change Code Font…",
            CommandsAction::OpenMonoFontPicker,
        ),
        D::new(ids::SCALE_UP, "Zoom In", CommandsAction::IncreaseUiScale)
            .shortcut(S::IncreaseUiScale),
        D::new(ids::SCALE_DOWN, "Zoom Out", CommandsAction::DecreaseUiScale)
            .shortcut(S::DecreaseUiScale),
        D::new(
            ids::SCALE_RESET,
            "Actual Size",
            CommandsAction::SetUiScalePct(DEFAULT_UI_SCALE_PCT),
        ),
        D::new(ids::SETTINGS, "Settings…", CommandsAction::OpenSettings).shortcut(S::OpenSettings),
        D::new(
            ids::KEYMAPS,
            "Keyboard Shortcuts",
            CommandsAction::OpenKeymaps,
        )
        .shortcut(S::ShowKeymaps),
        D::new(
            "updates.auto",
            if settings.auto_update {
                "Disable Automatic Updates"
            } else {
                "Enable Automatic Updates"
            },
            CommandsAction::ToggleAutoUpdate,
        ),
        D::new(
            ids::ABOUT,
            format!("About {name}"),
            CommandsAction::ShowAbout,
        ),
        D::new(
            ids::WEBSITE,
            format!("{name} on GitHub"),
            CommandsAction::OpenProjectPage,
        ),
        D::new(ids::QUIT, format!("Quit {name}"), CommandsAction::Quit),
    ];
    for command in &mut commands {
        if command.detail.is_none() {
            command.detail = command_detail(command.id).map(str::to_owned);
        }
    }
    commands
}

impl Controller for CommandsController {
    type Action = CommandsAction;
    type Event = CommandsEvent;

    fn reduce_action(&mut self, action: Self::Action, cx: &SharedContext) -> Reduction {
        use CommandsAction as A;
        match action {
            A::OpenCommandPalette => self.open_palette(cx),
            A::OpenRepoPicker { anchored } => self.open_repo_picker(anchored, cx),
            A::OpenFilePicker => self.open_file_picker("", cx),
            A::OpenSymbolPicker { workspace } => {
                self.open_file_picker(if workspace { "#" } else { "@" }, cx)
            }
            A::ToggleRepoGroup(group) => {
                let Some(picker) = self.picker.as_mut() else {
                    return Reduction::default();
                };
                let Some(view) = picker.repos.as_mut() else {
                    return Reduction::default();
                };
                view.toggle(&group);
                picker.refresh();
                Reduction::changed()
            }
            A::RepoGroupKey(open) => self.repo_group_key(open),
            A::RescanRepositories => self.scan_repositories(),
            A::ChooseRepoScanFolder => {
                Reduction::changed().effect(ShellEffect::OpenScanFolderDialog)
            }
            A::AddRepoScanRoot(path) => {
                let home = dirs::home_dir();
                let root = match home
                    .as_deref()
                    .and_then(|home| path.strip_prefix(home).ok())
                {
                    Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
                    Some(rest) => format!("~/{}", rest.display()),
                    None => path.display().to_string(),
                };
                let mut reduction = self.update_settings(|s| {
                    if !s.repo_discovery.roots.contains(&root) {
                        s.repo_discovery.roots.push(root);
                    }
                });
                reduction.merge(self.scan_repositories());
                reduction
            }
            A::RemoveRepoScanRoot(root) => {
                let mut reduction =
                    self.update_settings(|s| s.repo_discovery.roots.retain(|r| *r != root));
                reduction.merge(self.scan_repositories());
                reduction
            }
            A::SetRepoScanDepth(depth) => {
                let mut reduction = self.update_settings(|s| s.repo_discovery.depth = depth);
                reduction.merge(self.scan_repositories());
                reduction
            }
            A::ToggleRepoScanHidden => {
                let mut reduction =
                    self.update_settings(|s| s.repo_discovery.hidden = !s.repo_discovery.hidden);
                reduction.merge(self.scan_repositories());
                reduction
            }
            A::LauncherMove(delta) => match &mut self.launcher {
                Some(launcher) => {
                    launcher.move_selection(delta);
                    Reduction::changed()
                }
                None => Reduction::default(),
            },
            A::LauncherSelect(index) => {
                let Some(launcher) = &mut self.launcher else {
                    return Reduction::default();
                };
                if index >= launcher.rows.len() {
                    return Reduction::default();
                }
                launcher.selected = index;
                self.focus(Some(LAUNCHER));
                self.confirm_launcher(cx)
            }
            A::LauncherConfirm => self.confirm_launcher(cx),
            A::LauncherClear => match &mut self.launcher {
                Some(launcher) if !launcher.query.text().is_empty() => {
                    launcher.query.set_text("");
                    launcher.refilter();
                    self.count_launcher_rows()
                }
                _ => Reduction::default(),
            },
            A::LauncherFolderChosen { folder, path } => self.launcher_folder(folder, path),
            A::OpenRepositoryFolder => {
                self.pop_layer(LayerKind::Picker);
                self.request(PlatformRequest::Dispatch(
                    NavigationAction::OpenRepositoryDialog.into(),
                ));
                Reduction::changed()
            }
            A::OpenRefPicker(field) => {
                self.request(PlatformRequest::Dispatch(
                    NavigationAction::ChooseRef(field).into(),
                ));
                Reduction::default()
            }
            A::ShowRefPicker(request) => self.open_ref_picker(*request, cx),
            A::RefPicker(action) => self.ref_picker_action(action, cx),
            A::OpenCompareMenu => {
                let mode = self
                    .settings
                    .settings
                    .last_compare
                    .as_ref()
                    .map(|c| c.mode)
                    .unwrap_or_default();
                self.open_picker(
                    PickerKind::CompareMenu,
                    picker::compare_menu_candidates(mode),
                )
            }
            A::OpenPullRequestPicker => self.open_picker(PickerKind::PullRequest, Vec::new()),
            A::OpenThemePicker => self.open_theme_picker(),
            A::OpenUiFontPicker => self.open_font_picker(FontRole::Ui),
            A::OpenMonoFontPicker => self.open_font_picker(FontRole::Mono),
            A::ShowKeyboardShortcuts => self.open_shortcuts(cx),
            A::OpenKeymaps => self.open_settings(Some(SettingsSection::Keymaps)),
            A::CloseOverlay => self.close_top(),
            A::BackdropDismiss => {
                if self.activation_click.replace(false) {
                    Reduction::default()
                } else {
                    self.close_top()
                }
            }
            A::DismissOverlays => self.dismiss_overlays(),
            A::MoveOverlaySelection(delta) if self.ref_picker.is_some() => {
                if let Some(picker) = &mut self.ref_picker {
                    picker.move_selection(delta, self.row_stride);
                }
                let mut reduction = self.request_diff_size(cx);
                reduction.changed = true;
                reduction
            }
            A::MoveOverlaySelection(delta) => match &mut self.picker {
                Some(picker) => {
                    picker.move_selection(delta, self.row_stride);
                    Reduction::changed()
                }
                None => Reduction::default(),
            },
            A::SelectOverlayEntry(row) if self.ref_picker.is_some() => {
                self.choose_ref(Some(row), cx)
            }
            A::ConfirmOverlaySelection if self.ref_picker.is_some() => {
                let row = self.ref_picker.as_ref().and_then(|p| p.selected);
                self.choose_ref(row, cx)
            }
            A::ScrollOverlay(lines) if self.ref_picker.is_some() => {
                if let Some(picker) = &mut self.ref_picker {
                    picker.scroll(lines, self.row_stride);
                }
                Reduction::changed()
            }
            A::SelectOverlayEntry(index) => {
                let in_range = self.picker.as_mut().is_some_and(|picker| {
                    let ok = index < picker.rows.len();
                    if ok {
                        picker.selected = index;
                    }
                    ok
                });
                if in_range {
                    self.confirm_picker(cx)
                } else {
                    Reduction::default()
                }
            }
            A::ConfirmOverlaySelection => self.confirm_picker(cx),
            A::TabCompletePickerDir => {
                if self
                    .picker
                    .as_mut()
                    .is_some_and(PickerState::complete_directory)
                {
                    Reduction::changed()
                } else {
                    Reduction::default()
                }
            }
            A::ScrollOverlay(lines) => match &mut self.picker {
                Some(picker) => {
                    picker.scroll(lines, self.row_stride);
                    Reduction::changed()
                }
                None => Reduction::default(),
            },
            A::Leader(binding) => self.leader(&binding, cx),
            A::DismissTip => self.dismiss_tip(),
            A::HideHintBar => self.update_settings(|s| s.tips.hint_bar_hidden = true),
            A::OverlayKey(binding) => match self.top_layer() {
                Some(LayerKind::Palette) => {
                    let outcome = binding
                        .parse::<Binding>()
                        .ok()
                        .and_then(|b| self.palette.handle_key(&b));
                    self.palette_outcome(outcome, cx)
                }
                Some(LayerKind::ContextMenu) => self.context_menu_key(&binding, cx),
                _ => Reduction::default(),
            },
            A::Palette(PaletteEvent::Dismiss) if self.activation_click.replace(false) => {
                Reduction::default()
            }
            A::Palette(event) => {
                self.activation_click.set(false);
                let outcome = self.palette.apply(event);
                self.palette_outcome(outcome, cx)
            }
            A::SetExternalCommands(commands) => {
                self.external = commands;
                Reduction::default()
            }
            A::RunCommand(id) => self.run_command(id, cx),
            A::Shortcut(command) => self.shortcut(command, cx),
            A::OpenContextMenu { entries, x, y } => self.open_context_menu(entries, x, y),
            A::SelectContextMenuEntry(index) => {
                let action = self.context_entries.get(index).cloned();
                self.pop_layer(LayerKind::ContextMenu);
                let mut reduction = Reduction::changed();
                if let Some(action) = action {
                    reduction.merge(self.run(action, cx));
                }
                reduction
            }
            A::CloseContextMenu => {
                if self.pop_layer(LayerKind::ContextMenu) {
                    Reduction::changed()
                } else {
                    Reduction::default()
                }
            }
            A::Confirm => {
                let Some(request) = self.confirmation.clone() else {
                    return Reduction::default();
                };
                self.pop_layer(LayerKind::Confirmation);
                if let Some(focus) = request.return_focus {
                    self.focus(Some(crate::app::text_targets::focus_id(focus)));
                }
                let mut reduction = Reduction::changed();
                reduction.merge(self.run(*request.confirm, cx));
                reduction
            }
            A::CancelConfirmation => {
                let return_focus = self.confirmation.as_ref().and_then(|r| r.return_focus);
                if !self.pop_layer(LayerKind::Confirmation) {
                    return Reduction::default();
                }
                if let Some(focus) = return_focus {
                    self.focus(Some(crate::app::text_targets::focus_id(focus)));
                }
                Reduction::changed()
            }
            A::DismissToast(id) => match self.toasts.take(id) {
                Some(item) => {
                    if let Some(expired) = item.expired {
                        self.request(PlatformRequest::Dispatch(expired));
                    }
                    Reduction::changed()
                }
                None => Reduction::default(),
            },
            A::UndoToast(id) => match self.toasts.take(id) {
                Some(item) => {
                    if let Some(undo) = item.undo {
                        self.request(PlatformRequest::Dispatch(undo));
                    }
                    Reduction::changed()
                }
                None => Reduction::default(),
            },
            A::OpenSettings => self.open_settings(None),
            A::CloseSettings => self.close_settings(),
            A::SetSettingsSection(section) => match &mut self.settings_page {
                Some(page) => {
                    if page.section != section {
                        page.section = section;
                        page.scroll.set_offset(0.0, 0.0);
                    }
                    page.capture = None;
                    page.notice = None;
                    self.prepare_section();
                    Reduction::changed()
                }
                None => self.open_settings(Some(section)),
            },
            A::ToggleWrap => {
                self.update_settings(|s| s.viewport.wrap_enabled = !s.viewport.wrap_enabled)
            }
            A::SetWrapColumn(column) => self.update_settings(|s| s.viewport.wrap_column = column),
            A::SetLayout(layout) => self.update_settings(|s| s.viewport.layout = layout),
            A::SetEngine(renderer) => {
                self.request(PlatformRequest::Dispatch(
                    PipelineAction::SetRenderer(renderer).into(),
                ));
                Reduction::changed()
            }
            A::SetSidebarWidth(width) => self.update_settings(|s| s.sidebar_width_px = Some(width)),
            A::IncreaseUiScale => self.update_settings(|s| {
                s.ui_scale_pct = step_scale(s.ui_scale_pct, i32::from(UI_SCALE_STEP_PCT))
            }),
            A::DecreaseUiScale => self.update_settings(|s| {
                s.ui_scale_pct = step_scale(s.ui_scale_pct, -i32::from(UI_SCALE_STEP_PCT))
            }),
            A::SetUiScalePct(pct) => self.update_settings(|s| s.ui_scale_pct = pct),
            A::ToggleThemeMode => self.update_settings(|s| {
                s.theme_mode = match s.theme_mode {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light | ThemeMode::System => ThemeMode::Dark,
                }
            }),
            A::SetThemeMode(mode) => self.update_settings(|s| s.theme_mode = mode),
            A::SetThemeName(name) => self.update_settings(|s| s.theme_name = name),
            A::SetUiFontFamily(family) => self.update_settings(|s| {
                s.fonts.ui_family = normalize_font_selection(FontRole::Ui, &family)
            }),
            A::SetMonoFontFamily(family) => self.update_settings(|s| {
                s.fonts.mono_family = normalize_font_selection(FontRole::Mono, &family)
            }),
            A::SetWheelScrollLines(lines) => self.update_settings(|s| s.wheel_scroll_lines = lines),
            A::ToggleContinuousScroll => {
                self.update_settings(|s| s.continuous_scroll = !s.continuous_scroll)
            }
            A::ToggleAutoUpdate => self.update_settings(|s| s.auto_update = !s.auto_update),
            A::SetAiSteeringPrompt(prompt) => {
                self.update_settings(|s| s.ai_steering_prompt = prompt)
            }
            A::SetAiProvider(provider) => self.update_settings(|s| s.ai_provider = provider),
            A::ToggleAiCli => self.update_settings(|s| s.ai_use_cli = !s.ai_use_cli),
            A::BeginKeymapRebind(command) => match &mut self.settings_page {
                Some(page) => {
                    page.capture = Some(command);
                    page.notice = None;
                    Reduction::changed()
                }
                None => Reduction::default(),
            },
            A::ApplyKeymapBinding { command, binding } => {
                self.apply_keymap_binding(command, binding)
            }
            A::ResetKeymapBinding(command) => {
                if let Some(page) = &mut self.settings_page {
                    page.capture = None;
                    page.notice = None;
                }
                let mut reduction =
                    self.update_settings(|s| s.keymap_overrides.retain(|o| o.command != command));
                reduction.changed = true;
                reduction
            }
            A::CancelKeymapRebind => match &mut self.settings_page {
                Some(page) if page.capture.is_some() => {
                    page.capture = None;
                    page.notice = None;
                    Reduction::changed()
                }
                _ => Reduction::default(),
            },
            A::ShowAbout => self.open_settings(Some(SettingsSection::Updates)),
            A::OpenProjectPage => Reduction::default().effect(ShellEffect::OpenBrowser {
                url: PROJECT_URL.to_owned(),
            }),
            A::Quit => {
                self.request(PlatformRequest::Quit);
                Reduction::default()
            }
        }
    }

    fn reduce_event(&mut self, event: Self::Event, cx: &SharedContext) -> Reduction {
        match event {
            CommandsEvent::SettingsSaved => Reduction::default(),
            CommandsEvent::SettingsSaveFailed { message } => {
                self.toasts.apply(ToastUpdate::error(format!(
                    "Settings were not saved: {message}"
                )));
                Reduction::changed()
            }
            CommandsEvent::RefCatalogReady {
                generation,
                catalog,
            } => {
                let current = self
                    .ref_picker
                    .as_ref()
                    .is_some_and(|p| p.catalog_generation == generation);
                if !current {
                    return Reduction::default();
                }
                if let Some(repo) = cx.repository.as_ref() {
                    self.ref_catalog =
                        Some((Arc::as_ptr(&repo.snapshot) as usize, catalog.clone()));
                }
                self.install_ref_catalog(catalog)
            }
            CommandsEvent::RefsFiltered {
                generation,
                results,
            } => match &mut self.ref_picker {
                Some(picker) if picker.filter_generation == generation => {
                    picker.set_results(results);
                    let mut reduction = self.request_diff_size(cx);
                    reduction.changed = true;
                    reduction
                }
                _ => Reduction::default(),
            },
            CommandsEvent::ToastExpired(job) => match self.toasts.take_by_job(job) {
                Some(item) => {
                    if let Some(expired) = item.expired {
                        self.request(PlatformRequest::Dispatch(expired));
                    }
                    Reduction::changed()
                }
                None => Reduction::default(),
            },
            CommandsEvent::DiffSizeReady { token, size } => match &mut self.ref_picker {
                Some(picker) if picker.size_token == token => {
                    picker.size = size;
                    Reduction::changed()
                }
                _ => Reduction::default(),
            },
            CommandsEvent::FilesListed { request, files } => self.files_listed(*request, files),
            CommandsEvent::RepositoriesScanned { scan, cached } => {
                if !cached {
                    self.repo_scanning = false;
                }
                if cached && self.repo_scan.is_some() {
                    return Reduction::default();
                }
                self.repo_scan = Some(scan);
                self.apply_repo_scan();
                self.refresh_launcher()
            }
            CommandsEvent::ChangesCounted { path, count } => {
                let mut shown = false;
                if let Some(picker) = self.picker.as_mut()
                    && let Some(view) = picker.repos.as_mut()
                    && view.worktrees.iter().any(|tree| tree.path == path)
                {
                    view.changed.insert(path.clone(), count.unwrap_or(0));
                    picker.refresh();
                    shown = true;
                }
                shown |= self
                    .launcher
                    .as_mut()
                    .is_some_and(|launcher| launcher.set_count(path, count));
                if shown {
                    Reduction::changed()
                } else {
                    Reduction::default()
                }
            }
            CommandsEvent::CloneProgress {
                job,
                received,
                total,
            } => {
                let Some(name) = self.clone_name(job) else {
                    return Reduction::default();
                };
                let message = format!("Cloning {name}: {received}/{total} objects\u{2026}");
                self.toasts.apply(ToastUpdate::Progress {
                    id: job,
                    message,
                    completed: received as u64,
                    total: total as u64,
                });
                Reduction::changed()
            }
            CommandsEvent::CloneFinished { job, path, result } => {
                let Some(index) = self.clones.iter().position(|(id, _)| *id == job) else {
                    return Reduction::default();
                };
                let (_, name) = self.clones.remove(index);
                let (kind, message, description) = match result {
                    Ok(()) => (ToastKind::Success, format!("Cloned {name}"), None),
                    Err(error) => (
                        ToastKind::Error,
                        format!("Couldn't clone {name}"),
                        Some(error),
                    ),
                };
                if kind == ToastKind::Success && self.launcher.is_some() {
                    self.request(PlatformRequest::Dispatch(
                        NavigationAction::OpenRepository(path).into(),
                    ));
                }
                self.toasts.apply(ToastUpdate::Show {
                    id: Some(job),
                    kind,
                    message,
                    description,
                });
                Reduction::changed()
            }
            CommandsEvent::FontCatalogReady { ui, mono } => {
                self.fonts = Some(FontCatalog { ui, mono });
                if let Some(PickerKind::Font(role)) = self.picker.as_ref().map(|p| p.kind) {
                    let current = self.settings.settings.fonts.family(role).to_owned();
                    let candidates = picker::font_candidates(&self.font_entries(role), &current);
                    if let Some(picker) = &mut self.picker {
                        picker.set_candidates(candidates);
                    }
                }
                Reduction::changed()
            }
        }
    }

    fn apply_intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        match intent {
            Intent::Startup(startup) => {
                self.startup(&cx.settings, startup.options.args.repo.is_none())
            }
            Intent::ActivateWorkspace { .. } => {
                if let Some(launcher) = &mut self.launcher {
                    launcher.reset();
                }
                if self.layers.is_empty() && self.view_focus == Some(LAUNCHER) {
                    self.view_focus = Some(crate::app::text_targets::SIDEBAR);
                    self.focus(self.view_focus);
                }
                Reduction::changed()
            }
            Intent::SettingsChanged(snapshot) => {
                if Rc::ptr_eq(snapshot, &self.settings) || **snapshot == *self.settings {
                    return Reduction::default();
                }
                let before = self.settings.settings.clone();
                self.apply_platform_diff(&before, &snapshot.settings);
                self.settings = snapshot.clone();
                Reduction::changed().effect(CommandsEffect::SaveSettings(Box::new(
                    snapshot.settings.clone(),
                )))
            }
            Intent::RepositoryChanged(repository) => {
                // A review cache is opened for a pull request, not chosen.
                if repository.snapshot.reason == RepositorySyncReason::Open
                    && !crate::platform::startup::is_review_cache(&repository.snapshot.path)
                    && let Some(store) = &self.frecency
                {
                    store.record_access(&format!("repo:{}", repository.snapshot.path.display()));
                }
                self.prefetch_files(cx)
            }
            Intent::PublishManifest(_) => self.prefetch_files(cx),
            Intent::Toast(update) => {
                let mut reduction = Reduction::changed();
                if let ToastUpdate::Undo { id, expires_ms, .. } = update {
                    reduction = reduction.effect(CommandsEffect::ExpireToast {
                        id: *id,
                        after_ms: *expires_ms,
                    });
                }
                self.toasts.apply(update.clone());
                reduction
            }
            Intent::ChooseDefinition(targets) => {
                let candidates = targets
                    .iter()
                    .map(|target| {
                        let (dir, name) =
                            target.path.rsplit_once('/').unwrap_or(("", &target.path));
                        let mut candidate = PickerCandidate::new(
                            format!("{name}:{}", target.line + 1),
                            PickerChoice::Action(SymbolsAction::Jump(target.clone()).into()),
                        )
                        .detail(dir);
                        if let Some(kind) = target.kind {
                            candidate = candidate.meta(outline::kind_label(kind));
                        }
                        candidate
                    })
                    .collect();
                self.show_picker(PickerState::new(PickerKind::Definition, candidates))
            }
            Intent::RequestConfirmation(request) => {
                let Some(return_focus) = self.admit(LayerKind::Confirmation) else {
                    return Reduction::default();
                };
                self.confirmation = Some(request.clone());
                self.push_layer(LayerKind::Confirmation, return_focus);
                Reduction::changed()
            }
            _ => Reduction::default(),
        }
    }
}

impl TextHost for CommandsController {
    fn owns_text(&self, target: FocusId) -> bool {
        target == PALETTE_INPUT
            || target == PICKER
            || target == SETTINGS_SEARCH
            || (target == LAUNCHER && self.launcher.is_some())
    }

    fn after_edit(&mut self, target: FocusId, _cx: &SharedContext) -> Reduction {
        if target == PICKER && self.ref_picker.as_ref().is_some_and(|p| p.query_dirty) {
            return self.filter_refs();
        }
        if target == LAUNCHER {
            return self.count_launcher_rows();
        }
        Reduction::default()
    }

    fn edit_text(
        &mut self,
        target: FocusId,
        command: TextEditCommand,
        cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        if target == PALETTE_INPUT {
            return Some(self.palette.edit(command, cx.now_ms));
        }
        if target == SETTINGS_SEARCH {
            let page = self.settings_page.as_mut()?;
            let outcome = page.search.apply_at(command, cx.now_ms);
            if outcome.text_changed {
                page.scroll.set_offset(0.0, 0.0);
            }
            return Some(outcome);
        }
        if target == LAUNCHER {
            let launcher = self.launcher.as_mut()?;
            let outcome = launcher.query.apply_at(command, cx.now_ms);
            if outcome.text_changed {
                launcher.refilter();
            }
            return Some(outcome);
        }
        if let Some(picker) = &mut self.ref_picker {
            let outcome = picker.query.apply_at(command, cx.now_ms);
            picker.query_dirty |= outcome.text_changed;
            return Some(outcome);
        }
        let picker = self.picker.as_mut()?;
        let outcome = picker.query.apply_at(command, cx.now_ms);
        if outcome.text_changed {
            picker.refilter();
        }
        Some(outcome)
    }

    fn set_preedit(
        &mut self,
        target: FocusId,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> bool {
        if target == PALETTE_INPUT {
            self.palette.set_preedit(text, cursor);
            return true;
        }
        if target == SETTINGS_SEARCH {
            return match &mut self.settings_page {
                Some(page) => {
                    page.search.set_preedit(text, cursor);
                    true
                }
                None => false,
            };
        }
        if target == LAUNCHER {
            return match &mut self.launcher {
                Some(launcher) => {
                    launcher.query.set_preedit(text, cursor);
                    true
                }
                None => false,
            };
        }
        if let Some(picker) = &mut self.ref_picker {
            picker.query.set_preedit(text, cursor);
            return true;
        }
        match &mut self.picker {
            Some(picker) => {
                picker.query.set_preedit(text, cursor);
                true
            }
            None => false,
        }
    }

    fn set_text_value(
        &mut self,
        target: FocusId,
        value: String,
        _cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        if target == PALETTE_INPUT {
            self.palette.set_query(&value);
            return Some(TextEditOutcome {
                text_changed: true,
                ..TextEditOutcome::default()
            });
        }
        if target == SETTINGS_SEARCH {
            let page = self.settings_page.as_mut()?;
            let changed = page.search.text() != value;
            page.search.set_text(value);
            return Some(TextEditOutcome {
                text_changed: changed,
                selection_changed: true,
                ..TextEditOutcome::default()
            });
        }
        if target == LAUNCHER {
            let launcher = self.launcher.as_mut()?;
            let changed = launcher.query.text() != value;
            launcher.query.set_text(value);
            if changed {
                launcher.refilter();
            }
            return Some(TextEditOutcome {
                text_changed: changed,
                selection_changed: true,
                ..TextEditOutcome::default()
            });
        }
        if let Some(picker) = &mut self.ref_picker {
            let changed = picker.query.text() != value;
            picker.query.set_text(value);
            picker.query_dirty |= changed;
            return Some(TextEditOutcome {
                text_changed: changed,
                selection_changed: true,
                ..TextEditOutcome::default()
            });
        }
        let picker = self.picker.as_mut()?;
        let changed = picker.query.text() != value;
        picker.query.set_text(value);
        picker.refilter();
        Some(TextEditOutcome {
            text_changed: changed,
            selection_changed: true,
            ..TextEditOutcome::default()
        })
    }
}
