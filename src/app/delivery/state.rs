use crate::contracts::delivery::DeliveryAction;
use crate::contracts::delivery::DeliveryEvent;
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Reduction};

#[derive(Debug, Default)]
pub struct DeliveryController {}

impl Controller for DeliveryController {
    type Action = DeliveryAction;
    type Event = DeliveryEvent;

    fn reduce_action(&mut self, _action: Self::Action, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }

    fn reduce_event(&mut self, _event: Self::Event, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }
}
