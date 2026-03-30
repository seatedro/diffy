//! Perceptual 12-step color scale generation in Oklch.
//!
//! Each scale provides 12 steps with defined semantic roles:
//!   Steps 1-2:  App backgrounds (minimal contrast)
//!   Steps 3-5:  Component backgrounds (interactive states)
//!   Steps 6-8:  Borders (subtle → prominent)
//!   Step 9:     Saturated semantic indicator
//!   Steps 10-12: Text and icons (increasing contrast)

use crate::ui::theme::Color;

/// A 12-step color scale (indices 0..12 map to steps 1..12).
pub type Scale = [Color; 12];

/// A 12-step alpha scale — same hue but with varying alpha.
pub type AlphaScale = [Color; 12];

// ---------------------------------------------------------------------------
// Oklch → sRGB conversion
// ---------------------------------------------------------------------------

/// Convert Oklch (L in 0..1, C >= 0, H in degrees) to sRGB Color.
fn oklch_to_color(l: f32, c: f32, h_deg: f32) -> Color {
    let h = h_deg.to_radians();
    let a = c * h.cos();
    let b = c * h.sin();
    oklab_to_color(l, a, b)
}

fn oklab_to_color(l: f32, a: f32, b: f32) -> Color {
    // Oklab → linear sRGB via LMS intermediate.
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;

    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;

    let r_lin = 4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3;
    let g_lin = -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3;
    let b_lin = -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3;

    Color::rgba(
        linear_to_srgb_u8(r_lin),
        linear_to_srgb_u8(g_lin),
        linear_to_srgb_u8(b_lin),
        255,
    )
}

fn linear_to_srgb_u8(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}

// ---------------------------------------------------------------------------
// Scale generation
// ---------------------------------------------------------------------------

/// Generate a 12-step dark-mode scale for the given hue (degrees) and peak
/// chroma. Lightness runs from very dark (step 1) to very light (step 12).
pub fn dark_scale(hue: f32, peak_chroma: f32) -> Scale {
    // Lightness curve: starts very dark, ramps through mid-tones, ends bright.
    // Tuned so that steps 1-2 are near-black backgrounds, steps 10-12 are
    // readable text, and step 9 is the most saturated.
    const L: [f32; 12] = [
        0.17, // 1: deepest background   (~#111316)
        0.20, // 2: canvas/editor        (~#16191e)
        0.23, // 3: surface/panel        (~#1c1f26)
        0.26, // 4: elevated surface     (~#22262f)
        0.29, // 5: element background   (~#252a33)
        0.32, // 6: element hover        (~#2e3440)
        0.37, // 7: border               (~#353d4c)
        0.44, // 8: strong border
        0.58, // 9: saturated indicator
        0.65, // 10: muted text          (~#8892a2)
        0.86, // 11: body text           (~#d8dee9)
        0.95, // 12: strong text         (~#eceff4)
    ];

    // Chroma curve: low for backgrounds, peaks at step 9, moderate for text.
    const C_FACTOR: [f32; 12] = [
        0.25, 0.30, 0.35, 0.38, 0.40, 0.42, 0.45, 0.50, 1.00, // peak
        0.55, 0.30, 0.12,
    ];

    let mut scale = [Color::default(); 12];
    for i in 0..12 {
        scale[i] = oklch_to_color(L[i], peak_chroma * C_FACTOR[i], hue);
    }
    scale
}

/// Generate a 12-step light-mode scale.
pub fn light_scale(hue: f32, peak_chroma: f32) -> Scale {
    // Inverted lightness: step 1 is lightest background, step 12 is darkest.
    const L: [f32; 12] = [
        0.97, // 1: lightest background
        0.95, // 2: canvas
        0.92, // 3: panel
        0.88, // 4: element bg
        0.84, // 5: element hover
        0.78, // 6: subtle border
        0.70, // 7: border
        0.60, // 8: strong border
        0.55, // 9: saturated indicator
        0.45, // 10: muted text
        0.30, // 11: body text
        0.18, // 12: strong text
    ];

    const C_FACTOR: [f32; 12] = [
        0.10, 0.12, 0.15, 0.18, 0.22, 0.28, 0.35, 0.45, 1.00, 0.60, 0.40, 0.20,
    ];

    let mut scale = [Color::default(); 12];
    for i in 0..12 {
        scale[i] = oklch_to_color(L[i], peak_chroma * C_FACTOR[i], hue);
    }
    scale
}

/// Generate an alpha scale from a base color, with alpha ranging from subtle
/// to opaque across 12 steps.
pub fn alpha_scale(base: Color) -> AlphaScale {
    const ALPHA: [u8; 12] = [5, 10, 15, 22, 30, 40, 55, 75, 100, 140, 190, 240];
    let mut scale = [Color::default(); 12];
    for i in 0..12 {
        scale[i] = Color::rgba(base.r, base.g, base.b, ALPHA[i]);
    }
    scale
}

// ---------------------------------------------------------------------------
// Predefined palette hues and chromas
// ---------------------------------------------------------------------------

/// Neutral blue-grey (the existing diffy palette lean, ~255°).
pub const NEUTRAL_HUE: f32 = 255.0;
pub const NEUTRAL_CHROMA: f32 = 0.035;

/// Blue accent.
pub const BLUE_HUE: f32 = 255.0;
pub const BLUE_CHROMA: f32 = 0.14;

/// Red (errors, deletions).
pub const RED_HUE: f32 = 25.0;
pub const RED_CHROMA: f32 = 0.16;

/// Green (success, additions).
pub const GREEN_HUE: f32 = 145.0;
pub const GREEN_CHROMA: f32 = 0.14;

/// Yellow/gold (warnings).
pub const YELLOW_HUE: f32 = 85.0;
pub const YELLOW_CHROMA: f32 = 0.14;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_neutral_scale_is_monotonically_brighter() {
        let scale = dark_scale(NEUTRAL_HUE, NEUTRAL_CHROMA);
        // Rough check: each step should have higher perceived brightness.
        // We use the sum of RGB as a proxy.
        for i in 1..12 {
            let prev_sum = scale[i - 1].r as u16 + scale[i - 1].g as u16 + scale[i - 1].b as u16;
            let curr_sum = scale[i].r as u16 + scale[i].g as u16 + scale[i].b as u16;
            assert!(
                curr_sum >= prev_sum,
                "step {} (sum={}) should be >= step {} (sum={})",
                i + 1,
                curr_sum,
                i,
                prev_sum
            );
        }
    }

    #[test]
    fn light_neutral_scale_is_monotonically_darker() {
        let scale = light_scale(NEUTRAL_HUE, NEUTRAL_CHROMA);
        for i in 1..12 {
            let prev_sum = scale[i - 1].r as u16 + scale[i - 1].g as u16 + scale[i - 1].b as u16;
            let curr_sum = scale[i].r as u16 + scale[i].g as u16 + scale[i].b as u16;
            assert!(
                curr_sum <= prev_sum,
                "step {} (sum={}) should be <= step {} (sum={})",
                i + 1,
                curr_sum,
                i,
                prev_sum
            );
        }
    }

    #[test]
    fn alpha_scale_increases() {
        let base = Color::rgba(255, 255, 255, 255);
        let scale = alpha_scale(base);
        for i in 1..12 {
            assert!(scale[i].a > scale[i - 1].a);
        }
    }
}
