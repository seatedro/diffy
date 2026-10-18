#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewAction {
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewEffect {
    Noop,
}

#[derive(Debug, Clone)]
pub enum ReviewEvent {
    Noop,
}

#[derive(Debug, Clone, Default)]
pub struct ReviewContext {}
