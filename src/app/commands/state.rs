use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use quark_components::{
    CommandPalette, ContextMenuOutcome, ContextMenuState, PALETTE_INPUT, PaletteOutcome,
};
use quark_ui::FocusId;
use quark_ui::element::{Binding, ScrollHandle};
use quark_ui::text_input::{TextEditCommand, TextEditOutcome};

use super::keymap;
use super::palette::{self, PaletteCommand, PaletteSource};
use super::picker::{self, PickerCandidate, PickerChoice, PickerKind, PickerState};
use super::platform::{self, PlatformRequest, ids};
use super::ref_picker::{RefPicker, is_working_tree};
use super::toasts::ToastHost;
use crate::app::text_targets::{PICKER, SETTINGS, TextCx, TextHost};
use crate::contracts::commands::{
    CommandDescriptor, CommandsAction, CommandsEffect, CommandsEvent, ConfirmationRequest,
    ContextMenuEntry, RefCatalogRequest, RefPickerAction, RefPickerRequest, SettingsSection,
    SettingsSnapshot, ToastUpdate,
};
use crate::contracts::navigation::{
    CompareField, NavigationAction, RepositorySyncReason, SidebarTab,
};
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{
    Action, Controller, Intent, LogicalFocus, Reduction, ShellAction, ShellEffect,
};
use crate::contracts::viewport::pipeline::{DiffSizeRequest, PipelineAction};
use crate::core::compare::LayoutMode;
use crate::core::frecency::{FrecencyStore, recent_repo_paths};
use crate::core::keymap::{KeymapOverride, ShortcutCommand};
use crate::core::ref_catalog::{RefCatalog, RefEntryKind, WORKING_TREE_LABEL};
use crate::core::theme_types::ThemeMode;
use crate::core::vcs::git::WORKDIR_REF;
use crate::core::vcs::model::VcsKind;
use crate::fonts::{FontFamilyEntry, FontRole, bundled_font_entries, normalize_font_selection};
use crate::platform::persistence::Settings;

pub const CONFIRM_CANCEL: FocusId = FocusId::from_key("diffy.confirm.cancel");
pub const CONFIRM_ACCEPT: FocusId = FocusId::from_key("diffy.confirm.accept");

pub const MIN_UI_SCALE_PCT: u16 = 70;
pub const MAX_UI_SCALE_PCT: u16 = 180;
pub const UI_SCALE_STEP_PCT: u16 = 10;
pub const DEFAULT_UI_SCALE_PCT: u16 = 100;
const RECENT_REPOSITORIES: usize = 12;
const RECENT_COMMANDS: usize = 8;
const PROJECT_URL: &str = "https://github.com/seatedro/diffy";
const QUIT_BINDING: &str = "mod+q";
const REF_FRECENCY_PREFIX: &str = "ref:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    Palette,
    Picker,
    Confirmation,
    ContextMenu,
    RefPicker,
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
    row_stride: f32,
    outbox: RefCell<Vec<PlatformRequest>>,
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
        let mut palette =
            CommandPalette::new().placeholder("Search commands, files and repositories");
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
            row_stride: 0.0,
            outbox: RefCell::new(Vec::new()),
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

    pub fn ref_picker(&self) -> Option<&RefPicker> {
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

    pub(super) fn capturing(&self) -> Option<ShortcutCommand> {
        self.settings_page.as_ref().and_then(|page| page.capture)
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
        self.palette_source.commands = self
            .commands()
            .into_iter()
            .filter(|descriptor| descriptor.id != ids::PALETTE)
            .map(|descriptor| PaletteCommand {
                binding: descriptor
                    .shortcut
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
        if self.has_layer(LayerKind::Picker) {
            self.picker = Some(PickerState::new(kind, candidates));
            self.focus(Some(PICKER));
            return Reduction::changed();
        }
        let Some(return_focus) = self.admit(LayerKind::Picker) else {
            return Reduction::default();
        };
        self.picker = Some(PickerState::new(kind, candidates));
        self.push_layer(LayerKind::Picker, return_focus);
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
        let default = crate::app::theme::DEFAULT_THEME;
        let builtin = registry.get(default).is_none().then_some(default);
        let candidates = builtin
            .into_iter()
            .chain(registry.names())
            .map(|name| {
                let detail = match registry.variant(name) {
                    _ if builtin == Some(name) => "Light and dark",
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
                reduction = reduction.intent(Intent::OpenPullRequest { url });
            }
            (_, PickerChoice::Ref(_) | PickerChoice::Font(_)) => {}
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
                    return_focus: self.view_focus,
                });
            }
        }
        self.focus(Some(SETTINGS));
        Reduction::changed()
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
        }
    }

    fn install_keymap(&self, settings: &Settings) {
        let saved = &settings.keymap_overrides;
        self.request(PlatformRequest::KeyBindings(keymap::key_bindings(saved)));
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

    fn startup(&mut self, settings: &Settings) -> Reduction {
        let mut settings = settings.clone();
        normalize(&mut settings);
        #[cfg(not(test))]
        {
            self.frecency = crate::core::frecency::open_default_store();
        }
        self.request(PlatformRequest::Theme(Box::new(
            crate::app::theme::settings_theme(&settings),
        )));
        self.request(PlatformRequest::Fonts(settings.fonts.clone()));
        self.install_keymap(&settings);
        let snapshot = Rc::new(SettingsSnapshot {
            revision: self.settings.revision + 1,
            settings,
        });
        self.settings = snapshot.clone();
        Reduction::changed().intent(Intent::SettingsChanged(snapshot))
    }

    #[cfg(test)]
    pub fn set_frecency(&mut self, store: FrecencyStore) {
        self.frecency = Some(store);
    }
}

fn layer_focus(kind: LayerKind) -> FocusId {
    match kind {
        LayerKind::Palette => PALETTE_INPUT,
        LayerKind::Picker | LayerKind::RefPicker => PICKER,
        LayerKind::Confirmation => CONFIRM_CANCEL,
        LayerKind::ContextMenu => crate::app::text_targets::VIEWPORT,
    }
}

fn normalize(settings: &mut Settings) {
    settings.ui_scale_pct = settings
        .ui_scale_pct
        .clamp(MIN_UI_SCALE_PCT, MAX_UI_SCALE_PCT);
    settings.fonts = settings.fonts.normalized();
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

pub fn builtin_commands(settings: &Settings) -> Vec<CommandDescriptor> {
    use CommandDescriptor as D;
    use ShortcutCommand as S;
    let other_mode = match settings.theme_mode {
        ThemeMode::Dark => "Switch to Light Mode",
        ThemeMode::Light => "Switch to Dark Mode",
    };
    let name = crate::platform::startup::app_display_name();
    vec![
        D::new(
            ids::PALETTE,
            "Command Palette",
            CommandsAction::OpenCommandPalette,
        )
        .shortcut(S::OpenCommandPalette),
        D::new(
            ids::OPEN_REPOSITORY,
            "Open Repository…",
            NavigationAction::OpenRepositoryDialog,
        ),
        D::new(
            ids::SWITCH_REPOSITORY,
            "Switch Repository…",
            CommandsAction::OpenRepoPicker,
        ),
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
    ]
}

impl Controller for CommandsController {
    type Action = CommandsAction;
    type Event = CommandsEvent;

    fn reduce_action(&mut self, action: Self::Action, cx: &SharedContext) -> Reduction {
        use CommandsAction as A;
        match action {
            A::OpenCommandPalette => self.open_palette(cx),
            A::OpenRepoPicker => {
                let current = cx.repository.as_ref().map(|r| r.snapshot.path.clone());
                let mut recent: Vec<PathBuf> =
                    recent_repo_paths(self.frecency.as_ref(), RECENT_REPOSITORIES);
                if let Some(current) = &current
                    && !recent.contains(current)
                {
                    recent.insert(0, current.clone());
                }
                let candidates = picker::repository_candidates(&recent, current.as_deref());
                self.open_picker(PickerKind::Repository, candidates)
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
            A::ShowKeyboardShortcuts | A::OpenKeymaps => {
                self.open_settings(Some(SettingsSection::Keymaps))
            }
            A::CloseOverlay => self.close_top(),
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
            A::Palette(event) => {
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
            A::DismissToast(id) => {
                if self.toasts.dismiss(id) {
                    Reduction::changed()
                } else {
                    Reduction::default()
                }
            }
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
                    Reduction::changed()
                }
                None => self.open_settings(Some(section)),
            },
            A::ToggleWrap => {
                self.update_settings(|s| s.viewport.wrap_enabled = !s.viewport.wrap_enabled)
            }
            A::SetWrapColumn(column) => self.update_settings(|s| s.viewport.wrap_column = column),
            A::SetLayout(layout) => self.update_settings(|s| s.viewport.layout = layout),
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
                    ThemeMode::Light => ThemeMode::Dark,
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
            A::ShowAbout => self.open_settings(Some(SettingsSection::About)),
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
            CommandsEvent::DiffSizeReady { token, size } => match &mut self.ref_picker {
                Some(picker) if picker.size_token == token => {
                    picker.size = size;
                    Reduction::changed()
                }
                _ => Reduction::default(),
            },
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

    fn apply_intent(&mut self, intent: &Intent, _cx: &SharedContext) -> Reduction {
        match intent {
            Intent::Startup(startup) => self.startup(&startup.settings),
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
                if repository.snapshot.reason == RepositorySyncReason::Open
                    && let Some(store) = &self.frecency
                {
                    store.record_access(&format!("repo:{}", repository.snapshot.path.display()));
                }
                Reduction::default()
            }
            Intent::Toast(update) => {
                self.toasts.apply(update.clone());
                Reduction::changed()
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
        target == PALETTE_INPUT || target == PICKER
    }

    fn after_edit(&mut self, target: FocusId, _cx: &SharedContext) -> Reduction {
        if target == PICKER && self.ref_picker.as_ref().is_some_and(|p| p.query_dirty) {
            return self.filter_refs();
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
