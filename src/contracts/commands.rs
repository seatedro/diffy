use std::rc::Rc;
use std::sync::Arc;

use quark_components::PaletteEvent;

use crate::contracts::navigation::CompareField;
use crate::contracts::shared::{JobId, SharedContext};
use crate::contracts::shell::{Action, LogicalFocus};
use crate::core::keymap::ShortcutCommand;
use crate::core::theme_types::ThemeMode;
use crate::fonts::FontFamilyEntry;
use crate::platform::persistence::Settings;

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

    pub const ALL: [Self; 6] = [
        Self::Appearance,
        Self::Editor,
        Self::Behavior,
        Self::Keymaps,
        Self::Clankers,
        Self::About,
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
    OpenRepoPicker,
    OpenRefPicker(CompareField),
    OpenCompareMenu,
    OpenPullRequestPicker,
    OpenThemePicker,
    OpenUiFontPicker,
    OpenMonoFontPicker,
    ShowKeyboardShortcuts,
    CloseOverlay,
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
    OpenSettings,
    OpenKeymaps,
    CloseSettings,
    SetSettingsSection(SettingsSection),
    ToggleWrap,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandsEffect {
    SaveSettings(Box<Settings>),
    LoadFontCatalog,
}

#[derive(Debug, Clone)]
pub enum CommandsEvent {
    SettingsSaved,
    SettingsSaveFailed {
        message: String,
    },
    FontCatalogReady {
        ui: Arc<[FontFamilyEntry]>,
        mono: Arc<[FontFamilyEntry]>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
