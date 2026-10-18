use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const BLACK: Self = Self::new(0, 0, 0, 255);
    pub const WHITE: Self = Self::new(255, 255, 255, 255);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let inv = 1.0 - t;
        let mix = |from: u8, to: u8| (f32::from(from) * inv + f32::from(to) * t).round() as u8;
        Self {
            r: mix(self.r, other.r),
            g: mix(self.g, other.g),
            b: mix(self.b, other.b),
            a: mix(self.a, other.a),
        }
    }

    pub fn relative_luminance(self) -> f32 {
        fn linearize(v: u8) -> f32 {
            let s = f32::from(v) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * linearize(self.r) + 0.7152 * linearize(self.g) + 0.0722 * linearize(self.b)
    }

    pub fn contrast_ratio(self, other: Self) -> f32 {
        let l1 = self.relative_luminance();
        let l2 = other.relative_luminance();
        let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (lighter + 0.05) / (darker + 0.05)
    }

    pub fn ensure_contrast(self, bg: Self, min_ratio: f32, is_dark: bool) -> Self {
        if self.contrast_ratio(bg) >= min_ratio {
            return self;
        }
        let target = if is_dark { Self::WHITE } else { Self::BLACK };
        let mut lo = 0.0_f32;
        let mut hi = 1.0_f32;
        for _ in 0..12 {
            let mid = (lo + hi) * 0.5;
            if self.lerp(target, mid).contrast_ratio(bg) >= min_ratio {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        self.lerp(target, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeMode;

    #[test]
    fn theme_mode_serialized_names_are_stable() {
        assert_eq!(serde_json::to_string(&ThemeMode::Dark).unwrap(), "\"dark\"");
        assert_eq!(
            serde_json::from_str::<ThemeMode>("\"light\"").unwrap(),
            ThemeMode::Light
        );
    }
}
