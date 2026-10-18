#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandsAction {
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandsEffect {
    Noop,
}

#[derive(Debug, Clone)]
pub enum CommandsEvent {
    Noop,
}

#[derive(Debug, Clone, Default)]
pub struct SettingsSnapshot {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationRequest {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastUpdate {}

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
