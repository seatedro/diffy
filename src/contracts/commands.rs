use crate::contracts::navigation::CompareField;
use crate::contracts::shared::JobId;
use crate::contracts::shell::{Action, LogicalFocus};
use crate::core::keymap::ShortcutCommand;
use crate::core::theme_types::ThemeMode;
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
pub enum CommandsAction {
    OpenCommandPalette,
    OpenRepoPicker,
    OpenRefPicker(CompareField),
    OpenCompareMenu,
    OpenThemePicker,
    OpenUiFontPicker,
    OpenMonoFontPicker,
    ShowKeyboardShortcuts,
    CloseOverlay,
    MoveOverlaySelection(i32),
    SelectOverlayEntry(usize),
    ConfirmOverlaySelection,
    TabCompletePickerDir,
    OpenContextMenu {
        entries: Vec<ContextMenuEntry>,
        x: i32,
        y: i32,
    },
    CloseContextMenu,
    Confirm,
    CancelConfirmation,
    DismissToast(JobId),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandsEffect {
    SaveSettings(Box<Settings>),
}

#[derive(Debug, Clone)]
pub enum CommandsEvent {
    SettingsSaved,
    SettingsSaveFailed { message: String },
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
        Self::Show {
            id: None,
            kind: ToastKind::Error,
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
