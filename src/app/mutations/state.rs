use diffy_ui::diff_view::decorator::DiffDecorator;

use crate::app::text_targets::TextHost;
use crate::contracts::mutations::MutationsAction;
use crate::contracts::mutations::MutationsEvent;
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Reduction};

#[derive(Debug, Default)]
pub struct MutationPresentation {
    revision: u64,
}

impl DiffDecorator for MutationPresentation {
    fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Debug, Default)]
pub struct MutationsController {
    presentation: MutationPresentation,
}

impl MutationsController {
    pub fn presentation(&self) -> &MutationPresentation {
        &self.presentation
    }
}

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

impl TextHost for MutationsController {}
