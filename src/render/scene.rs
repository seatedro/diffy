use crate::ui::theme::Color;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x <= self.right() && y <= self.bottom()
    }

    pub fn right(self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    pub fn inset(self, amount: f32) -> Self {
        Self {
            x: self.x + amount,
            y: self.y + amount,
            width: (self.width - amount * 2.0).max(0.0),
            height: (self.height - amount * 2.0).max(0.0),
        }
    }

    pub fn pad(self, left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            x: self.x + left,
            y: self.y + top,
            width: (self.width - left - right).max(0.0),
            height: (self.height - top - bottom).max(0.0),
        }
    }

    pub fn center(self, child_w: f32, child_h: f32) -> Self {
        Self {
            x: self.x + ((self.width - child_w).max(0.0) * 0.5),
            y: self.y + ((self.height - child_h).max(0.0) * 0.5),
            width: child_w.min(self.width - 24.0),
            height: child_h.min(self.height - 24.0),
        }
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        let width = right - left;
        let height = bottom - top;
        if width <= 0.0 || height <= 0.0 {
            None
        } else {
            Some(Self {
                x: left,
                y: top,
                width,
                height,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FontKind {
    #[default]
    Ui,
    Mono,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub primitives: Vec<Primitive>,
}

impl Scene {
    pub fn push(&mut self, primitive: Primitive) {
        self.primitives.push(primitive);
    }

    pub fn rect(&mut self, rect: RectPrimitive) {
        self.push(Primitive::Rect(rect));
    }

    pub fn rounded_rect(&mut self, rect: RoundedRectPrimitive) {
        self.push(Primitive::RoundedRect(rect));
    }

    pub fn border(&mut self, border: BorderPrimitive) {
        self.push(Primitive::Border(border));
    }

    pub fn shadow(&mut self, shadow: ShadowPrimitive) {
        self.push(Primitive::Shadow(shadow));
    }

    pub fn text(&mut self, text: TextPrimitive) {
        self.push(Primitive::TextRun(text));
    }

    pub fn rich_text(&mut self, text: RichTextPrimitive) {
        self.push(Primitive::RichTextRun(text));
    }

    pub fn blur_region(&mut self, blur: BlurRegionPrimitive) {
        self.push(Primitive::BlurRegion(blur));
    }

    pub fn effect_quad(&mut self, effect: EffectQuadPrimitive) {
        self.push(Primitive::EffectQuad(effect));
    }

    pub fn clip(&mut self, rect: Rect) {
        self.push(Primitive::ClipStart(ClipPrimitive { rect }));
    }

    pub fn pop_clip(&mut self) {
        self.push(Primitive::ClipEnd);
    }

    pub fn push_z_index(&mut self, z: i32) {
        self.push(Primitive::ZIndexPush(z));
    }

    pub fn pop_z_index(&mut self) {
        self.push(Primitive::ZIndexPop);
    }

    pub fn len(&self) -> usize {
        self.primitives.len()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    Rect(RectPrimitive),
    RoundedRect(RoundedRectPrimitive),
    Border(BorderPrimitive),
    Shadow(ShadowPrimitive),
    TextRun(TextPrimitive),
    RichTextRun(RichTextPrimitive),
    Icon(IconPrimitive),
    EffectQuad(EffectQuadPrimitive),
    /// Start a frosted-glass blur region. Content rendered before this
    /// primitive (within the given bounds) will be blurred and composited
    /// as a backdrop before children are painted on top.
    BlurRegion(BlurRegionPrimitive),
    ClipStart(ClipPrimitive),
    ClipEnd,
    /// Push a z-index context. Primitives inside render on top of lower z-indices.
    ZIndexPush(i32),
    /// Pop the current z-index context.
    ZIndexPop,
    LayerBoundary,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RectPrimitive {
    pub rect: Rect,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RoundedRectPrimitive {
    pub rect: Rect,
    /// Corner radii: [top-left, top-right, bottom-right, bottom-left].
    pub corner_radii: [f32; 4],
    pub color: Color,
}

impl RoundedRectPrimitive {
    pub fn uniform(rect: Rect, radius: f32, color: Color) -> Self {
        Self {
            rect,
            corner_radii: [radius; 4],
            color,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BorderPrimitive {
    pub rect: Rect,
    /// Border widths: [top, right, bottom, left].
    pub widths: [f32; 4],
    /// Corner radii: [top-left, top-right, bottom-right, bottom-left].
    pub corner_radii: [f32; 4],
    pub color: Color,
}

impl BorderPrimitive {
    pub fn uniform(rect: Rect, width: f32, radius: f32, color: Color) -> Self {
        Self {
            rect,
            widths: [width; 4],
            corner_radii: [radius; 4],
            color,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ShadowPrimitive {
    pub rect: Rect,
    pub blur_radius: f32,
    pub corner_radius: f32,
    /// Offset applied to shadow position: [x, y].
    pub offset: [f32; 2],
    pub color: Color,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextPrimitive {
    pub rect: Rect,
    pub text: String,
    pub color: Color,
    pub font_size: f32,
    pub font_kind: FontKind,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RichTextSpan {
    pub text: String,
    pub color: Color,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RichTextPrimitive {
    pub rect: Rect,
    pub spans: Vec<RichTextSpan>,
    pub default_color: Color,
    pub font_size: f32,
    pub font_kind: FontKind,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IconPrimitive {
    pub rect: Rect,
    pub name: String,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ClipPrimitive {
    pub rect: Rect,
}

// ---------------------------------------------------------------------------
// EffectQuad — procedural background (GPU-computed per-pixel)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BlurRegionPrimitive {
    pub rect: Rect,
    pub blur_radius: f32,
    pub corner_radius: f32,
}

/// Effect type for procedural background quads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum EffectType {
    /// Simplex noise blended between two colors.
    #[default]
    NoiseGradient = 0,
    /// Linear gradient with configurable angle.
    LinearGradient = 1,
    /// Radial gradient — color_a at center, color_b at edge.
    RadialGradient = 2,
    /// Animated shimmer — diagonal highlight sweep.
    Shimmer = 3,
    /// Vignette — edge darkening/coloring.
    Vignette = 4,
    /// Color tint — flat semi-transparent color overlay.
    ColorTint = 5,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EffectQuadPrimitive {
    pub rect: Rect,
    pub effect_type: EffectType,
    pub color_a: Color,
    pub color_b: Color,
    /// Effect-specific parameters: [param1, param2].
    /// - NoiseGradient: [scale, 0.0]
    /// - LinearGradient: [angle_radians, 0.0]
    pub params: [f32; 2],
    pub corner_radius: f32,
}
