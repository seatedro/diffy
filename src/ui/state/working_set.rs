use std::collections::HashSet;

use super::ViewportSlotKey;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct WorkingSetFileKey {
    index: usize,
    path: String,
    left_ref: String,
    right_ref: String,
}

impl WorkingSetFileKey {
    pub(super) fn new(index: usize, path: String, left_ref: String, right_ref: String) -> Self {
        Self {
            index,
            path,
            left_ref,
            right_ref,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct FileWorkingSet {
    tick: u64,
    protected: HashSet<WorkingSetFileKey>,
}

impl FileWorkingSet {
    pub(super) fn reset(&mut self) {
        self.tick = 0;
        self.protected.clear();
    }

    pub(super) fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.saturating_add(1);
        self.tick
    }

    pub(super) fn protect_slots(&mut self, slots: &[ViewportSlotKey]) {
        self.protected.clear();
        self.protected
            .extend(slots.iter().filter_map(ViewportSlotKey::working_set_key));
    }

    pub(super) fn protected_snapshot(&self) -> HashSet<WorkingSetFileKey> {
        self.protected.clone()
    }
}
