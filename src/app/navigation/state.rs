use quark_ui::FocusId;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome, TextField};

use crate::app::text_targets::{SIDEBAR_FILTER, TextCx, TextHost};
use crate::contracts::navigation::NavigationAction;
use crate::contracts::navigation::NavigationEvent;
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Reduction};

#[derive(Debug, Default)]
pub struct NavigationController {
    filter: TextField,
}

impl NavigationController {
    pub fn filter(&self) -> &TextField {
        &self.filter
    }
}

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

impl TextHost for NavigationController {
    fn owns_text(&self, target: FocusId) -> bool {
        target == SIDEBAR_FILTER
    }

    fn edit_text(
        &mut self,
        _target: FocusId,
        command: TextEditCommand,
        cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        Some(self.filter.apply_at(command, cx.now_ms))
    }

    fn set_preedit(
        &mut self,
        _target: FocusId,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> bool {
        self.filter.set_preedit(text, cursor);
        true
    }

    fn set_text_value(
        &mut self,
        _target: FocusId,
        value: String,
        _cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        let text_changed = self.filter.text() != value;
        self.filter.set_text(value);
        Some(TextEditOutcome {
            text_changed,
            selection_changed: true,
            ..TextEditOutcome::default()
        })
    }
}
