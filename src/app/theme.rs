use quark_ui::theme::{Color, Theme, ThemeColors, ThemeMode};

use crate::core::themes::{SemanticPalette, ThemeRegistry};

pub const DEFAULT_THEME: &str = "diffy-default";

#[derive(Debug, Clone, PartialEq)]
pub struct ThemePair {
    pub light: Theme,
    pub dark: Theme,
}

impl ThemePair {
    pub fn load(registry: &ThemeRegistry, name: &str, ui_scale: f32) -> Self {
        Self {
            light: theme_for(registry, name, ThemeMode::Light, ui_scale),
            dark: theme_for(registry, name, ThemeMode::Dark, ui_scale),
        }
    }

    pub fn get(&self, mode: ThemeMode) -> &Theme {
        match mode {
            ThemeMode::Light => &self.light,
            ThemeMode::Dark => &self.dark,
        }
    }
}

impl Default for ThemePair {
    fn default() -> Self {
        Self {
            light: Theme::default_light(),
            dark: Theme::default_dark(),
        }
    }
}

pub fn theme_for(registry: &ThemeRegistry, name: &str, mode: ThemeMode, ui_scale: f32) -> Theme {
    let theme = match registry.get(name) {
        Some(entry) => from_palette(match mode {
            ThemeMode::Dark => &entry.dark,
            ThemeMode::Light => &entry.light,
        }),
        None => Theme::for_mode(mode),
    };
    theme.with_ui_scale(ui_scale)
}

pub fn from_palette(palette: &SemanticPalette) -> Theme {
    let mode = if palette.is_dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    let mut theme = Theme::for_mode(mode);
    theme.colors = colors(palette);
    theme
}

macro_rules! rgb {
    ($color:expr) => {{
        let c = $color;
        Color::rgba(c.r, c.g, c.b, c.a)
    }};
}

fn colors(palette: &SemanticPalette) -> ThemeColors {
    let l = palette.to_theme_colors();
    let panel = rgb!(l.panel);
    let sidebar = rgb!(l.sidebar_background);
    let editor = rgb!(l.editor_surface);
    let text_muted = rgb!(l.text_muted);
    let accent = rgb!(l.accent);
    ThemeColors {
        app_bg: rgb!(l.app_bg),
        canvas: rgb!(l.canvas),
        panel,
        panel_strong: rgb!(l.panel_strong),
        border_soft: rgb!(l.border_soft),
        text_strong: rgb!(l.text_strong),
        accent,
        accent_strong: rgb!(l.accent_strong),
        on_accent: on_accent(accent),
        selection_bg: rgb!(l.selection_bg),
        background: rgb!(l.background),
        surface: rgb!(l.surface),
        editor_surface: editor,
        elevated_surface: rgb!(l.elevated_surface),
        modal_surface: rgb!(l.modal_surface),
        overlay_scrim: rgb!(l.overlay_scrim),
        border: rgb!(l.border),
        control_border: text_muted,
        border_variant: rgb!(l.border_variant),
        focus_border: rgb!(l.focus_border),
        text: rgb!(l.text),
        text_muted,
        placeholder: text_muted,
        text_disabled: text_muted,
        text_accent: rgb!(l.text_accent),
        icon: rgb!(l.icon),
        element_background: rgb!(l.element_background),
        element_hover: rgb!(l.element_hover),
        element_active: rgb!(l.element_active),
        element_selected: flatten(rgb!(l.element_selected), panel),
        ghost_element_hover: flatten(rgb!(l.ghost_element_hover), panel),
        ghost_element_active: flatten(rgb!(l.ghost_element_active), panel),
        ghost_element_selected: flatten(rgb!(l.ghost_element_selected), panel),
        title_bar_background: rgb!(l.title_bar_background),
        status_bar_background: rgb!(l.status_bar_background),
        sidebar_background: sidebar,
        sidebar_row_hover: flatten(rgb!(l.sidebar_row_hover), sidebar),
        sidebar_row_selected: flatten(rgb!(l.sidebar_row_selected), sidebar),
        empty_state_background: rgb!(l.empty_state_background),
        empty_state_border: rgb!(l.empty_state_border),
        scrollbar_thumb: flatten(rgb!(l.scrollbar_thumb), editor),
        status_info: rgb!(l.status_info),
        status_warning: rgb!(l.status_warning),
        status_error: rgb!(l.status_error),
        line_add: rgb!(l.line_add),
        line_del: rgb!(l.line_del),
        line_modified: rgb!(l.line_modified),
        gutter_bg: rgb!(l.gutter_bg),
        gutter_text: rgb!(l.gutter_text),
        file_header_bg: rgb!(l.file_header_bg),
        hunk_header_bg: rgb!(l.hunk_header_bg),
        line_add_text: rgb!(l.line_add_text),
        line_del_text: rgb!(l.line_del_text),
        line_add_word_bg: rgb!(l.line_add_word_bg),
        line_del_word_bg: rgb!(l.line_del_word_bg),
        hover_overlay: flatten(rgb!(l.hover_overlay), panel),
        search_match_bg: flatten(rgb!(l.search_match_bg), editor),
        search_match_active_bg: flatten(rgb!(l.search_match_active_bg), editor),
        syntax_keyword: rgb!(l.syntax_keyword),
        syntax_string: rgb!(l.syntax_string),
        syntax_comment: rgb!(l.syntax_comment),
        syntax_function: rgb!(l.syntax_function),
        syntax_type: rgb!(l.syntax_type),
        syntax_number: rgb!(l.syntax_number),
        syntax_property: rgb!(l.syntax_property),
        syntax_operator: rgb!(l.syntax_operator),
    }
}

fn flatten(tint: Color, base: Color) -> Color {
    base.lerp(tint.with_alpha(255), f32::from(tint.a) / 255.0)
        .with_alpha(255)
}

fn on_accent(accent: Color) -> Color {
    let white = Color::rgba(255, 255, 255, 255);
    let black = Color::rgba(0, 0, 0, 255);
    if contrast(white, accent) >= contrast(black, accent) {
        white
    } else {
        black
    }
}

fn contrast(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn luminance(c: Color) -> f32 {
    let channel = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque(colors: &ThemeColors) -> Vec<&'static str> {
        ThemeColors::TOKENS
            .iter()
            .copied()
            .filter(|name| *name != "overlay_scrim")
            .filter(|name| colors.get(name).is_some_and(|c| c.a != 255))
            .collect()
    }

    #[test]
    fn default_theme_uses_the_builtin_palette() {
        let registry = ThemeRegistry::load();
        let pair = ThemePair::load(&registry, DEFAULT_THEME, 1.0);
        assert_eq!(pair, ThemePair::default());
    }

    #[test]
    fn catalog_theme_keeps_semantic_diff_and_syntax_colors() {
        let registry = ThemeRegistry::load();
        let name = registry
            .names()
            .find(|name| {
                registry
                    .get(name)
                    .is_some_and(|entry| entry.dark.is_dark && !entry.light.is_dark)
            })
            .map(str::to_owned)
            .expect("a dual theme");
        let entry = registry.get(&name).expect("entry");
        let pair = ThemePair::load(&registry, &name, 1.0);

        assert_eq!(pair.dark.mode, ThemeMode::Dark);
        assert_eq!(pair.light.mode, ThemeMode::Light);
        for (theme, palette) in [(&pair.dark, &entry.dark), (&pair.light, &entry.light)] {
            let c = &theme.colors;
            assert_eq!(c.app_bg, rgb!(palette.app_bg));
            assert_eq!(c.line_add, rgb!(palette.line_add));
            assert_eq!(c.line_del_word_bg, rgb!(palette.line_del_accent));
            assert_eq!(c.syntax_keyword, rgb!(palette.syn_keyword));
            assert_eq!(c.syntax_comment, rgb!(palette.syn_comment));
            assert!(opaque(c).is_empty(), "translucent: {:?}", opaque(c));
            assert!(contrast(c.on_accent, c.accent) >= 3.0);
        }
    }

    #[test]
    fn ui_scale_scales_metrics() {
        let registry = ThemeRegistry::load();
        let theme = theme_for(&registry, DEFAULT_THEME, ThemeMode::Dark, 1.5);
        let base = Theme::default_dark();
        assert_eq!(
            theme.metrics.title_bar_height,
            base.metrics.title_bar_height * 1.5
        );
    }
}
