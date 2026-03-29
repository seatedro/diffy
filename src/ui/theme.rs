use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
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
        Self {
            mode: ThemeMode::Dark,
            sans_family: default_sans_family(),
            mono_family: default_mono_family(),
            colors: ThemeColors {
                app_bg: hex("#171a20"),
                canvas: hex("#1b1f26"),
                panel: hex("#20242c"),
                panel_strong: hex("#262b34"),
                border_soft: hex("#37404c"),
                text_strong: hex("#f2f5f8"),
                accent: hex("#5da9f6"),
                selection_bg: hex("#30435d"),
                background: hex("#171a20"),
                surface: hex("#20242c"),
                editor_surface: hex("#1b1f26"),
                elevated_surface: hex("#262b34"),
                modal_surface: hex("#2a303a"),
                overlay_scrim: hex("#05070b99"),
                border: hex("#37404c"),
                border_variant: hex("#2d343e"),
                focus_border: hex("#5da9f6"),
                text: hex("#f2f5f8"),
                text_muted: hex("#a9b3bf"),
                text_accent: hex("#8bc3ff"),
                icon: hex("#bac4cf"),
                element_background: hex("#2a3039"),
                element_hover: hex("#313846"),
                element_active: hex("#384254"),
                element_selected: hex("#3d5f8a"),
                ghost_element_hover: hex("#ffffff10"),
                ghost_element_active: hex("#ffffff18"),
                ghost_element_selected: hex("#2f3b4f"),
                title_bar_background: hex("#1b2028"),
                status_bar_background: hex("#1b2028"),
                sidebar_background: hex("#1d2129"),
                sidebar_row_hover: hex("#252b35"),
                sidebar_row_selected: hex("#30435d"),
                empty_state_background: hex("#20252e"),
                empty_state_border: hex("#39414d"),
                scrollbar_thumb: hex("#4d5866"),
                status_info: hex("#61afef"),
                status_warning: hex("#e5c07b"),
                status_error: hex("#e06c75"),
                line_add: hex("#24342a"),
                line_del: hex("#382728"),
                line_modified: hex("#2b303d"),
                gutter_bg: hex("#1a1d23"),
                gutter_text: hex("#7f8893"),
                file_header_bg: hex("#262b34"),
                hunk_header_bg: hex("#242f3c"),
                line_add_text: hex("#a7e3b1"),
                line_del_text: hex("#f0b2b4"),
                hover_overlay: hex("#ffffff12"),
            },
            metrics: ThemeMetrics {
                title_bar_height: 40.0,
                status_bar_height: 28.0,
                sidebar_width: 320.0,
                panel_radius: 10.0,
                control_radius: 8.0,
                modal_radius: 14.0,
                spacing_xs: 6.0,
                spacing_sm: 10.0,
                spacing_md: 16.0,
                spacing_lg: 24.0,
                ui_font_size: 14.0,
                ui_small_font_size: 12.0,
                heading_font_size: 17.0,
                mono_font_size: 13.0,
            },
        }
    }

    pub fn default_light() -> Self {
        Self {
            mode: ThemeMode::Light,
            sans_family: default_sans_family(),
            mono_family: default_mono_family(),
            colors: ThemeColors {
                app_bg: hex("#f3f5f8"),
                canvas: hex("#fbfcfd"),
                panel: hex("#ffffff"),
                panel_strong: hex("#f2f5f8"),
                border_soft: hex("#c9d2dc"),
                text_strong: hex("#18212b"),
                accent: hex("#2d6dd2"),
                selection_bg: hex("#dce8f8"),
                background: hex("#f3f5f8"),
                surface: hex("#ffffff"),
                editor_surface: hex("#fbfcfd"),
                elevated_surface: hex("#ffffff"),
                modal_surface: hex("#ffffff"),
                overlay_scrim: hex("#0b152033"),
                border: hex("#c9d2dc"),
                border_variant: hex("#dde3ea"),
                focus_border: hex("#2d6dd2"),
                text: hex("#18212b"),
                text_muted: hex("#617183"),
                text_accent: hex("#1f5fba"),
                icon: hex("#5e6d7e"),
                element_background: hex("#f2f5f8"),
                element_hover: hex("#e6edf5"),
                element_active: hex("#dce6f2"),
                element_selected: hex("#d2e3fb"),
                ghost_element_hover: hex("#0000000a"),
                ghost_element_active: hex("#00000012"),
                ghost_element_selected: hex("#dbe8f8"),
                title_bar_background: hex("#eef2f6"),
                status_bar_background: hex("#eef2f6"),
                sidebar_background: hex("#f7f9fb"),
                sidebar_row_hover: hex("#edf2f7"),
                sidebar_row_selected: hex("#dce8f8"),
                empty_state_background: hex("#ffffff"),
                empty_state_border: hex("#d4dde7"),
                scrollbar_thumb: hex("#b1bcc9"),
                status_info: hex("#2d6dd2"),
                status_warning: hex("#b27700"),
                status_error: hex("#c14953"),
                line_add: hex("#e9f7ec"),
                line_del: hex("#fdecec"),
                line_modified: hex("#eef2fb"),
                gutter_bg: hex("#eef2f6"),
                gutter_text: hex("#7a8797"),
                file_header_bg: hex("#f2f5f8"),
                hunk_header_bg: hex("#eaf0f8"),
                line_add_text: hex("#1e7a34"),
                line_del_text: hex("#b3404b"),
                hover_overlay: hex("#0000000c"),
            },
            metrics: ThemeMetrics {
                title_bar_height: 40.0,
                status_bar_height: 28.0,
                sidebar_width: 320.0,
                panel_radius: 10.0,
                control_radius: 8.0,
                modal_radius: 14.0,
                spacing_xs: 6.0,
                spacing_sm: 10.0,
                spacing_md: 16.0,
                spacing_lg: 24.0,
                ui_font_size: 14.0,
                ui_small_font_size: 12.0,
                heading_font_size: 17.0,
                mono_font_size: 13.0,
            },
        }
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

fn hex(value: &str) -> Color {
    let hex = value.strip_prefix('#').unwrap_or(value);
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or_default();
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or_default();
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or_default();
            Color::rgba(r, g, b, 255)
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or_default();
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or_default();
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or_default();
            let a = u8::from_str_radix(&hex[6..8], 16).unwrap_or_default();
            Color::rgba(r, g, b, a)
        }
        _ => Color::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Color, Theme, ThemeMode};

    #[test]
    fn default_dark_theme_uses_expected_accent() {
        let theme = Theme::default_dark();
        assert_eq!(
            theme.colors.focus_border,
            Color::rgba(0x5d, 0xa9, 0xf6, 0xff)
        );
    }

    #[test]
    fn mode_factory_returns_light_theme() {
        let theme = Theme::for_mode(ThemeMode::Light);
        assert_eq!(theme.mode, ThemeMode::Light);
        assert_eq!(theme.colors.background, Color::rgba(0xf3, 0xf5, 0xf8, 0xff));
    }
}
