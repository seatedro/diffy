pub mod interaction;
pub mod pipeline;

use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Intent, Reduction};
use crate::contracts::viewport::interaction::InstallItem;
use crate::contracts::viewport::pipeline::PipelineAction;
use crate::contracts::viewport::{ViewportAction, ViewportEvent};

pub use interaction::InteractionController;
pub use pipeline::PipelineController;
use pipeline::PipelineInstall;

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
        let batch = self.pipeline.take_batch();
        if !batch.is_empty() {
            let items = batch.into_iter().map(|install| match install {
                PipelineInstall::View(install) => InstallItem::View(install),
                PipelineInstall::Slot(update) => InstallItem::Slot(update),
            });
            reduction.merge(self.interaction.install_batch(items, cx));
        }
        reduction
    }

    fn request_sources(&mut self, mut reduction: Reduction, cx: &SharedContext) -> Reduction {
        let requests = self.interaction.take_source_requests();
        if requests.is_empty() {
            return reduction;
        }
        self.prepare();
        for (file, purpose) in requests {
            reduction.merge(
                self.pipeline
                    .reduce_action(PipelineAction::RequestSources { file, purpose }, cx),
            );
        }
        self.install(reduction, cx)
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
            ViewportAction::Interaction(action) => {
                let reduction = self.interaction.reduce_action(action, cx);
                self.request_sources(reduction, cx)
            }
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
