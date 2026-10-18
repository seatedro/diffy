#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionAction {
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionEffect {
    Noop,
}

#[derive(Debug, Clone)]
pub enum InteractionEvent {
    Noop,
}
