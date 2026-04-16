//! Halogen — declarative UI toolkit with fine-grained reactivity.
//!
//! - `view!` declarative macro (re-exported from `halogen_macros`)
//! - `reactive` module with `Signal`, `SignalStore`, memos, tracking
//! - `geometry::Rect` — pure 2D rectangle
//! - `hit` — pointer hit-testing primitives generic over a click-result payload
//! - `scene` — immediate-mode render primitives

pub use halogen_macros::{Store, view};

pub mod color;
pub mod geometry;
pub mod hit;
pub mod reactive;
pub mod scene;
pub mod style;

pub use color::Color;
pub use geometry::Rect;
pub use hit::{
    ClickEvent, ClickHandler, CursorHint, Hitbox, HitboxBehavior, HitboxId, HitIdentity,
    HitRegion, resolve_hovered,
};
pub use scene::{
    BlurRegionPrimitive, BorderPrimitive, ClipPrimitive, EditorTextSlot, EffectQuadPrimitive,
    EffectType, FontKind, FontWeight, IconPrimitive, ImagePrimitive, Primitive, RectPrimitive,
    RichTextPrimitive, RichTextSpan, RoundedRectPrimitive, Scene, ShadowPrimitive, TextPrimitive,
};
pub use style::{ElementStyle, ShadowStyle, StyleOverride, apply_override};
