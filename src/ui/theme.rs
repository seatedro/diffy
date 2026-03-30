use crate::ui::palette::{self, Scale};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Self = Self { r: 0, g: 0, b: 0, a: 0 };

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self {
            r: self.r,
            g: self.g,
            b: self.b,
            a,
        }
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let inv = 1.0 - t;
        Self {
            r: (self.r as f32 * inv + other.r as f32 * t).round() as u8,
            g: (self.g as f32 * inv + other.g as f32 * t).round() as u8,
            b: (self.b as f32 * inv + other.b as f32 * t).round() as u8,
            a: (self.a as f32 * inv + other.a as f32 * t).round() as u8,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeColors {
    pub app_bg: Color,
    pub canvas: Color,
    pub panel: Color,
    pub panel_strong: Color,
    pub border_soft: Color,
    pub text_strong: Color,
    pub accent: Color,
    pub selection_bg: Color,
    pub background: Color,
    pub surface: Color,
    pub editor_surface: Color,
    pub elevated_surface: Color,
    pub modal_surface: Color,
    pub overlay_scrim: Color,
    pub border: Color,
    pub border_variant: Color,
    pub focus_border: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_accent: Color,
    pub icon: Color,
    pub element_background: Color,
    pub element_hover: Color,
    pub element_active: Color,
    pub element_selected: Color,
    pub ghost_element_hover: Color,
    pub ghost_element_active: Color,
    pub ghost_element_selected: Color,
    pub title_bar_background: Color,
    pub status_bar_background: Color,
    pub sidebar_background: Color,
    pub sidebar_row_hover: Color,
    pub sidebar_row_selected: Color,
    pub empty_state_background: Color,
    pub empty_state_border: Color,
    pub scrollbar_thumb: Color,
    pub status_info: Color,
    pub status_warning: Color,
    pub status_error: Color,
    pub line_add: Color,
    pub line_del: Color,
    pub line_modified: Color,
    pub gutter_bg: Color,
    pub gutter_text: Color,
    pub file_header_bg: Color,
    pub hunk_header_bg: Color,
    pub line_add_text: Color,
    pub line_del_text: Color,
    pub hover_overlay: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeMetrics {
    pub title_bar_height: f32,
    pub status_bar_height: f32,
    pub sidebar_width: f32,
    pub panel_radius: f32,
    pub control_radius: f32,
    pub modal_radius: f32,
    pub spacing_xs: f32,
    pub spacing_sm: f32,
    pub spacing_md: f32,
    pub spacing_lg: f32,
    pub ui_font_size: f32,
    pub ui_small_font_size: f32,
    pub heading_font_size: f32,
    pub mono_font_size: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub mode: ThemeMode,
    pub sans_family: &'static str,
    pub mono_family: &'static str,
    pub colors: ThemeColors,
    pub metrics: ThemeMetrics,
}

impl Theme {
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::default_dark(),
            ThemeMode::Light => Self::default_light(),
        }
    }

    pub fn toggle_mode(&mut self) {
        *self = Self::for_mode(match self.mode {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        });
    }

    pub fn default_dark() -> Self {
        let n = palette::dark_scale(palette::NEUTRAL_HUE, palette::NEUTRAL_CHROMA);
        let blue = palette::dark_scale(palette::BLUE_HUE, palette::BLUE_CHROMA);
        let red = palette::dark_scale(palette::RED_HUE, palette::RED_CHROMA);
        let green = palette::dark_scale(palette::GREEN_HUE, palette::GREEN_CHROMA);
        let yellow = palette::dark_scale(palette::YELLOW_HUE, palette::YELLOW_CHROMA);

        Self {
            mode: ThemeMode::Dark,
            sans_family: default_sans_family(),
            mono_family: default_mono_family(),
            colors: dark_colors(&n, &blue, &red, &green, &yellow),
            metrics: default_metrics(),
        }
    }

    pub fn default_light() -> Self {
        let n = palette::light_scale(palette::NEUTRAL_HUE, palette::NEUTRAL_CHROMA);
        let blue = palette::light_scale(palette::BLUE_HUE, palette::BLUE_CHROMA);
        let red = palette::light_scale(palette::RED_HUE, palette::RED_CHROMA);
        let green = palette::light_scale(palette::GREEN_HUE, palette::GREEN_CHROMA);
        let yellow = palette::light_scale(palette::YELLOW_HUE, palette::YELLOW_CHROMA);

        Self {
            mode: ThemeMode::Light,
            sans_family: default_sans_family(),
            mono_family: default_mono_family(),
            colors: light_colors(&n, &blue, &red, &green, &yellow),
            metrics: default_metrics(),
        }
    }
}

fn default_metrics() -> ThemeMetrics {
    ThemeMetrics {
        title_bar_height: 48.0,
        status_bar_height: 28.0,
        sidebar_width: 280.0,
        panel_radius: 12.0,
        control_radius: 8.0,
        modal_radius: 16.0,
        spacing_xs: 4.0,
        spacing_sm: 6.0,
        spacing_md: 12.0,
        spacing_lg: 16.0,
        ui_font_size: 16.0,
        ui_small_font_size: 14.0,
        heading_font_size: 20.0,
        mono_font_size: 15.0,
    }
}

/// Build dark-mode theme colors from perceptual scales.
///
/// Mapping convention — `n` is the 12-step neutral, `b` blue accent,
/// `r` red, `g` green, `y` yellow. Indices are 0-based (step 1 = [0]).
fn dark_colors(n: &Scale, b: &Scale, r: &Scale, g: &Scale, y: &Scale) -> ThemeColors {
    ThemeColors {
        // Backgrounds — clear visual hierarchy between layers.
        // n[0] is the deepest black, n[3] is noticeably lighter.
        app_bg:             n[0],
        canvas:             n[1],
        panel:              n[2],
        panel_strong:       n[3],
        background:         n[0],
        surface:            n[2],
        editor_surface:     n[1],
        elevated_surface:   n[3],
        modal_surface:      n[4],
        title_bar_background: n[2],     // slightly lifted from bg
        status_bar_background: n[1],
        sidebar_background: n[1],
        empty_state_background: n[2],
        gutter_bg:          n[0],
        file_header_bg:     n[3],
        hunk_header_bg:     b[3],

        // Interactive elements — clear affordance
        element_background: n[3],
        element_hover:      n[4],
        element_active:     n[5],
        element_selected:   b[5],

        // Ghost elements (semi-transparent overlays)
        ghost_element_hover:    Color::rgba(255, 255, 255, 20),
        ghost_element_active:   Color::rgba(255, 255, 255, 36),
        ghost_element_selected: b[4],
        hover_overlay:          Color::rgba(255, 255, 255, 14),

        sidebar_row_hover:    Color::rgba(255, 255, 255, 14),
        sidebar_row_selected: b[4],

        // Borders — subtle but visible separation
        border_soft:    n[3],
        border:         n[4],
        border_variant: n[3],
        focus_border:   b[8],
        empty_state_border: n[4],

        // Text — readable hierarchy
        text_strong: Color::rgba(240, 240, 245, 255),
        text:        n[10],
        text_muted:  n[8],
        text_accent: b[9],
        icon:        n[9],
        gutter_text: n[7],

        // Accent — vibrant blue
        accent:       b[8],
        selection_bg: b[4],

        // Overlay
        overlay_scrim: Color::rgba(0, 0, 0, 180),

        // Scrollbar
        scrollbar_thumb: Color::rgba(255, 255, 255, 100),

        // Status indicators
        status_info:    b[8],
        status_warning: y[8],
        status_error:   r[8],

        // Diff colors
        line_add:      g[2],
        line_del:      r[2],
        line_modified: b[2],
        line_add_text: g[8],
        line_del_text: r[8],
    }
}

/// Build light-mode theme colors from perceptual scales.
fn light_colors(n: &Scale, b: &Scale, r: &Scale, g: &Scale, y: &Scale) -> ThemeColors {
    ThemeColors {
        // Backgrounds (steps 1-4 — lightest first)
        app_bg:             n[0],
        canvas:             n[1],
        panel:              n[2],
        panel_strong:       n[3],
        background:         n[0],
        surface:            n[2],
        editor_surface:     n[1],
        elevated_surface:   n[2],
        modal_surface:      n[2],
        title_bar_background: n[3],
        status_bar_background: n[3],
        sidebar_background: n[1],
        empty_state_background: n[2],
        gutter_bg:          n[3],
        file_header_bg:     n[3],
        hunk_header_bg:     b[2],

        // Interactive elements
        element_background: n[3],
        element_hover:      n[4],
        element_active:     n[5],
        element_selected:   b[4],

        // Ghost elements (semi-transparent dark overlays)
        ghost_element_hover:    Color::rgba(0, 0, 0, 15),  // ~6%
        ghost_element_active:   Color::rgba(0, 0, 0, 31),  // ~12%
        ghost_element_selected: b[3],
        hover_overlay:          Color::rgba(0, 0, 0, 20),  // ~8%

        sidebar_row_hover:    n[4],
        sidebar_row_selected: b[3],

        // Borders
        border_soft:    n[6],
        border:         n[6],
        border_variant: n[5],
        focus_border:   b[8],
        empty_state_border: n[6],

        // Text (steps 10-12 — darkest)
        text_strong: n[11],
        text:        n[11],
        text_muted:  n[9],
        text_accent: b[9],
        icon:        n[9],
        gutter_text: n[8],

        // Accent
        accent:       b[8],
        selection_bg: b[3],

        // Overlay
        overlay_scrim: Color::rgba(11, 21, 32, 51),

        // Scrollbar
        scrollbar_thumb: n[7],

        // Status
        status_info:    b[8],
        status_warning: y[8],
        status_error:   r[8],

        // Diff
        line_add:      g[2],
        line_del:      r[2],
        line_modified: b[2],
        line_add_text: g[9],
        line_del_text: r[9],
    }
}

fn default_sans_family() -> &'static str {
    if cfg!(target_os = "windows") {
        "Segoe UI"
    } else if cfg!(target_os = "macos") {
        "Arial"
    } else {
        "DejaVu Sans"
    }
}

fn default_mono_family() -> &'static str {
    if cfg!(target_os = "windows") {
        "Consolas"
    } else if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "DejaVu Sans Mono"
    }
}

#[cfg(test)]
mod tests {
    use super::{Theme, ThemeMode};

    #[test]
    fn dark_focus_border_is_blue_accent() {
        let theme = Theme::default_dark();
        // focus_border comes from blue scale step 9 — should be a vivid blue.
        let c = theme.colors.focus_border;
        assert!(c.b > c.r && c.b > c.g, "focus_border should be distinctly blue");
        assert!(c.a == 255);
    }

    #[test]
    fn mode_factory_returns_light_theme() {
        let theme = Theme::for_mode(ThemeMode::Light);
        assert_eq!(theme.mode, ThemeMode::Light);
        // Light background should be very bright.
        let bg = theme.colors.background;
        assert!(bg.r > 230 && bg.g > 230 && bg.b > 230);
    }

    #[test]
    fn dark_neutral_steps_are_distinguishable() {
        let theme = Theme::default_dark();
        // Each surface tier should be brighter than the one below.
        let tiers = [
            theme.colors.background,
            theme.colors.editor_surface,
            theme.colors.surface,
            theme.colors.elevated_surface,
        ];
        for i in 1..tiers.len() {
            let prev = tiers[i - 1].r as u16 + tiers[i - 1].g as u16 + tiers[i - 1].b as u16;
            let curr = tiers[i].r as u16 + tiers[i].g as u16 + tiers[i].b as u16;
            assert!(
                curr >= prev,
                "surface tier {} should be >= tier {} ({} vs {})",
                i, i - 1, curr, prev,
            );
        }
    }
}
