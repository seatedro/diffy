//! Fine-grained reactive signals.
//!
//! `Signal<T>` is a Copy handle (8 bytes) into a persistent `SignalStore`.
//! Values live in a slot arena and are accessed via `cx.read()` / `cx.write()`.

use std::any::Any;
use std::cell::RefCell;
use std::marker::PhantomData;

// ---------------------------------------------------------------------------
// SignalId — stable arena index + generation for use-after-free detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SignalId {
    index: u32,
    generation: u32,
}

// ---------------------------------------------------------------------------
// Signal<T> — Copy handle into the store
// ---------------------------------------------------------------------------

/// A reactive signal handle. Copy, 8 bytes. The actual value lives in
/// the `SignalStore` and is accessed via `SignalStore::read` / `write`.
pub struct Signal<T> {
    pub(crate) id: SignalId,
    _marker: PhantomData<T>,
}

// Manual impls to avoid requiring T: Copy/Clone.
impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Signal<T> {}

impl<T> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T> Eq for Signal<T> {}

impl<T> std::fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signal")
            .field("index", &self.id.index)
            .field("generation", &self.id.generation)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Dependency tracking — thread-local observer
// ---------------------------------------------------------------------------

struct TrackingScope {
    dependencies: Vec<SignalId>,
}

thread_local! {
    static OBSERVER: RefCell<Option<TrackingScope>> = RefCell::new(None);
}

fn track_read(id: SignalId) {
    OBSERVER.with(|obs| {
        if let Some(scope) = obs.borrow_mut().as_mut() {
            if !scope.dependencies.contains(&id) {
                scope.dependencies.push(id);
            }
        }
    });
}

/// Run `f` with dependency tracking enabled. Returns the result plus the list
/// of signal IDs that were read during `f`.
pub fn with_tracking<R>(f: impl FnOnce() -> R) -> (R, Vec<SignalId>) {
    let prev = OBSERVER.with(|obs| obs.borrow_mut().take());

    OBSERVER.with(|obs| {
        *obs.borrow_mut() = Some(TrackingScope {
            dependencies: Vec::new(),
        });
    });

    let result = f();

    let scope = OBSERVER.with(|obs| obs.borrow_mut().take())
        .expect("tracking scope disappeared during with_tracking");

    OBSERVER.with(|obs| {
        *obs.borrow_mut() = prev;
    });

    (result, scope.dependencies)
}

// ---------------------------------------------------------------------------
// Slot — one entry in the arena
// ---------------------------------------------------------------------------

struct Slot {
    /// The stored value, type-erased. `None` if the slot is free.
    value: Option<Box<dyn Any>>,
    /// Generation counter — incremented each time the slot is reused.
    generation: u32,
}

// ---------------------------------------------------------------------------
// SignalStore — the slot arena
// ---------------------------------------------------------------------------

type MemoFn = Box<dyn Fn(&SignalStore) -> Box<dyn Any>>;

/// Persistent store for signal values. Lives in the app, survives across frames.
pub struct SignalStore {
    slots: Vec<Slot>,
    free_list: Vec<u32>,
    /// For each slot: indices of slots that depend on it (downstream).
    subscribers: Vec<Vec<u32>>,
    dirty: Vec<bool>,
    /// For memo slots: the compute function. `None` for regular signals.
    memo_fns: Vec<Option<MemoFn>>,
}

impl SignalStore {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_list: Vec::new(),
            subscribers: Vec::new(),
            dirty: Vec::new(),
            memo_fns: Vec::new(),
        }
    }

    /// Create a new signal with the given initial value.
    pub fn create<T: 'static>(&mut self, value: T) -> Signal<T> {
        let boxed: Box<dyn Any> = Box::new(value);

        let (index, generation) = if let Some(index) = self.free_list.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(boxed);
            self.subscribers[index as usize].clear();
            self.dirty[index as usize] = false;
            self.memo_fns[index as usize] = None;
            (index, slot.generation)
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Slot {
                value: Some(boxed),
                generation: 0,
            });
            self.subscribers.push(Vec::new());
            self.dirty.push(false);
            self.memo_fns.push(None);
            (index, 0)
        };

        Signal {
            id: SignalId { index, generation },
            _marker: PhantomData,
        }
    }

    /// Create a derived signal (memo) whose value is computed from other signals.
    /// The compute function runs immediately to get the initial value, then
    /// re-runs automatically when dependencies change (after `update_memos()`).
    pub fn create_memo<T: 'static + Clone + PartialEq>(
        &mut self,
        compute: impl Fn(&SignalStore) -> T + 'static,
    ) -> Signal<T> {
        // Run the compute function to get the initial value and discover deps.
        let (initial, deps) = {
            let store_ref: &SignalStore = self;
            with_tracking(|| compute(store_ref))
        };

        let signal = self.create(initial);
        let idx = signal.id.index;

        // Register this memo as a subscriber of each dependency.
        for dep in &deps {
            let subs = &mut self.subscribers[dep.index as usize];
            if !subs.contains(&idx) {
                subs.push(idx);
            }
        }

        // Store the compute function (type-erased).
        self.memo_fns[idx as usize] = Some(Box::new(move |store: &SignalStore| {
            Box::new(compute(store)) as Box<dyn Any>
        }));

        signal
    }

    /// Read a signal's value by cloning it out. Registers the read with the
    /// current tracking scope, if one exists.
    ///
    /// Panics if the signal handle is stale (freed and reallocated).
    pub fn read<T: 'static + Clone>(&self, signal: Signal<T>) -> T {
        self.with(signal, Clone::clone)
    }

    /// Read a signal's value without registering a dependency.
    pub fn read_untracked<T: 'static + Clone>(&self, signal: Signal<T>) -> T {
        self.with_untracked(signal, Clone::clone)
    }

    /// Access a signal's value by reference via a closure. Registers the read
    /// with the current tracking scope, if one exists.
    ///
    /// This avoids cloning when you only need to inspect the value.
    pub fn with<T: 'static, R>(&self, signal: Signal<T>, f: impl FnOnce(&T) -> R) -> R {
        track_read(signal.id);
        self.with_untracked(signal, f)
    }

    fn with_untracked<T: 'static, R>(&self, signal: Signal<T>, f: impl FnOnce(&T) -> R) -> R {
        let slot = &self.slots[signal.id.index as usize];
        assert_eq!(
            slot.generation, signal.id.generation,
            "stale signal handle (generation mismatch)"
        );
        let value = slot
            .value
            .as_ref()
            .expect("signal slot is empty")
            .downcast_ref::<T>()
            .expect("signal type mismatch");
        f(value)
    }

    /// Replace a signal's value and propagate dirtiness to subscribers.
    pub fn write<T: 'static>(&mut self, signal: Signal<T>, value: T) {
        let idx = signal.id.index as usize;
        let slot = &mut self.slots[idx];
        assert_eq!(
            slot.generation, signal.id.generation,
            "stale signal handle (generation mismatch)"
        );
        slot.value = Some(Box::new(value));
        self.mark_dirty_and_propagate(idx);
    }

    /// Mutate a signal's value in place and propagate dirtiness to subscribers.
    pub fn update<T: 'static>(&mut self, signal: Signal<T>, f: impl FnOnce(&mut T)) {
        let idx = signal.id.index as usize;
        let slot = &mut self.slots[idx];
        assert_eq!(
            slot.generation, signal.id.generation,
            "stale signal handle (generation mismatch)"
        );
        let value = slot
            .value
            .as_mut()
            .expect("signal slot is empty")
            .downcast_mut::<T>()
            .expect("signal type mismatch");
        f(value);
        self.mark_dirty_and_propagate(idx);
    }

    fn mark_dirty_and_propagate(&mut self, idx: usize) {
        self.dirty[idx] = true;
        let subs = self.subscribers[idx].clone();
        for sub_idx in subs {
            if !self.dirty[sub_idx as usize] {
                self.dirty[sub_idx as usize] = true;
                // Recursive propagation for chained memos.
                let transitive = self.subscribers[sub_idx as usize].clone();
                for t in transitive {
                    if !self.dirty[t as usize] {
                        self.dirty[t as usize] = true;
                    }
                }
            }
        }
    }

    /// Recompute all dirty memos. Call once per frame before rendering.
    /// Memos whose computed value hasn't changed won't propagate further dirtiness.
    pub fn update_memos(&mut self) {
        let dirty_memos: Vec<u32> = (0..self.slots.len())
            .filter(|&i| self.dirty[i] && self.memo_fns[i].is_some())
            .map(|i| i as u32)
            .collect();

        for index in dirty_memos {
            let compute = self.memo_fns[index as usize].take().unwrap();

            // Recompute with dependency tracking.
            let (new_value, new_deps) = {
                let store_ref: &SignalStore = self;
                with_tracking(|| compute(store_ref))
            };

            // Clear old subscriber registrations for this memo.
            for subs in &mut self.subscribers {
                subs.retain(|&s| s != index);
            }

            // Register new dependencies.
            for dep in &new_deps {
                let subs = &mut self.subscribers[dep.index as usize];
                if !subs.contains(&index) {
                    subs.push(index);
                }
            }

            // Update the cached value.
            self.slots[index as usize].value = Some(new_value);
            self.dirty[index as usize] = false;

            // Put the compute function back.
            self.memo_fns[index as usize] = Some(compute);
        }
    }

    /// Dispose a signal, freeing its slot for reuse.
    pub fn dispose<T>(&mut self, signal: Signal<T>) {
        let idx = signal.id.index as usize;
        let slot = &mut self.slots[idx];
        if slot.generation == signal.id.generation {
            slot.value = None;
            slot.generation = slot.generation.wrapping_add(1);
            self.memo_fns[idx] = None;
            // Remove from all subscriber lists.
            for subs in &mut self.subscribers {
                subs.retain(|&s| s != signal.id.index);
            }
            self.subscribers[idx].clear();
            self.free_list.push(signal.id.index);
        }
    }

    pub fn mark_dirty(&mut self, signal_id: SignalId) {
        self.mark_dirty_and_propagate(signal_id.index as usize);
    }

    pub fn is_dirty(&self, signal_id: SignalId) -> bool {
        self.dirty[signal_id.index as usize]
    }

    /// Returns true if any signal has been modified since the last `clear_dirty()`.
    pub fn any_dirty(&self) -> bool {
        self.dirty.iter().any(|&d| d)
    }

    pub fn clear_dirty(&mut self) {
        self.dirty.iter_mut().for_each(|d| *d = false);
    }

    /// Returns true if the given signal is a memo (has a compute function).
    pub fn is_memo(&self, signal_id: SignalId) -> bool {
        self.memo_fns
            .get(signal_id.index as usize)
            .map_or(false, |f| f.is_some())
    }

    /// Number of live signals.
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.value.is_some()).count()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_read_signal() {
        let mut store = SignalStore::new();
        let sig = store.create(42i32);
        assert_eq!(store.read(sig), 42);
    }

    #[test]
    fn write_signal() {
        let mut store = SignalStore::new();
        let sig = store.create(0i32);
        store.write(sig, 99);
        assert_eq!(store.read(sig), 99);
    }

    #[test]
    fn update_signal_in_place() {
        let mut store = SignalStore::new();
        let sig = store.create(vec![1, 2, 3]);
        store.update(sig, |v| v.push(4));
        assert_eq!(store.read(sig), vec![1, 2, 3, 4]);
    }

    #[test]
    fn with_avoids_clone() {
        let mut store = SignalStore::new();
        let sig = store.create(String::from("hello"));
        let len = store.with(sig, |s| s.len());
        assert_eq!(len, 5);
    }

    #[test]
    fn signal_is_copy() {
        let mut store = SignalStore::new();
        let sig = store.create(10u32);
        let sig2 = sig; // Copy
        let sig3 = sig; // Still valid
        assert_eq!(store.read(sig2), 10);
        assert_eq!(store.read(sig3), 10);
    }

    #[test]
    fn multiple_signals_independent() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);
        let b = store.create(2i32);
        let c = store.create(3i32);
        store.write(b, 20);
        assert_eq!(store.read(a), 1);
        assert_eq!(store.read(b), 20);
        assert_eq!(store.read(c), 3);
    }

    #[test]
    fn dispose_and_reuse_slot() {
        let mut store = SignalStore::new();
        let sig1 = store.create(100i32);
        let old_index = sig1.id.index;
        store.dispose(sig1);

        // New signal should reuse the freed slot.
        let sig2 = store.create(200i32);
        assert_eq!(sig2.id.index, old_index);
        assert_ne!(sig2.id.generation, sig1.id.generation);
        assert_eq!(store.read(sig2), 200);
    }

    #[test]
    #[should_panic(expected = "stale signal handle")]
    fn stale_handle_panics_on_read() {
        let mut store = SignalStore::new();
        let sig = store.create(1i32);
        store.dispose(sig);
        let _new = store.create(2i32); // reuses slot with new generation
        store.read(sig); // stale handle — should panic
    }

    #[test]
    fn different_types_coexist() {
        let mut store = SignalStore::new();
        let int_sig = store.create(42i32);
        let str_sig = store.create(String::from("hello"));
        let bool_sig = store.create(true);

        assert_eq!(store.read(int_sig), 42);
        assert_eq!(store.read(str_sig), "hello");
        assert_eq!(store.read(bool_sig), true);
    }

    #[test]
    fn len_tracks_live_signals() {
        let mut store = SignalStore::new();
        assert_eq!(store.len(), 0);
        let a = store.create(1);
        let b = store.create(2);
        assert_eq!(store.len(), 2);
        store.dispose(a);
        assert_eq!(store.len(), 1);
        store.dispose(b);
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn signal_with_struct() {
        #[derive(Clone, Debug, PartialEq)]
        struct FileEntry {
            path: String,
            selected: bool,
        }

        let mut store = SignalStore::new();
        let sig = store.create(FileEntry {
            path: "src/main.rs".into(),
            selected: false,
        });

        store.update(sig, |f| f.selected = true);

        let entry = store.read(sig);
        assert!(entry.selected);
        assert_eq!(entry.path, "src/main.rs");
    }

    #[test]
    fn with_tracking_captures_reads() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);
        let b = store.create(2i32);
        let c = store.create(3i32);

        let (sum, deps) = with_tracking(|| {
            store.read(a) + store.read(b)
        });

        assert_eq!(sum, 3);
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&a.id));
        assert!(deps.contains(&b.id));
        assert!(!deps.contains(&c.id));
    }

    #[test]
    fn nested_tracking_scopes_independent() {
        let mut store = SignalStore::new();
        let a = store.create(10i32);
        let b = store.create(20i32);

        let (_, outer_deps) = with_tracking(|| {
            store.read(a);

            let (_, inner_deps) = with_tracking(|| {
                store.read(b);
            });

            assert_eq!(inner_deps.len(), 1);
            assert!(inner_deps.contains(&b.id));
        });

        assert_eq!(outer_deps.len(), 1);
        assert!(outer_deps.contains(&a.id));
        assert!(!outer_deps.contains(&b.id));
    }

    #[test]
    fn read_untracked_not_captured() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);
        let b = store.create(2i32);

        let (_, deps) = with_tracking(|| {
            store.read(a);
            store.read_untracked(b);
        });

        assert_eq!(deps.len(), 1);
        assert!(deps.contains(&a.id));
        assert!(!deps.contains(&b.id));
    }

    #[test]
    fn write_marks_dirty() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);
        let b = store.create(2i32);

        assert!(!store.is_dirty(a.id));
        assert!(!store.is_dirty(b.id));

        store.write(a, 10);
        assert!(store.is_dirty(a.id));
        assert!(!store.is_dirty(b.id));
    }

    #[test]
    fn update_marks_dirty() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);

        assert!(!store.is_dirty(a.id));
        store.update(a, |v| *v += 1);
        assert!(store.is_dirty(a.id));
    }

    #[test]
    fn clear_dirty_resets_all() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);
        let b = store.create(2i32);

        store.write(a, 10);
        store.write(b, 20);
        assert!(store.is_dirty(a.id));
        assert!(store.is_dirty(b.id));

        store.clear_dirty();
        assert!(!store.is_dirty(a.id));
        assert!(!store.is_dirty(b.id));
    }

    #[test]
    fn duplicate_reads_deduped() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);

        let (_, deps) = with_tracking(|| {
            store.read(a);
            store.read(a);
            store.read(a);
        });

        assert_eq!(deps.len(), 1);
        assert!(deps.contains(&a.id));
    }

    #[test]
    fn memo_computes_initial_value() {
        let mut store = SignalStore::new();
        let a = store.create(3i32);
        let b = store.create(7i32);

        let sum = store.create_memo(move |s| s.read(a) + s.read(b));

        assert_eq!(store.read(sum), 10);
        assert!(store.is_memo(sum.id));
    }

    #[test]
    fn memo_recomputes_when_dependency_changes() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);
        let b = store.create(2i32);
        let sum = store.create_memo(move |s| s.read(a) + s.read(b));

        assert_eq!(store.read(sum), 3);

        store.write(a, 10);
        assert!(store.is_dirty(sum.id), "memo should be marked dirty");

        store.update_memos();
        assert_eq!(store.read(sum), 12);
    }

    #[test]
    fn memo_tracks_new_dependencies() {
        let mut store = SignalStore::new();
        let flag = store.create(true);
        let a = store.create(100i32);
        let b = store.create(200i32);

        // Memo reads a or b depending on flag.
        let val = store.create_memo(move |s| {
            if s.read(flag) { s.read(a) } else { s.read(b) }
        });

        assert_eq!(store.read(val), 100);

        // Change flag → memo should now depend on b.
        store.write(flag, false);
        store.update_memos();
        assert_eq!(store.read(val), 200);

        // Change b → memo should update.
        store.write(b, 999);
        store.update_memos();
        assert_eq!(store.read(val), 999);
    }

    #[test]
    fn write_propagates_to_subscribers() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);
        let doubled = store.create_memo(move |s| s.read(a) * 2);

        assert_eq!(store.read(doubled), 2);

        store.write(a, 5);
        assert!(store.is_dirty(doubled.id));

        store.update_memos();
        assert_eq!(store.read(doubled), 10);
        assert!(!store.is_dirty(doubled.id));
    }

    #[test]
    fn chained_memos() {
        let mut store = SignalStore::new();
        let base = store.create(2i32);
        let doubled = store.create_memo(move |s| s.read(base) * 2);
        let quadrupled = store.create_memo(move |s| s.read(doubled) * 2);

        assert_eq!(store.read(base), 2);
        assert_eq!(store.read(doubled), 4);
        assert_eq!(store.read(quadrupled), 8);

        store.write(base, 3);
        store.update_memos();

        assert_eq!(store.read(doubled), 6);
        assert_eq!(store.read(quadrupled), 12);
    }

    #[test]
    fn any_dirty_reflects_state() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);

        assert!(!store.any_dirty());
        store.write(a, 2);
        assert!(store.any_dirty());
        store.clear_dirty();
        assert!(!store.any_dirty());
    }

    #[test]
    fn dispose_memo_cleans_up_subscribers() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);
        let m = store.create_memo(move |s| s.read(a) + 1);

        assert_eq!(store.read(m), 2);

        store.dispose(m);

        // Writing a should not panic even though the memo is gone.
        store.write(a, 10);
        store.update_memos();
    }
}
