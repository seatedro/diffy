use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Reduction};
use crate::contracts::viewport::InteractionAction;
use crate::contracts::viewport::InteractionEvent;

#[derive(Debug, Default)]
pub struct InteractionController {}

impl Controller for InteractionController {
    type Action = InteractionAction;
    type Event = InteractionEvent;

    fn reduce_action(&mut self, _action: Self::Action, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }

    fn reduce_event(&mut self, _event: Self::Event, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }
}
