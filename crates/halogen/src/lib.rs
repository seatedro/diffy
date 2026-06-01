//! Halogen — declarative UI toolkit with fine-grained reactivity.
//!
//! - `view!` declarative macro (re-exported from `halogen_macros`)
//! - `reactive` module with `Signal`, `SignalStore`, memos, tracking
//! - `geometry::Rect` — pure 2D rectangle
//! - `hit` — pointer hit-testing primitives generic over a click-result payload
//! - `scene` — immediate-mode render primitives
//! - `semantic` — retained native UI semantics for accessibility, focus,
//!   events, hit testing, and devtools

pub use halogen_macros::{Store, view};

pub mod color;
pub mod event;
pub mod focus;
pub mod geometry;
pub mod hit;
pub mod identity;
pub mod reactive;
pub mod retained;
pub mod scene;
pub mod semantic;
pub mod style;
pub mod style_state;

pub use color::Color;
pub use event::{
    DragSession, PointerCapture, RoutedEventStep, UiEventBinding, UiEventKind, UiEventPhase,
    UiEventPropagation, UiEventResult, UiEventRoute,
};
pub use focus::{FocusNode, FocusScopeId, FocusTree, KeyContext, TabStop};
pub use geometry::Rect;
pub use hit::{
    ClickEvent, ClickHandler, CursorHint, HitIdentity, HitRegion, Hitbox, HitboxBehavior, HitboxId,
    TooltipRegion, resolve_hovered,
};
pub use identity::{TestId, UiKey, UiNodeId};
pub use retained::{DisposedNode, RetainedNode, RetainedTree};
pub use scene::{
    BlurRegionPrimitive, BorderPrimitive, ClipPrimitive, EffectQuadPrimitive, EffectType, FontKind,
    FontWeight, IconPrimitive, ImagePrimitive, Primitive, RectPrimitive, RichTextPrimitive,
    RichTextSpan, RoundedRectPrimitive, Scene, ShadowPrimitive, TextPrimitive,
};
pub use semantic::{
    SemanticActions, SemanticFrame, SemanticNode, SemanticNodeState, SemanticRole, dump_semantic,
};
pub use style::{BackgroundEffect, ElementStyle, ShadowStyle, StyleOverride, apply_override};
pub use style_state::{StyleInvalidation, StyleInvalidationReason, StyleState};
