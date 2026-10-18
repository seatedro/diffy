use std::sync::Arc;

use diffy_syntax::{GrammarStore, HighlightWorker};
use diffy_ui::DiffSessionViewState;

pub fn grammar_store() -> GrammarStore {
    GrammarStore::none()
}

pub type SyntaxWake = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub struct ViewportSyntax {
    worker: Option<(HighlightWorker, GrammarStore)>,
    wake: Option<SyntaxWake>,
}

impl ViewportSyntax {
    pub fn set_wake(&mut self, wake: SyntaxWake) {
        self.wake = Some(wake);
    }

    pub fn has_wake(&self) -> bool {
        self.wake.is_some()
    }

    pub fn attach(&mut self, diff: &mut DiffSessionViewState) {
        if let Some(wake) = &self.wake {
            let wake = wake.clone();
            diff.set_syntax_wake(move || wake());
        }
        let (worker, store) = self.worker.get_or_insert_with(|| {
            let store = grammar_store();
            (HighlightWorker::new(store.clone()), store)
        });
        diff.enable_syntax_shared(worker, store.clone());
    }
}
