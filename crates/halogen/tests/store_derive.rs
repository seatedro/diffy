//! Integration tests for `#[derive(Store)]`.

use halogen::Store;
use halogen::reactive::SignalStore;

#[derive(Debug, Clone, Default, PartialEq, Store)]
pub struct Pane {
    pub scroll_px: f32,
    pub hovered: Option<usize>,
    pub filter: String,
}

#[derive(Debug, Clone, Default, PartialEq, Store)]
pub struct PaneDebug {
    pub last_primitive_count: usize,
    pub last_frame_us: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Store)]
pub struct Workspace {
    pub ready: bool,
    #[store(flatten)]
    pub pane: Pane,
    #[store(flatten)]
    pub debug: PaneDebug,
    #[store(skip)]
    pub callbacks: Vec<String>, // stays out of the store
}

#[test]
fn generates_store_with_leaf_signals() {
    let store = SignalStore::default();
    let initial = Pane {
        scroll_px: 42.5,
        hovered: Some(3),
        filter: "foo".into(),
    };
    let pane = PaneStore::new(&store, initial);

    assert_eq!(store.read(pane.scroll_px), 42.5);
    assert_eq!(store.read(pane.hovered), Some(3));
    assert_eq!(store.read(pane.filter), "foo");
}

#[test]
fn store_is_copy() {
    let store = SignalStore::default();
    let pane = PaneStore::new(&store, Pane::default());
    let pane2 = pane; // Copy
    let pane3 = pane;
    assert_eq!(store.read(pane2.scroll_px), 0.0);
    assert_eq!(store.read(pane3.scroll_px), 0.0);
}

#[test]
fn writes_propagate_to_any_dirty() {
    let store = SignalStore::default();
    let pane = PaneStore::new(&store, Pane::default());
    assert!(!store.any_dirty());
    store.write(pane.scroll_px, 100.0);
    assert!(store.any_dirty());
    assert_eq!(store.read(pane.scroll_px), 100.0);
}

#[test]
fn flatten_nests_stores() {
    let store = SignalStore::default();
    let ws = WorkspaceStore::new(
        &store,
        Workspace {
            ready: true,
            pane: Pane {
                scroll_px: 7.0,
                hovered: None,
                filter: "bar".into(),
            },
            debug: PaneDebug {
                last_primitive_count: 1000,
                last_frame_us: 16_666,
            },
            callbacks: vec!["ignored".into()],
        },
    );

    assert!(store.read(ws.ready));
    assert_eq!(store.read(ws.pane.scroll_px), 7.0);
    assert_eq!(store.read(ws.pane.filter), "bar");
    assert_eq!(store.read(ws.debug.last_frame_us), 16_666);
}

#[test]
fn set_if_changed_works_on_generated_signals() {
    let store = SignalStore::default();
    let pane = PaneStore::new(&store, Pane::default());

    assert!(store.set_if_changed(pane.scroll_px, 5.0));
    store.clear_dirty();
    // Writing the same value should be a no-op (no dirty).
    assert!(!store.set_if_changed(pane.scroll_px, 5.0));
    assert!(!store.any_dirty());
    // Different value → dirty.
    assert!(store.set_if_changed(pane.scroll_px, 7.0));
    assert!(store.any_dirty());
}

#[test]
fn snapshot_reconstructs_original() {
    let store = SignalStore::default();
    let initial = Pane {
        scroll_px: 12.5,
        hovered: Some(9),
        filter: "hello".into(),
    };
    let pane = PaneStore::new(&store, initial.clone());

    assert_eq!(pane.snapshot(&store), initial);

    // Mutations reflect in the snapshot.
    store.write(pane.scroll_px, 99.0);
    store.write(pane.filter, "world".into());
    let snap = pane.snapshot(&store);
    assert_eq!(snap.scroll_px, 99.0);
    assert_eq!(snap.hovered, Some(9));
    assert_eq!(snap.filter, "world");
}

#[test]
fn snapshot_recurses_through_flatten() {
    let store = SignalStore::default();
    // Workspace has #[store(skip)] for callbacks, so snapshot is NOT generated.
    // Build a pure nested store to test snapshot recursion.
    #[derive(Debug, Clone, Default, PartialEq, halogen::Store)]
    struct Outer {
        pub flag: bool,
        #[store(flatten)]
        pub inner: Pane,
    }

    let outer = OuterStore::new(
        &store,
        Outer {
            flag: true,
            inner: Pane {
                scroll_px: 1.5,
                hovered: None,
                filter: "x".into(),
            },
        },
    );
    let snap = outer.snapshot(&store);
    assert!(snap.flag);
    assert_eq!(snap.inner.scroll_px, 1.5);
    assert_eq!(snap.inner.filter, "x");
}

#[test]
fn new_default_constructs_from_default() {
    let store = SignalStore::default();
    let pane = PaneStore::new_default(&store);
    assert_eq!(pane.snapshot(&store), Pane::default());
}

#[test]
fn signal_ergonomic_methods() {
    let store = SignalStore::default();
    let pane = PaneStore::new(&store, Pane::default());

    // .set / .get
    pane.scroll_px.set(&store, 42.0);
    assert_eq!(pane.scroll_px.get(&store), 42.0);

    // .update
    pane.scroll_px.update(&store, |v| *v *= 2.0);
    assert_eq!(pane.scroll_px.get(&store), 84.0);

    // .with (no clone)
    let len = pane.filter.with(&store, |s| s.len());
    assert_eq!(len, 0);

    // .set_if_changed
    store.clear_dirty();
    assert!(!pane.scroll_px.set_if_changed(&store, 84.0));
    assert!(!store.any_dirty());
    assert!(pane.scroll_px.set_if_changed(&store, 85.0));
    assert!(store.any_dirty());
}
