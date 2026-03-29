pub mod renderer;
pub mod scene;
#[cfg(feature = "capture")]
pub mod capture;

pub use renderer::{FrameStats, OffscreenTarget, RenderError, Renderer, TextMetrics};
pub use scene::{
    BlurRegionPrimitive, BorderPrimitive, ClipPrimitive, EffectQuadPrimitive, EffectType, FontKind,
    Primitive, Rect, RectPrimitive, RichTextPrimitive, RichTextSpan, RoundedRectPrimitive, Scene,
    ShadowPrimitive, TextPrimitive,
};
