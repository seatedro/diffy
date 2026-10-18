use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Reduction};
use crate::contracts::viewport::PipelineAction;
use crate::contracts::viewport::PipelineEvent;

#[derive(Debug, Default)]
pub struct PipelineController {}

impl Controller for PipelineController {
    type Action = PipelineAction;
    type Event = PipelineEvent;

    fn reduce_action(&mut self, _action: Self::Action, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }

    fn reduce_event(&mut self, _event: Self::Event, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }
}
