//! UI-local signals for reactive state that components read directly.
//!
//! These signals hold ephemeral UI state (animation factors) that would
//! otherwise clutter AppState. Components read them via `cx.read()` during
//! rendering.

use halogen::reactive::{Signal, SignalStore};

/// All UI signal handles. Created once at app startup, persists across frames.
#[derive(Debug, Clone, Copy)]
pub struct UiSignals {
    /// Sidebar width factor: 1.0 = fully expanded, 0.0 = collapsed.
    /// Animated smoothly on toggle.
    pub sidebar_width_factor: Signal<f32>,
}

impl UiSignals {
    /// Create all UI signals in the given store.
    pub fn new(store: &SignalStore) -> Self {
        Self {
            sidebar_width_factor: store.create(1.0_f32),
        }
    }

    /// Sync sidebar target factor from AppState (called each frame).
    pub fn sync_from_state(&self, store: &SignalStore, sidebar_visible: bool) {
        let target = if sidebar_visible { 1.0 } else { 0.0 };
        store.set_if_changed(self.sidebar_width_factor, target);
    }
}
