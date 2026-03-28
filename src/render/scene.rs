use crate::ui::theme::Color;

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

    pub fn text(&mut self, text: TextPrimitive) {
        self.push(Primitive::TextRun(text));
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
    Icon(IconPrimitive),
    ClipStart(ClipPrimitive),
    ClipEnd,
    LayerBoundary,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RectPrimitive {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RoundedRectPrimitive {
    pub rect: RectPrimitive,
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BorderPrimitive {
    pub rect: RectPrimitive,
    pub width: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ShadowPrimitive {
    pub rect: RectPrimitive,
    pub blur_radius: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextPrimitive {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub color: Color,
    pub font_size: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IconPrimitive {
    pub x: f32,
    pub y: f32,
    pub name: String,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ClipPrimitive {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
