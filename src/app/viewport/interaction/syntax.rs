use std::sync::Arc;

use diffy_syntax::{HighlightWorker, ScopeWorker};
use diffy_ui::DiffSessionViewState;

use crate::core::syntax::grammar::Grammars;

pub type SyntaxWake = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub struct ViewportSyntax {
    grammars: Option<Grammars>,
    worker: Option<HighlightWorker>,
    wake: Option<SyntaxWake>,
}

impl ViewportSyntax {
    pub fn set_grammars(&mut self, grammars: Grammars) {
        self.grammars = Some(grammars);
        self.worker = None;
    }

    pub fn set_wake(&mut self, wake: SyntaxWake) {
        self.wake = Some(wake);
    }

    pub fn has_wake(&self) -> bool {
        self.wake.is_some()
    }

    pub fn scope_worker(&mut self) -> ScopeWorker {
        let grammars = self
            .grammars
            .get_or_insert_with(|| Grammars::shared().clone());
        let worker = grammars.scope_worker();
        if let Some(wake) = &self.wake {
            let wake = wake.clone();
            worker.set_wake(move || wake());
        }
        worker
    }

    pub fn attach(&mut self, diff: &mut DiffSessionViewState) {
        if let Some(wake) = &self.wake {
            let wake = wake.clone();
            diff.set_syntax_wake(move || wake());
        }
        let grammars = self
            .grammars
            .get_or_insert_with(|| Grammars::shared().clone());
        let worker = self.worker.get_or_insert_with(|| grammars.worker());
        diff.enable_syntax_shared(worker, grammars.store());
    }
}
