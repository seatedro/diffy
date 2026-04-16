//! Style primitives — layout (Taffy) + visual (color, border, corner, shadow).
//!
//! Pure data. The fluent `Styled` trait built on top of these lives in diffy
//! because it depends on diffy's design-token system (`Sp`, `Rad`, `ShadowLayer`).

use crate::color::Color;

#[derive(Clone)]
pub struct ShadowStyle {
    pub blur_radius: f32,
    pub offset: [f32; 2],
    pub corner_radius: f32,
    pub color: Color,
}

#[derive(Clone)]
pub struct ElementStyle {
    pub layout: taffy::Style,
    pub background: Option<Color>,
    pub border_color: Option<Color>,
    pub border_widths: [f32; 4],
    pub corner_radius: f32,
    pub opacity: f32,
    pub z_index: i32,
    pub shadows: Vec<ShadowStyle>,
}

impl Default for ElementStyle {
    fn default() -> Self {
        Self {
            layout: taffy::Style {
                display: taffy::Display::Flex,
                ..Default::default()
            },
            background: None,
            border_color: None,
            border_widths: [0.0; 4],
            corner_radius: 0.0,
            opacity: 1.0,
            z_index: 0,
            shadows: Vec::new(),
        }
    }
}

#[derive(Clone, Default)]
pub struct StyleOverride {
    pub background: Option<Color>,
    pub border_color: Option<Color>,
    pub corner_radius: Option<f32>,
    pub opacity: Option<f32>,
    pub text_color: Option<Color>,
    pub icon_color: Option<Color>,
}

impl StyleOverride {
    pub fn bg(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub fn border_color(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self
    }

    pub fn rounded(mut self, r: f32) -> Self {
        self.corner_radius = Some(r);
        self
    }

    pub fn opacity(mut self, v: f32) -> Self {
        self.opacity = Some(v);
        self
    }

    pub fn text_color(mut self, color: Color) -> Self {
        self.text_color = Some(color);
        self
    }

    pub fn icon_color(mut self, color: Color) -> Self {
        self.icon_color = Some(color);
        self
    }
}

pub fn apply_override(base: &mut ElementStyle, ov: &StyleOverride) {
    if let Some(bg) = ov.background {
        base.background = Some(bg);
    }
    if let Some(bc) = ov.border_color {
        base.border_color = Some(bc);
    }
    if let Some(cr) = ov.corner_radius {
        base.corner_radius = cr;
    }
    if let Some(op) = ov.opacity {
        base.opacity = op;
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BackgroundEffect {
    NoiseGradient {
        scale: f32,
        color_a: Color,
        color_b: Color,
    },
    LinearGradient {
        angle: f32,
        color_a: Color,
        color_b: Color,
    },
    RadialGradient {
        color_a: Color,
        color_b: Color,
    },
    Shimmer {
        base: Color,
        highlight: Color,
        speed: f32,
    },
    Vignette {
        color: Color,
        intensity: f32,
    },
    ColorTint {
        color: Color,
    },
}

pub fn noise_gradient(scale: f32, color_a: Color, color_b: Color) -> BackgroundEffect {
    BackgroundEffect::NoiseGradient {
        scale,
        color_a,
        color_b,
    }
}

pub fn linear_gradient(angle: f32, color_a: Color, color_b: Color) -> BackgroundEffect {
    BackgroundEffect::LinearGradient {
        angle,
        color_a,
        color_b,
    }
}

pub fn radial_gradient(center: Color, edge: Color) -> BackgroundEffect {
    BackgroundEffect::RadialGradient {
        color_a: center,
        color_b: edge,
    }
}

pub fn shimmer(base: Color, highlight: Color, speed: f32) -> BackgroundEffect {
    BackgroundEffect::Shimmer {
        base,
        highlight,
        speed,
    }
}

pub fn vignette(color: Color, intensity: f32) -> BackgroundEffect {
    BackgroundEffect::Vignette { color, intensity }
}

pub fn color_tint(color: Color) -> BackgroundEffect {
    BackgroundEffect::ColorTint { color }
}
