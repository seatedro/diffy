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

impl ViewportController {
    fn prepare(&mut self) {
        if let Some(diff) = self.interaction.diff() {
            self.pipeline.set_preparation(diff.preparation());
        }
    }

    fn install(&mut self, mut reduction: Reduction, cx: &SharedContext) -> Reduction {
        for install in self.pipeline.take_installs() {
            reduction.merge(self.interaction.install(install, cx));
        }
        reduction
    }
}

impl Controller for ViewportController {
    type Action = ViewportAction;
    type Event = ViewportEvent;

    fn reduce_action(&mut self, action: ViewportAction, cx: &SharedContext) -> Reduction {
        match action {
            ViewportAction::Pipeline(action) => {
                self.prepare();
                let reduction = self.pipeline.reduce_action(action, cx);
                self.install(reduction, cx)
            }
            ViewportAction::Interaction(action) => self.interaction.reduce_action(action, cx),
        }
    }

    fn reduce_event(&mut self, event: ViewportEvent, cx: &SharedContext) -> Reduction {
        match event {
            ViewportEvent::Pipeline(event) => {
                self.prepare();
                let reduction = self.pipeline.reduce_event(event, cx);
                self.install(reduction, cx)
            }
            ViewportEvent::Interaction(event) => self.interaction.reduce_event(event, cx),
        }
    }

    fn apply_intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        self.prepare();
        let reduction = self.pipeline.apply_intent(intent, cx);
        let mut reduction = self.install(reduction, cx);
        reduction.merge(self.interaction.apply_intent(intent, cx));
        reduction
    }
}
