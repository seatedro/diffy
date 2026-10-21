pub mod files;
pub mod interaction;
pub mod pipeline;
pub mod symbols;

pub use files::{FilesAction, FilesEffect, FilesEvent};
pub use interaction::{InteractionAction, InteractionEffect, InteractionEvent};
pub use pipeline::{
    CompareIntent, CompareManifestSnapshot, PipelineAction, PipelineEffect, PipelineEvent,
    PreparedDiff,
};
pub use symbols::{SymbolsAction, SymbolsEffect, SymbolsEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewportAction {
    Pipeline(PipelineAction),
    Interaction(InteractionAction),
    Files(FilesAction),
    Symbols(SymbolsAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewportEffect {
    Pipeline(PipelineEffect),
    Interaction(InteractionEffect),
    Files(FilesEffect),
    Symbols(SymbolsEffect),
}

#[derive(Debug, Clone)]
pub enum ViewportEvent {
    Pipeline(PipelineEvent),
    Interaction(InteractionEvent),
    Files(FilesEvent),
    Symbols(SymbolsEvent),
}
