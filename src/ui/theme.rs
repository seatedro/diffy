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
    pub text_muted: Color,
    pub accent: Color,
    pub selection_bg: Color,
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
    pub scrollbar_thumb: Color,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub mode: ThemeMode,
    pub sans_family: &'static str,
    pub mono_family: &'static str,
    pub colors: ThemeColors,
}

impl Theme {
    pub fn default_dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            sans_family: default_sans_family(),
            mono_family: default_mono_family(),
            colors: ThemeColors {
                app_bg: hex("#1b1e24"),
                canvas: hex("#20242b"),
                panel: hex("#282d36"),
                panel_strong: hex("#323844"),
                border_soft: hex("#39414d"),
                text_strong: hex("#f2f5f8"),
                text_muted: hex("#a9b3bf"),
                accent: hex("#5da9f6"),
                selection_bg: hex("#2f3b4f"),
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
                scrollbar_thumb: hex("#4d5866"),
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
    use super::{Color, Theme};

    #[test]
    fn default_dark_theme_uses_expected_accent() {
        let theme = Theme::default_dark();
        assert_eq!(theme.colors.accent, Color::rgba(0x5d, 0xa9, 0xf6, 0xff));
    }
}
