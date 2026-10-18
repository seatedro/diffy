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
