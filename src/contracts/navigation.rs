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
