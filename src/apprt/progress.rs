//! Progress reporting channel used by the compare pipeline to surface
//! phase transitions back to the UI.
//!
//! A `ProgressReporter` wraps the runtime's event sender and stamps each
//! emission with the caller's generation id. The state layer applies
//! incoming `CompareProgressUpdate` events only when the generation still
//! matches `compare_progress`, so stale workers silently drop updates.
//!
//! The reporter implements `core::compare::ProgressSink` so backends can
//! take the trait object without depending on apprt directly.

use crate::apprt::runtime::RuntimeEventSender;
use crate::core::compare::{ComparePhase, ProgressSink};
use crate::events::CompareEvent;

#[derive(Clone)]
pub struct ProgressReporter {
    generation: u64,
    sender: RuntimeEventSender,
}

impl ProgressReporter {
    pub(crate) fn new(generation: u64, sender: RuntimeEventSender) -> Self {
        Self { generation, sender }
    }
}

impl ProgressSink for ProgressReporter {
    fn phase(&self, phase: ComparePhase) {
        self.sender.send(CompareEvent::CompareProgressUpdate {
            generation: self.generation,
            phase,
        });
    }
}
