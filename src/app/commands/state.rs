use crate::contracts::commands::CommandsAction;
use crate::contracts::commands::CommandsEvent;
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Reduction};

#[derive(Debug, Default)]
pub struct CommandsController {}

impl Controller for CommandsController {
    type Action = CommandsAction;
    type Event = CommandsEvent;

    fn reduce_action(&mut self, _action: Self::Action, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }

    fn reduce_event(&mut self, _event: Self::Event, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }
}
