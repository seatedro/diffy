pub mod files;
pub mod interaction;
pub mod pipeline;
pub mod symbols;

use crate::apprt::services::ViewportServices;
use crate::apprt::transport::ScopedEventSender;
use crate::contracts::viewport::ViewportEffect;

pub use files::FilesRuntime;
pub use interaction::InteractionRuntime;
pub use pipeline::PipelineRuntime;
pub use symbols::SymbolsRuntime;

pub struct ViewportRuntime {
    pub pipeline: PipelineRuntime,
    pub interaction: InteractionRuntime,
    pub files: FilesRuntime,
    pub symbols: SymbolsRuntime,
}

impl ViewportRuntime {
    pub fn new(services: ViewportServices) -> Self {
        Self {
            pipeline: PipelineRuntime::new(services.pipeline),
            interaction: InteractionRuntime::new(services.interaction),
            files: FilesRuntime::new(services.files),
            symbols: SymbolsRuntime::default(),
        }
    }

    pub fn dispatch(&self, effect: ViewportEffect, events: ScopedEventSender) {
        match effect {
            ViewportEffect::Pipeline(effect) => self.pipeline.dispatch(effect, events),
            ViewportEffect::Interaction(effect) => self.interaction.dispatch(effect, events),
            ViewportEffect::Files(effect) => self.files.dispatch(effect, events),
            ViewportEffect::Symbols(effect) => self.symbols.dispatch(effect, events),
        }
    }
}
