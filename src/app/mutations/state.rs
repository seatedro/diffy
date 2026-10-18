use crate::contracts::mutations::MutationsAction;
use crate::contracts::mutations::MutationsEvent;
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Reduction};

#[derive(Debug, Default)]
pub struct MutationsController {}

impl Controller for MutationsController {
    type Action = MutationsAction;
    type Event = MutationsEvent;

    fn reduce_action(&mut self, _action: Self::Action, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }

    fn reduce_event(&mut self, _event: Self::Event, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }
}
