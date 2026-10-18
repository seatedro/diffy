use diffy_ui::diff_view::decorator::DiffDecorator;

use crate::app::text_targets::TextHost;
use crate::contracts::review::ReviewAction;
use crate::contracts::review::ReviewEvent;
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Reduction};

#[derive(Debug, Default)]
pub struct ReviewPresentation {
    revision: u64,
}

impl DiffDecorator for ReviewPresentation {
    fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Debug, Default)]
pub struct ReviewController {
    presentation: ReviewPresentation,
}

impl ReviewController {
    pub fn presentation(&self) -> &ReviewPresentation {
        &self.presentation
    }
}

impl Controller for ReviewController {
    type Action = ReviewAction;
    type Event = ReviewEvent;

    fn reduce_action(&mut self, _action: Self::Action, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }

    fn reduce_event(&mut self, _event: Self::Event, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }
}

impl TextHost for ReviewController {}
