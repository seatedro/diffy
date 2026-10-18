pub mod interaction;
pub mod pipeline;

pub use interaction::{InteractionAction, InteractionEffect, InteractionEvent};
pub use pipeline::{
    CompareIntent, CompareManifestSnapshot, PipelineAction, PipelineEffect, PipelineEvent,
    PreparedDiff,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewportAction {
    Pipeline(PipelineAction),
    Interaction(InteractionAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewportEffect {
    Pipeline(PipelineEffect),
    Interaction(InteractionEffect),
}

#[derive(Debug, Clone)]
pub enum ViewportEvent {
    Pipeline(PipelineEvent),
    Interaction(InteractionEvent),
}
