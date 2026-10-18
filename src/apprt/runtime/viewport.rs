pub mod interaction;
pub mod pipeline;

use crate::apprt::services::ViewportServices;
use crate::apprt::transport::ScopedEventSender;
use crate::contracts::viewport::ViewportEffect;

pub use interaction::InteractionRuntime;
pub use pipeline::PipelineRuntime;

pub struct ViewportRuntime {
    pub pipeline: PipelineRuntime,
    pub interaction: InteractionRuntime,
}

impl ViewportRuntime {
    pub fn new(services: ViewportServices) -> Self {
        Self {
            pipeline: PipelineRuntime::new(services.pipeline),
            interaction: InteractionRuntime::new(services.interaction),
        }
    }

    pub fn dispatch(&self, effect: ViewportEffect, events: ScopedEventSender) {
        match effect {
            ViewportEffect::Pipeline(effect) => self.pipeline.dispatch(effect, events),
            ViewportEffect::Interaction(effect) => self.interaction.dispatch(effect, events),
        }
    }
}
