#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationsAction {
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationsEffect {
    Noop,
}

#[derive(Debug, Clone)]
pub enum MutationsEvent {
    Noop,
}

#[derive(Debug, Clone)]
pub struct MutationSource {}
