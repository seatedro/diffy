//! UI-local signals for reactive state that components read directly.
//!
//! These signals hold ephemeral UI state (animation factors, hover indices,
//! scroll positions) that would otherwise clutter AppState. Components read
//! them via `cx.read()` during rendering.

use crate::ui::signals::{Signal, SignalStore};

/// All UI signal handles. Created once at app startup, persists across frames.
#[derive(Debug, Clone, Copy)]
pub struct UiSignals {
    /// Sidebar width factor: 1.0 = fully expanded, 0.0 = collapsed.
    /// Animated smoothly on toggle.
    pub sidebar_width_factor: Signal<f32>,

    /// Currently hovered file index in the sidebar.
    pub hovered_file_index: Signal<Option<usize>>,

    /// Currently hovered toast index.
    pub hovered_toast_index: Signal<Option<usize>>,

    /// File list scroll position in pixels.
    pub file_list_scroll_px: Signal<f32>,

    /// Viewport scroll position in pixels.
    pub viewport_scroll_px: Signal<f32>,
}

impl UiSignals {
    /// Create all UI signals in the given store.
    pub fn new(store: &mut SignalStore) -> Self {
        Self {
            sidebar_width_factor: store.create(1.0_f32),
            hovered_file_index: store.create(None::<usize>),
            hovered_toast_index: store.create(None::<usize>),
            file_list_scroll_px: store.create(0.0_f32),
            viewport_scroll_px: store.create(0.0_f32),
        }
    }

    /// Sync scroll positions from AppState into signals (called each frame).
    pub fn sync_from_state(
        &self,
        store: &mut SignalStore,
        file_list_scroll: f32,
        viewport_scroll: f32,
        sidebar_visible: bool,
    ) {
        store.write(self.file_list_scroll_px, file_list_scroll);
        store.write(self.viewport_scroll_px, viewport_scroll);

        let target = if sidebar_visible { 1.0 } else { 0.0 };
        store.write(self.sidebar_width_factor, target);
    }
}
