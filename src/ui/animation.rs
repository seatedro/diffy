use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Animation keys
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum AnimationKey {
    FileListHover(usize),
    ViewportRowHover(usize),
    ToastEntrance(u64),
    ToastExit(u64),
}

// ---------------------------------------------------------------------------
// Transition
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Transition {
    target: f32,
    current: f32,
    started_at_ms: u64,
    duration_ms: u64,
    start_value: f32,
}

// ---------------------------------------------------------------------------
// Easing
// ---------------------------------------------------------------------------

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

// ---------------------------------------------------------------------------
// Animation state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AnimationState {
    transitions: HashMap<AnimationKey, Transition>,
    has_active: bool,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self {
            transitions: HashMap::new(),
            has_active: false,
        }
    }
}

impl AnimationState {
    pub fn set_target(&mut self, key: AnimationKey, target: f32, duration_ms: u64, clock_ms: u64) {
        let entry = self.transitions.entry(key).or_insert(Transition {
            target,
            current: if target > 0.5 { 0.0 } else { 1.0 },
            started_at_ms: clock_ms,
            duration_ms,
            start_value: if target > 0.5 { 0.0 } else { 1.0 },
        });
        if (entry.target - target).abs() > f32::EPSILON {
            entry.start_value = entry.current;
            entry.target = target;
            entry.started_at_ms = clock_ms;
            entry.duration_ms = duration_ms;
        }
    }

    pub fn progress(&self, key: AnimationKey) -> Option<f32> {
        self.transitions.get(&key).map(|t| t.current)
    }

    pub fn has_active(&self) -> bool {
        self.has_active
    }

    pub fn tick(&mut self, clock_ms: u64) {
        let mut any_active = false;
        self.transitions.retain(|_, t| {
            let elapsed = clock_ms.saturating_sub(t.started_at_ms) as f32;
            let duration = t.duration_ms.max(1) as f32;
            let raw_t = (elapsed / duration).clamp(0.0, 1.0);
            let eased = ease_out(raw_t);
            t.current = t.start_value + (t.target - t.start_value) * eased;

            let done = raw_t >= 1.0;
            if !done {
                any_active = true;
            }
            // Keep completed transitions briefly for reading, remove faded-out ones
            if done && t.target < 0.01 {
                return false;
            }
            true
        });
        self.has_active = any_active;
    }
}
