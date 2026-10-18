pub mod interaction;
pub mod pipeline;

use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Intent, Reduction};
use crate::contracts::viewport::{ViewportAction, ViewportEvent};

pub use interaction::InteractionController;
pub use pipeline::PipelineController;

#[derive(Debug, Default)]
pub struct ViewportController {
    pub pipeline: PipelineController,
    pub interaction: InteractionController,
}

impl Controller for ViewportController {
    type Action = ViewportAction;
    type Event = ViewportEvent;

    fn reduce_action(&mut self, action: ViewportAction, cx: &SharedContext) -> Reduction {
        match action {
            ViewportAction::Pipeline(action) => self.pipeline.reduce_action(action, cx),
            ViewportAction::Interaction(action) => self.interaction.reduce_action(action, cx),
        }
    }

    fn reduce_event(&mut self, event: ViewportEvent, cx: &SharedContext) -> Reduction {
        match event {
            ViewportEvent::Pipeline(event) => self.pipeline.reduce_event(event, cx),
            ViewportEvent::Interaction(event) => self.interaction.reduce_event(event, cx),
        }
    }

    fn apply_intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        let mut reduction = self.pipeline.apply_intent(intent, cx);
        reduction.merge(self.interaction.apply_intent(intent, cx));
        reduction
    }
}
