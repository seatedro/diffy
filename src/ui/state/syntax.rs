use std::collections::HashSet;

use crate::actions::SyntaxAction;
use crate::effects::{Effect, LoadFileSyntaxRequest};
use crate::events::SyntaxEvent;

use super::{AppState, MAX_PENDING_SYNTAX_WINDOWS};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SyntaxInflightKey {
    generation: u64,
    request_id: u64,
}

#[derive(Debug, Default)]
pub(super) struct SyntaxRequestTracker {
    next_request_id: u64,
    epoch: u64,
    inflight: HashSet<SyntaxInflightKey>,
}

impl SyntaxRequestTracker {
    pub(super) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(super) fn next_request_id(&mut self) -> u64 {
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.next_request_id
    }

    pub(super) fn last_request_id(&self) -> u64 {
        self.next_request_id
    }

    pub(super) fn set_last_request_id(&mut self, request_id: u64) {
        self.next_request_id = request_id;
    }

    pub(super) fn outstanding_count(&self, pending_count: usize) -> usize {
        self.inflight.len().max(pending_count)
    }

    pub(super) fn budget_available(&self, pending_count: usize) -> bool {
        self.outstanding_count(pending_count) < MAX_PENDING_SYNTAX_WINDOWS
    }

    pub(super) fn track(&mut self, request: &LoadFileSyntaxRequest) {
        self.inflight.insert(SyntaxInflightKey {
            generation: request.cache_generation,
            request_id: request.request_id,
        });
    }

    pub(super) fn finish(&mut self, generation: u64, request_id: u64) {
        self.inflight.remove(&SyntaxInflightKey {
            generation,
            request_id,
        });
    }

    pub(super) fn invalidate(&mut self) {
        self.inflight.clear();
        self.epoch = self.epoch.saturating_add(1);
    }

    #[cfg(test)]
    pub(super) fn inflight_len(&self) -> usize {
        self.inflight.len()
    }

    #[cfg(test)]
    pub(super) fn insert_inflight(&mut self, generation: u64, request_id: u64) {
        self.inflight.insert(SyntaxInflightKey {
            generation,
            request_id,
        });
    }
}

pub(super) fn reduce_action(_state: &mut AppState, action: SyntaxAction) -> Vec<Effect> {
    match action {}
}

pub(super) fn reduce_event(state: &mut AppState, event: SyntaxEvent) -> Vec<Effect> {
    match event {
        SyntaxEvent::FileSyntaxReady(payload) => state.handle_file_syntax_ready(payload),
        SyntaxEvent::SyntaxPackInstallStarted { language } => {
            state.handle_syntax_pack_install_started(&language);
            Vec::new()
        }
        SyntaxEvent::SyntaxPacksInstalled { languages } => {
            state.handle_syntax_packs_installed(&languages)
        }
        SyntaxEvent::SyntaxPackInstallFinished { language }
        | SyntaxEvent::SyntaxPackInstallFailed { language } => {
            state.handle_syntax_pack_install_finished(&language);
            Vec::new()
        }
    }
}
