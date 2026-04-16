//! Pointer hit-testing primitives. Generic over the click-result payload so
//! halogen stays independent of the hosting app's action enum.
//!
//! Diffy instantiates these with its own `ClickResult` enum via type aliases.

use std::rc::Rc;

use crate::geometry::Rect;

/// Requested cursor shape for a hit region.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorHint {
    #[default]
    Default,
    Pointer,
    Text,
    ResizeCol,
}

/// Opaque identity payload for hover routing. Lets halogen-owned code
/// answer "which file/toast/entry is hovered?" without pattern-matching on
/// the app's action enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitIdentity {
    File(usize),
    Toast(usize),
    OverlayEntry(usize),
    OverlayBackdrop,
}

/// A pointer click at a specific position. Coordinates are in the same
/// space as the HitRegion's rect.
#[derive(Debug, Clone, Copy)]
pub struct ClickEvent {
    pub x: f32,
    pub y: f32,
}

/// Click callback producing some app-defined result `R`. Stored as `Rc<Fn>`
/// so it can be cheaply cloned and invoked multiple times (e.g. tests
/// peeking the outcome without consuming).
///
/// `Clone` is a manual impl that doesn't require `R: Clone` — the internal
/// `Rc` is always cheap to clone regardless of `R`.
pub struct ClickHandler<R: 'static>(Rc<dyn Fn(ClickEvent) -> R>);

impl<R: 'static> Clone for ClickHandler<R> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<R: 'static> ClickHandler<R> {
    pub fn new(f: impl Fn(ClickEvent) -> R + 'static) -> Self {
        Self(Rc::new(f))
    }

    pub fn invoke(&self, event: ClickEvent) -> R {
        (self.0)(event)
    }
}

impl<R: 'static> std::fmt::Debug for ClickHandler<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClickHandler(..)")
    }
}

/// A pointer-interactive rectangle collected during paint. The app drains
/// these off of the built UI frame each tick and dispatches through them.
///
/// `Clone` and `Debug` are implemented manually so callers can specialize
/// `R` with types (like an app's `ClickResult`) that themselves don't
/// implement those traits — the struct's cloneability comes from the
/// internal `Rc`, not from `R`.
pub struct HitRegion<R: 'static> {
    pub rect: Rect,
    pub cursor: CursorHint,
    pub on_click: ClickHandler<R>,
    pub identity: Option<HitIdentity>,
}

impl<R: 'static> Clone for HitRegion<R> {
    fn clone(&self) -> Self {
        Self {
            rect: self.rect,
            cursor: self.cursor,
            on_click: self.on_click.clone(),
            identity: self.identity,
        }
    }
}

impl<R: 'static> std::fmt::Debug for HitRegion<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HitRegion")
            .field("rect", &self.rect)
            .field("cursor", &self.cursor)
            .field("identity", &self.identity)
            .finish()
    }
}

impl<R: 'static> HitRegion<R> {
    pub fn new(rect: Rect, cursor: CursorHint, on_click: ClickHandler<R>) -> Self {
        Self {
            rect,
            cursor,
            on_click,
            identity: None,
        }
    }

    pub fn with_identity(mut self, identity: HitIdentity) -> Self {
        self.identity = Some(identity);
        self
    }
}

// ---------------------------------------------------------------------------
// Hitbox — prepaint-phase interaction regions with z-ordering and blocking
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HitboxId(usize);

impl HitboxId {
    pub fn new(id: usize) -> Self {
        Self(id)
    }

    pub fn raw(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitboxBehavior {
    Normal,
    BlockMouse,
}

#[derive(Debug, Clone)]
pub struct Hitbox {
    pub id: HitboxId,
    pub bounds: Rect,
    pub behavior: HitboxBehavior,
    pub z_index: i32,
}

/// A tooltip-bearing rectangle collected during paint. The host app drains
/// these off the built UI frame each tick and renders the tooltip when the
/// cursor lingers inside `bounds`.
#[derive(Debug, Clone)]
pub struct TooltipRegion {
    pub bounds: Rect,
    pub text: String,
}

/// Resolve which hitboxes are hovered given the current mouse position.
///
/// Walks candidates whose bounds contain the mouse, orders them by z-index
/// descending (with last-registered winning ties), then culls any hitbox
/// behind a `BlockMouse` whose bounds overlap the blocker.
pub fn resolve_hovered(hitboxes: &[Hitbox], mouse: Option<(f32, f32)>) -> Vec<HitboxId> {
    let mouse = match mouse {
        Some(pos) => pos,
        None => return Vec::new(),
    };

    let mut candidates: Vec<(HitboxId, Rect, HitboxBehavior, i32)> = Vec::new();
    for hb in hitboxes {
        if hb.bounds.contains(mouse.0, mouse.1) {
            candidates.push((hb.id, hb.bounds, hb.behavior, hb.z_index));
        }
    }

    candidates.reverse();
    candidates.sort_by(|a, b| b.3.cmp(&a.3));

    let mut hovered = Vec::new();
    let mut blocked_regions: Vec<Rect> = Vec::new();

    for &(id, bounds, behavior, _z) in &candidates {
        let is_blocked = blocked_regions
            .iter()
            .any(|blocker| blocker.intersection(bounds).is_some());

        if !is_blocked {
            hovered.push(id);
        }

        if behavior == HitboxBehavior::BlockMouse {
            blocked_regions.push(bounds);
        }
    }

    hovered
}
