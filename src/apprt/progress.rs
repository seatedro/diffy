//! Progress reporting channel used by the compare pipeline to surface
//! phase transitions back to the UI.
//!
//! A `ProgressReporter` wraps the runtime's event sender and stamps each
//! emission with the caller's generation id. The state layer applies
//! incoming `CompareProgressUpdate` events only when the generation still
//! matches `compare_progress`, so stale workers silently drop updates.

use crate::apprt::runtime::RuntimeEventSender;
use crate::events::AppEvent;
use crate::ui::state::ComparePhase;

#[derive(Clone)]
pub struct ProgressReporter {
    generation: u64,
    sender: RuntimeEventSender,
}

impl ProgressReporter {
    pub(crate) fn new(generation: u64, sender: RuntimeEventSender) -> Self {
        Self { generation, sender }
    }

    pub fn phase(&self, phase: ComparePhase) {
        self.sender.send(AppEvent::CompareProgressUpdate {
            generation: self.generation,
            phase,
        });
    }
}
