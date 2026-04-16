//! Fine-grained reactive signals.
//!
//! `Signal<T>` is a Copy handle (8 bytes) into a persistent `SignalStore`.
//! Values live in a slot arena. Reads are automatically tracked by the current
//! observer scope; writes mark subscribers for lazy recomputation.
//!
//! The reactive graph has three states per node: `Clean`, `Check`, `Dirty`.
//! Writes mark the source `Dirty` and all transitive subscribers `Check`.
//! Memos recompute lazily on read; if their recomputed value equals the
//! previous value (`PartialEq`), dependents stay `Clean` — no wasted work.

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

pub struct Signal<T> {
    pub(crate) id: SignalId,
    _marker: PhantomData<fn() -> T>,
}

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
// Signal<T> ergonomic methods — shorter left-to-right calls.
// `sig.get(&store)` instead of `store.read(sig)`.
// ---------------------------------------------------------------------------

impl<T: 'static + Clone> Signal<T> {
    /// Shorthand for `store.read(self)`. Tracked by the current observer.
    #[inline]
    pub fn get(self, store: &SignalStore) -> T {
        store.read(self)
    }

    /// Shorthand for `store.read_untracked(self)`.
    #[inline]
    pub fn get_untracked(self, store: &SignalStore) -> T {
        store.read_untracked(self)
    }
}

impl<T: 'static> Signal<T> {
    /// Shorthand for `store.write(self, value)`.
    #[inline]
    pub fn set(self, store: &SignalStore, value: T) {
        store.write(self, value);
    }

    /// Shorthand for `store.update(self, f)`.
    #[inline]
    pub fn update(self, store: &SignalStore, f: impl FnOnce(&mut T)) {
        store.update(self, f);
    }

    /// Shorthand for `store.with(self, f)`.
    #[inline]
    pub fn with<R>(self, store: &SignalStore, f: impl FnOnce(&T) -> R) -> R {
        store.with(self, f)
    }
}

impl<T: 'static + PartialEq> Signal<T> {
    /// Shorthand for `store.set_if_changed(self, value)`.
    #[inline]
    pub fn set_if_changed(self, store: &SignalStore, value: T) -> bool {
        store.set_if_changed(self, value)
    }
}

// ---------------------------------------------------------------------------
// Observer — thread-local dependency tracker
// ---------------------------------------------------------------------------

struct TrackingScope {
    dependencies: Vec<SignalId>,
}

thread_local! {
    static OBSERVER: RefCell<Option<TrackingScope>> = const { RefCell::new(None) };
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

    let scope = OBSERVER
        .with(|obs| obs.borrow_mut().take())
        .expect("tracking scope disappeared during with_tracking");

    OBSERVER.with(|obs| {
        *obs.borrow_mut() = prev;
    });

    (result, scope.dependencies)
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Clean,
    Check,
    Dirty,
}

struct Slot {
    value: Option<Box<dyn Any>>,
    generation: u32,
}

/// Compute callback for a memo. Given the store and an optional reference to
/// the previous value, returns the new value plus a bool indicating whether
/// the value actually changed (`true`) or stayed equal (`false`).
type MemoFn = Box<dyn Fn(&SignalStore, Option<&dyn Any>) -> (Box<dyn Any>, bool)>;

struct Inner {
    slots: Vec<Slot>,
    free_list: Vec<u32>,
    /// Downstream edges: `subscribers[i]` = slots that depend on `i`.
    subscribers: Vec<Vec<u32>>,
    /// Upstream edges (memos only): `sources[i]` = slots `i` depends on.
    sources: Vec<Vec<u32>>,
    state: Vec<SlotState>,
    memo_fns: Vec<Option<MemoFn>>,
    /// Any signal ever written since the last `clear_dirty()` — used by the
    /// frame loop to decide whether to rerender. Separate from per-slot state
    /// because states can return to Clean during realize().
    any_dirty: bool,
}

// ---------------------------------------------------------------------------
// SignalStore
// ---------------------------------------------------------------------------

/// Persistent store for signal values. Lives in the app, survives across frames.
pub struct SignalStore {
    inner: RefCell<Inner>,
}

impl std::fmt::Debug for SignalStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner.try_borrow() {
            Ok(inner) => f
                .debug_struct("SignalStore")
                .field(
                    "len",
                    &inner.slots.iter().filter(|s| s.value.is_some()).count(),
                )
                .field("any_dirty", &inner.any_dirty)
                .finish(),
            Err(_) => f.write_str("SignalStore { <borrowed> }"),
        }
    }
}

impl SignalStore {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(Inner {
                slots: Vec::new(),
                free_list: Vec::new(),
                subscribers: Vec::new(),
                sources: Vec::new(),
                state: Vec::new(),
                memo_fns: Vec::new(),
                any_dirty: false,
            }),
        }
    }

    /// Shared-borrow helper. Panics with a clear message on re-entrancy
    /// (e.g. attempting a write from inside a read or vice versa).
    fn inner_ref(&self) -> std::cell::Ref<'_, Inner> {
        self.inner
            .try_borrow()
            .expect("signal store re-entrancy: tried to read while a write is in progress")
    }

    /// Exclusive-borrow helper. Panics on re-entrancy.
    fn inner_mut(&self) -> std::cell::RefMut<'_, Inner> {
        self.inner
            .try_borrow_mut()
            .expect("signal store re-entrancy: tried to write while another access is in progress")
    }

    /// Create a new signal with the given initial value.
    pub fn create<T: 'static>(&self, value: T) -> Signal<T> {
        let boxed: Box<dyn Any> = Box::new(value);
        let mut inner = self.inner_mut();
        let inner = &mut *inner;

        let (index, generation) = if let Some(idx) = inner.free_list.pop() {
            let slot = &mut inner.slots[idx as usize];
            slot.value = Some(boxed);
            inner.subscribers[idx as usize].clear();
            inner.sources[idx as usize].clear();
            inner.state[idx as usize] = SlotState::Clean;
            inner.memo_fns[idx as usize] = None;
            (idx, slot.generation)
        } else {
            let idx = inner.slots.len() as u32;
            inner.slots.push(Slot {
                value: Some(boxed),
                generation: 0,
            });
            inner.subscribers.push(Vec::new());
            inner.sources.push(Vec::new());
            inner.state.push(SlotState::Clean);
            inner.memo_fns.push(None);
            (idx, 0)
        };

        Signal {
            id: SignalId { index, generation },
            _marker: PhantomData,
        }
    }

    /// Create a derived signal (memo) whose value is computed from other signals.
    /// The compute function runs lazily on first read. Propagation to dependents
    /// is skipped when the recomputed value equals the previous one (`PartialEq`).
    pub fn create_memo<T: 'static + Clone + PartialEq>(
        &self,
        compute: impl Fn(&SignalStore) -> T + 'static,
    ) -> Signal<T> {
        // Allocate an empty slot; value is produced on first read.
        let signal: Signal<T> = {
            let mut inner = self.inner_mut();
            let inner = &mut *inner;
            let (index, generation) = if let Some(idx) = inner.free_list.pop() {
                let slot = &mut inner.slots[idx as usize];
                slot.value = None;
                inner.subscribers[idx as usize].clear();
                inner.sources[idx as usize].clear();
                inner.state[idx as usize] = SlotState::Dirty;
                inner.memo_fns[idx as usize] = None;
                (idx, slot.generation)
            } else {
                let idx = inner.slots.len() as u32;
                inner.slots.push(Slot {
                    value: None,
                    generation: 0,
                });
                inner.subscribers.push(Vec::new());
                inner.sources.push(Vec::new());
                inner.state.push(SlotState::Dirty);
                inner.memo_fns.push(None);
                (idx, 0)
            };
            Signal {
                id: SignalId { index, generation },
                _marker: PhantomData,
            }
        };

        // Wrap compute with PartialEq comparison against the prior cached value.
        let memo_fn: MemoFn = Box::new(move |store, prev: Option<&dyn Any>| {
            let new_value = compute(store);
            let changed = match prev.and_then(|p| p.downcast_ref::<T>()) {
                Some(old) => *old != new_value,
                None => true,
            };
            (Box::new(new_value) as Box<dyn Any>, changed)
        });

        self.inner_mut().memo_fns[signal.id.index as usize] = Some(memo_fn);
        signal
    }

    /// Read a signal's value (clones it). Registers this signal with the
    /// current tracking scope, if one exists.
    pub fn read<T: 'static + Clone>(&self, signal: Signal<T>) -> T {
        self.with(signal, Clone::clone)
    }

    /// Read a signal's value without registering a dependency.
    pub fn read_untracked<T: 'static + Clone>(&self, signal: Signal<T>) -> T {
        self.with_untracked(signal, Clone::clone)
    }

    /// Access a signal's value by reference. Registers this signal with the
    /// current tracking scope, if one exists.
    pub fn with<T: 'static, R>(&self, signal: Signal<T>, f: impl FnOnce(&T) -> R) -> R {
        track_read(signal.id);
        self.with_untracked(signal, f)
    }

    fn with_untracked<T: 'static, R>(&self, signal: Signal<T>, f: impl FnOnce(&T) -> R) -> R {
        self.realize(signal.id.index as usize);
        let inner = self.inner_ref();
        let slot = &inner.slots[signal.id.index as usize];
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
    pub fn write<T: 'static>(&self, signal: Signal<T>, value: T) {
        let idx = signal.id.index as usize;
        {
            let mut inner = self.inner_mut();
            let slot = &mut inner.slots[idx];
            assert_eq!(
                slot.generation, signal.id.generation,
                "stale signal handle (generation mismatch)"
            );
            slot.value = Some(Box::new(value));
        }
        self.mark_source_dirty(idx);
    }

    /// Write only if the new value differs from the current one (`PartialEq`).
    /// Returns `true` if the write happened. Use when pushing values that may
    /// be equal frame-to-frame so stable values don't re-dirty subscribers.
    pub fn set_if_changed<T: 'static + PartialEq>(&self, signal: Signal<T>, value: T) -> bool {
        let idx = signal.id.index as usize;
        {
            let mut inner = self.inner_mut();
            let slot = &mut inner.slots[idx];
            assert_eq!(
                slot.generation, signal.id.generation,
                "stale signal handle (generation mismatch)"
            );
            if let Some(cur) = slot.value.as_ref().and_then(|b| b.downcast_ref::<T>())
                && *cur == value
            {
                return false;
            }
            slot.value = Some(Box::new(value));
        }
        self.mark_source_dirty(idx);
        true
    }

    /// Mutate a signal's value in place and propagate dirtiness to subscribers.
    pub fn update<T: 'static>(&self, signal: Signal<T>, f: impl FnOnce(&mut T)) {
        let idx = signal.id.index as usize;
        {
            let mut inner = self.inner_mut();
            let slot = &mut inner.slots[idx];
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
        }
        self.mark_source_dirty(idx);
    }

    /// Dispose a signal, freeing its slot for reuse.
    pub fn dispose<T>(&self, signal: Signal<T>) {
        let idx = signal.id.index as usize;
        let mut inner = self.inner_mut();
        let inner = &mut *inner;
        let slot = &mut inner.slots[idx];
        if slot.generation == signal.id.generation {
            slot.value = None;
            slot.generation = slot.generation.wrapping_add(1);
            inner.memo_fns[idx] = None;
            for subs in &mut inner.subscribers {
                subs.retain(|&s| s != signal.id.index);
            }
            for srcs in &mut inner.sources {
                srcs.retain(|&s| s != signal.id.index);
            }
            inner.subscribers[idx].clear();
            inner.sources[idx].clear();
            inner.state[idx] = SlotState::Clean;
            inner.free_list.push(signal.id.index);
        }
    }

    pub fn mark_dirty(&self, signal_id: SignalId) {
        self.mark_source_dirty(signal_id.index as usize);
    }

    pub fn is_dirty(&self, signal_id: SignalId) -> bool {
        !matches!(
            self.inner_ref().state[signal_id.index as usize],
            SlotState::Clean
        )
    }

    /// Returns true if any signal has been written since the last `clear_dirty()`.
    /// Used by the frame loop to decide whether to rerender.
    pub fn any_dirty(&self) -> bool {
        self.inner_ref().any_dirty
    }

    pub fn clear_dirty(&self) {
        self.inner_mut().any_dirty = false;
    }

    /// Returns true if the given signal is a memo.
    pub fn is_memo(&self, signal_id: SignalId) -> bool {
        self.inner_ref()
            .memo_fns
            .get(signal_id.index as usize)
            .is_some_and(|f| f.is_some())
    }

    /// Number of live signals.
    pub fn len(&self) -> usize {
        self.inner_ref()
            .slots
            .iter()
            .filter(|s| s.value.is_some())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // -----------------------------------------------------------------------
    // Internal: dirty propagation & lazy realization
    // -----------------------------------------------------------------------

    /// Mark a raw signal `Dirty` and all transitive subscribers `Check`.
    fn mark_source_dirty(&self, idx: usize) {
        let mut inner = self.inner_mut();
        let inner = &mut *inner;
        inner.state[idx] = SlotState::Dirty;
        inner.any_dirty = true;

        // BFS: mark all transitive subscribers Check (unless already Dirty).
        let mut queue: Vec<u32> = inner.subscribers[idx].clone();
        let mut visited: Vec<bool> = vec![false; inner.slots.len()];
        while let Some(sub) = queue.pop() {
            let sub_idx = sub as usize;
            if visited[sub_idx] {
                continue;
            }
            visited[sub_idx] = true;
            if inner.state[sub_idx] == SlotState::Clean {
                inner.state[sub_idx] = SlotState::Check;
            }
            for &t in &inner.subscribers[sub_idx] {
                if !visited[t as usize] {
                    queue.push(t);
                }
            }
        }
    }

    /// Ensure the slot's value is up-to-date. Returns `true` if the slot's
    /// value changed as a result of this call (a fresh write for raw signals,
    /// or a PartialEq-distinct recomputation for memos).
    fn realize(&self, idx: usize) -> bool {
        let state = self.inner_ref().state[idx];
        let is_memo = self.inner_ref().memo_fns[idx].is_some();
        match state {
            SlotState::Clean => false,
            SlotState::Dirty => {
                if is_memo {
                    self.recompute_memo(idx)
                } else {
                    // Raw signal: Dirty = "recently written" = value changed.
                    self.inner_mut().state[idx] = SlotState::Clean;
                    true
                }
            }
            SlotState::Check => {
                // Only memos can be Check.
                debug_assert!(is_memo, "raw signal in Check state");
                let srcs = self.inner_ref().sources[idx].clone();
                let mut any_source_changed = false;
                for src in srcs {
                    if self.realize(src as usize) {
                        any_source_changed = true;
                    }
                }
                if any_source_changed {
                    self.recompute_memo(idx)
                } else {
                    self.inner_mut().state[idx] = SlotState::Clean;
                    false
                }
            }
        }
    }

    /// Recompute a memo. Captures new dependencies via `with_tracking`; updates
    /// subscriber/source edges; PartialEq-compares result to previous value.
    /// When the value changes, escalates all transitive subscribers to Dirty so
    /// their own realize() will actually recompute instead of short-circuiting.
    /// Returns `true` if the new value differs from the cached one.
    fn recompute_memo(&self, idx: usize) -> bool {
        let memo_fn = self.inner_mut().memo_fns[idx]
            .take()
            .expect("recompute_memo on non-memo slot");
        let prev = self.inner_mut().slots[idx].value.take();

        let (result, new_deps) = with_tracking(|| memo_fn(self, prev.as_deref()));
        let (new_value, changed) = result;

        {
            let mut inner = self.inner_mut();
            inner.slots[idx].value = Some(new_value);
            inner.memo_fns[idx] = Some(memo_fn);
            inner.state[idx] = SlotState::Clean;
        }

        self.rewire_sources(idx as u32, new_deps);

        if changed {
            // This memo's value actually changed — promote Check subs to Dirty.
            let mut inner = self.inner_mut();
            let mut queue: Vec<u32> = inner.subscribers[idx].clone();
            let mut visited = vec![false; inner.slots.len()];
            while let Some(sub) = queue.pop() {
                let si = sub as usize;
                if visited[si] {
                    continue;
                }
                visited[si] = true;
                inner.state[si] = SlotState::Dirty;
                for &t in &inner.subscribers[si] {
                    if !visited[t as usize] {
                        queue.push(t);
                    }
                }
            }
        }

        changed
    }

    fn rewire_sources(&self, idx: u32, new_deps: Vec<SignalId>) {
        let mut inner = self.inner_mut();
        // Remove `idx` from every old source's subscriber list.
        let old_sources = std::mem::take(&mut inner.sources[idx as usize]);
        for src in old_sources {
            inner.subscribers[src as usize].retain(|&s| s != idx);
        }
        // Install new sources + register self as subscriber on each.
        let mut new_src_indices = Vec::with_capacity(new_deps.len());
        for dep in new_deps {
            let di = dep.index;
            if !new_src_indices.contains(&di) {
                new_src_indices.push(di);
                let subs = &mut inner.subscribers[di as usize];
                if !subs.contains(&idx) {
                    subs.push(idx);
                }
            }
        }
        inner.sources[idx as usize] = new_src_indices;
    }
}

impl Default for SignalStore {
    fn default() -> Self {
        Self::new()
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
        let sig2 = sig;
        let sig3 = sig;
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
        let old_gen = sig1.id.generation;
        store.dispose(sig1);
        let sig2 = store.create(200i32);
        assert_eq!(sig2.id.index, old_index);
        assert_ne!(sig2.id.generation, old_gen);
        assert_eq!(store.read(sig2), 200);
    }

    #[test]
    #[should_panic(expected = "stale signal handle")]
    fn stale_handle_panics_on_read() {
        let mut store = SignalStore::new();
        let sig = store.create(1i32);
        store.dispose(sig);
        let _new = store.create(2i32);
        store.read(sig);
    }

    #[test]
    fn different_types_coexist() {
        let mut store = SignalStore::new();
        let int_sig = store.create(42i32);
        let str_sig = store.create(String::from("hello"));
        let bool_sig = store.create(true);
        assert_eq!(store.read(int_sig), 42);
        assert_eq!(store.read(str_sig), "hello");
        assert!(store.read(bool_sig));
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
        let (sum, deps) = with_tracking(|| store.read(a) + store.read(b));
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
    fn write_marks_dirty_bit() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);
        let b = store.create(2i32);
        assert!(!store.any_dirty());
        store.write(a, 10);
        assert!(store.any_dirty());
        let _ = b; // keep alive
    }

    #[test]
    fn clear_dirty_resets_any_dirty() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);
        store.write(a, 10);
        assert!(store.any_dirty());
        store.clear_dirty();
        assert!(!store.any_dirty());
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
        assert_eq!(store.read(sum), 12);
    }

    #[test]
    fn memo_tracks_new_dependencies() {
        let mut store = SignalStore::new();
        let flag = store.create(true);
        let a = store.create(100i32);
        let b = store.create(200i32);
        let val = store.create_memo(move |s| if s.read(flag) { s.read(a) } else { s.read(b) });
        assert_eq!(store.read(val), 100);
        store.write(flag, false);
        assert_eq!(store.read(val), 200);
        store.write(b, 999);
        assert_eq!(store.read(val), 999);
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
    fn dispose_memo_does_not_leak() {
        let mut store = SignalStore::new();
        let a = store.create(1i32);
        let m = store.create_memo(move |s| s.read(a) + 1);
        assert_eq!(store.read(m), 2);
        store.dispose(m);
        store.write(a, 10); // must not panic
    }

    /// When a memo recomputes unchanged, its subscribers that were Check
    /// stay Clean after their own realize — no downstream work.
    #[test]
    fn memo_does_not_recompute_when_source_returns_same_value() {
        use std::cell::Cell;
        use std::rc::Rc;

        let mut store = SignalStore::new();
        let a = store.create(5i32);
        let sign = store.create_memo(move |s| s.read(a).signum());

        let run_count = Rc::new(Cell::new(0u32));
        let rc2 = Rc::clone(&run_count);
        let dependent = store.create_memo(move |s| {
            rc2.set(rc2.get() + 1);
            s.read(sign) * 10
        });

        assert_eq!(store.read(dependent), 10);
        assert_eq!(run_count.get(), 1);

        // Change a 5→7: sign unchanged → dependent should NOT rerun.
        store.write(a, 7);
        assert_eq!(store.read(dependent), 10);
        assert_eq!(
            run_count.get(),
            1,
            "dependent memo re-ran despite source PartialEq-equal"
        );

        // Change a 7→-3: sign changes to -1 → dependent MUST rerun.
        store.write(a, -3);
        assert_eq!(store.read(dependent), -10);
        assert_eq!(run_count.get(), 2);
    }
}
