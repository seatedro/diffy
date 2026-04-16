//! Halogen — declarative UI toolkit with fine-grained reactivity.
//!
//! - `view!` declarative macro (re-exported from `halogen_macros`)
//! - `reactive` module with `Signal`, `SignalStore`, memos, tracking
//! - `geometry::Rect` — pure 2D rectangle
//! - `hit` — pointer hit-testing primitives generic over a click-result payload

pub use halogen_macros::{Store, view};

pub mod geometry;
pub mod hit;
pub mod reactive;

pub use geometry::Rect;
pub use hit::{ClickEvent, ClickHandler, CursorHint, HitIdentity, HitRegion};
