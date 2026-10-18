#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationAction {
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationEffect {
    Noop,
}

#[derive(Debug, Clone)]
pub enum NavigationEvent {
    Noop,
}

#[derive(Debug, Clone, Default)]
pub struct RepositoryContext {}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompareField {
    #[default]
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarTab {
    #[default]
    Files,
    Commits,
}
