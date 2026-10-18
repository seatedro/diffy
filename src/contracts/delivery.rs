#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryAction {
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryEffect {
    Noop,
}

#[derive(Debug, Clone)]
pub enum DeliveryEvent {
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarAvailability {}
