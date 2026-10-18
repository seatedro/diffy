use crate::contracts::navigation::NavigationAction;
use crate::contracts::navigation::NavigationEvent;
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Reduction};

#[derive(Debug, Default)]
pub struct NavigationController {}

impl Controller for NavigationController {
    type Action = NavigationAction;
    type Event = NavigationEvent;

    fn reduce_action(&mut self, _action: Self::Action, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }

    fn reduce_event(&mut self, _event: Self::Event, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }
}
