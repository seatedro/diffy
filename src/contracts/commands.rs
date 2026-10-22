use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use quark_components::PaletteEvent;

use crate::contracts::navigation::{CompareField, RepositorySnapshot};
use crate::contracts::shared::{JobId, SharedContext};
use crate::contracts::shell::{Action, LogicalFocus};
use crate::contracts::viewport::pipeline::{DiffSize, DiffSizeRequest};
use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::core::keymap::ShortcutCommand;
use crate::core::ref_catalog::{RefCatalog, RefResults};
use crate::core::repo_discovery::RepoScan;
use crate::core::theme_types::ThemeMode;
use crate::core::vcs::model::RevisionId;
use crate::fonts::FontFamilyEntry;
use crate::platform::persistence::{RepoDiscovery, Settings};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SettingsSection {
    #[default]
    Appearance,
    Editor,
    Behavior,
    Keymaps,
    Clankers,
    Accounts,
    Updates,
}

impl SettingsSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Editor => "Diff",
            Self::Behavior => "Behavior",
            Self::Keymaps => "Keyboard",
            Self::Clankers => "AI",
            Self::Accounts => "Accounts",
            Self::Updates => "Updates",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Appearance => "How Diffy looks. Changes apply as you make them.",
            Self::Editor => "How diffs are computed and laid out.",
            Self::Behavior => "How Diffy behaves in the background.",
            Self::Keymaps => "Every shortcut. Click a binding to change it.",
            Self::Clankers => "Commit messages written by a model you choose.",
            Self::Accounts => "Services Diffy signs in to.",
            Self::Updates => "The installed version and how it updates.",
        }
    }

    pub const ALL: [Self; 7] = [
        Self::Appearance,
        Self::Editor,
        Self::Behavior,
        Self::Keymaps,
        Self::Clankers,
        Self::Accounts,
        Self::Updates,
    ];
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContextMenuEntry {
    Item {
        label: String,
        action: Box<Action>,
        shortcut: Option<String>,
        destructive: bool,
        disabled: bool,
    },
    Separator,
}

impl ContextMenuEntry {
    pub fn item(label: impl Into<String>, action: impl Into<Action>) -> Self {
        Self::Item {
            label: label.into(),
            action: Box::new(action.into()),
            shortcut: None,
            destructive: false,
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandDescriptor {
    pub id: &'static str,
    pub title: String,
    pub detail: Option<String>,
    pub action: Action,
    pub shortcut: Option<ShortcutCommand>,
    pub enabled: bool,
}

impl CommandDescriptor {
    pub fn new(id: &'static str, title: impl Into<String>, action: impl Into<Action>) -> Self {
        Self {
            id,
            title: title.into(),
            detail: None,
            action: action.into(),
            shortcut: None,
            enabled: true,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn shortcut(mut self, command: ShortcutCommand) -> Self {
        self.shortcut = Some(command);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

pub trait CommandSource {
    fn commands(&self, _cx: &SharedContext, _out: &mut Vec<CommandDescriptor>) {}
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommandsAction {
    OpenCommandPalette,
    OpenRepoPicker {
        anchored: bool,
    },
    OpenFilePicker,
    OpenSymbolPicker {
        workspace: bool,
    },
    OpenRepositoryFolder,
    ToggleRepoGroup(String),
    RepoGroupKey(bool),
    RescanRepositories,
    ChooseRepoScanFolder,
    AddRepoScanRoot(PathBuf),
    RemoveRepoScanRoot(String),
    SetRepoScanDepth(u8),
    ToggleRepoScanHidden,
    OpenRefPicker(CompareField),
    ShowRefPicker(Box<RefPickerRequest>),
    RefPicker(RefPickerAction),
    OpenCompareMenu,
    OpenPullRequestPicker,
    OpenThemePicker,
    OpenUiFontPicker,
    OpenMonoFontPicker,
    ShowKeyboardShortcuts,
    CloseOverlay,
    BackdropDismiss,
    DismissOverlays,
    MoveOverlaySelection(i32),
    SelectOverlayEntry(usize),
    ConfirmOverlaySelection,
    TabCompletePickerDir,
    ScrollOverlay(i32),
    OverlayKey(String),
    Palette(PaletteEvent),
    SetExternalCommands(Rc<[CommandDescriptor]>),
    RunCommand(&'static str),
    Shortcut(ShortcutCommand),
    Leader(String),
    OpenContextMenu {
        entries: Vec<ContextMenuEntry>,
        x: i32,
        y: i32,
    },
    SelectContextMenuEntry(usize),
    CloseContextMenu,
    Confirm,
    CancelConfirmation,
    DismissToast(u64),
    UndoToast(u64),
    OpenSettings,
    OpenKeymaps,
    CloseSettings,
    SetSettingsSection(SettingsSection),
    ToggleWrap,
    SetLayout(LayoutMode),
    SetEngine(RendererKind),
    SetWrapColumn(u32),
    SetSidebarWidth(u32),
    IncreaseUiScale,
    DecreaseUiScale,
    SetUiScalePct(u16),
    ToggleThemeMode,
    SetThemeMode(ThemeMode),
    SetThemeName(String),
    SetUiFontFamily(String),
    SetMonoFontFamily(String),
    SetWheelScrollLines(u8),
    ToggleContinuousScroll,
    ToggleAutoUpdate,
    SetAiSteeringPrompt(String),
    SetAiProvider(Option<crate::ai::Provider>),
    ToggleAiCli,
    BeginKeymapRebind(ShortcutCommand),
    ApplyKeymapBinding {
        command: ShortcutCommand,
        binding: String,
    },
    ResetKeymapBinding(ShortcutCommand),
    CancelKeymapRebind,
    ShowAbout,
    OpenProjectPage,
    Quit,
}

impl CommandsAction {
    pub fn blocked_by_modal(&self) -> bool {
        matches!(
            self,
            Self::OpenCommandPalette
                | Self::OpenRepoPicker { .. }
                | Self::OpenFilePicker
                | Self::OpenSymbolPicker { .. }
                | Self::OpenRefPicker(_)
                | Self::ShowRefPicker(_)
                | Self::OpenCompareMenu
                | Self::OpenPullRequestPicker
                | Self::OpenThemePicker
                | Self::OpenUiFontPicker
                | Self::OpenMonoFontPicker
                | Self::ShowKeyboardShortcuts
                | Self::Shortcut(_)
                | Self::OpenContextMenu { .. }
                | Self::OpenSettings
                | Self::OpenKeymaps
                | Self::ShowAbout
        )
    }

    pub fn wants_external_commands(&self) -> bool {
        matches!(self, Self::OpenCommandPalette | Self::Shortcut(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandsEffect {
    SaveSettings(Box<Settings>),
    ScanRepositories {
        options: Box<RepoDiscovery>,
        known: Vec<PathBuf>,
    },
    LoadFontCatalog,
    LoadRefCatalog(Box<RefCatalogRequest>),
    FilterRefs {
        generation: u64,
        catalog: Arc<RefCatalog>,
        query: String,
    },
    LoadDiffSize(Box<DiffSizeRequest>),
    ListFiles(Box<FileListRequest>),
    ExpireToast {
        id: JobId,
        after_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileListRequest {
    pub generation: u64,
    pub repo_path: PathBuf,
    pub revision: RevisionId,
}

#[derive(Debug, Clone)]
pub struct RefCatalogRequest {
    pub generation: u64,
    pub repo_path: PathBuf,
    pub snapshot: Arc<RepositorySnapshot>,
    pub recent: Vec<String>,
    pub git: bool,
}

impl PartialEq for RefCatalogRequest {
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.repo_path == other.repo_path
            && Arc::ptr_eq(&self.snapshot, &other.snapshot)
            && self.recent == other.recent
    }
}

impl Eq for RefCatalogRequest {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefPickerRequest {
    pub side: CompareField,
    pub left: String,
    pub right: String,
    pub working_copy: bool,
    pub mode: CompareMode,
    pub renderer: RendererKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefPickerAction {
    ToggleSide,
    SetSide(CompareField),
    Choose(usize),
    SetMode(CompareMode),
    SetEngine(RendererKind),
    CopySha,
}

#[derive(Debug, Clone)]
pub enum CommandsEvent {
    SettingsSaved,
    SettingsSaveFailed {
        message: String,
    },
    FontCatalogReady {
        ui: &'static [FontFamilyEntry],
        mono: &'static [FontFamilyEntry],
    },
    RefCatalogReady {
        generation: u64,
        catalog: Arc<RefCatalog>,
    },
    RefsFiltered {
        generation: u64,
        results: Arc<RefResults>,
    },
    DiffSizeReady {
        token: u64,
        size: Option<DiffSize>,
    },
    ToastExpired(JobId),
    FilesListed {
        request: Box<FileListRequest>,
        files: Result<Arc<[String]>, String>,
    },
    RepositoriesScanned {
        scan: Arc<RepoScan>,
        cached: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsSnapshot {
    pub revision: u64,
    pub settings: Settings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmationRequest {
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub destructive: bool,
    pub confirm: Box<Action>,
    pub return_focus: Option<LogicalFocus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToastUpdate {
    Show {
        id: Option<JobId>,
        kind: ToastKind,
        message: String,
        description: Option<String>,
    },
    Progress {
        id: JobId,
        message: String,
        completed: u64,
        total: u64,
    },
    Dismiss(JobId),
    Undo {
        id: JobId,
        kind: ToastKind,
        message: String,
        undo: Box<Action>,
        expires_ms: u64,
        expired: Option<Box<Action>>,
    },
}

impl ToastUpdate {
    pub fn error(message: impl Into<String>) -> Self {
        Self::show(ToastKind::Error, message)
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self::show(ToastKind::Info, message)
    }

    pub fn success(message: impl Into<String>) -> Self {
        Self::show(ToastKind::Success, message)
    }

    fn show(kind: ToastKind, message: impl Into<String>) -> Self {
        Self::Show {
            id: None,
            kind,
            message: message.into(),
            description: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::shell::{Action, AppEvent, Effect};

    #[test]
    fn routes_through_root_contracts() {
        assert_eq!(
            Action::from(CommandsAction::ToggleWrap),
            Action::Commands(CommandsAction::ToggleWrap)
        );
        assert_eq!(
            Effect::from(CommandsEffect::SaveSettings(Box::default())),
            Effect::Commands(CommandsEffect::SaveSettings(Box::default()))
        );
        assert!(matches!(
            AppEvent::from(CommandsEvent::SettingsSaved),
            AppEvent::Commands(CommandsEvent::SettingsSaved)
        ));
    }
}
