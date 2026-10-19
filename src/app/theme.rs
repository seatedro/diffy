use quark_ui::theme::{
    Color, ComponentMetrics, ControlMetrics, SurfaceMetrics, Theme, ThemeColors, ThemeMode,
};

use std::sync::OnceLock;

use crate::app::tokens::{self, quark_units};
use crate::core::themes::{SemanticPalette, ThemeRegistry};
use crate::platform::persistence::Settings;

pub const DEFAULT_THEME: &str = "TokyoNight";

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
        Self::load(registry(), DEFAULT_THEME, 1.0)
    }
}

pub fn registry() -> &'static ThemeRegistry {
    static REGISTRY: OnceLock<ThemeRegistry> = OnceLock::new();
    REGISTRY.get_or_init(ThemeRegistry::load)
}

pub fn quark_mode(mode: crate::core::theme_types::ThemeMode) -> Option<ThemeMode> {
    match mode {
        crate::core::theme_types::ThemeMode::Dark => Some(ThemeMode::Dark),
        crate::core::theme_types::ThemeMode::Light => Some(ThemeMode::Light),
        crate::core::theme_types::ThemeMode::System => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThemeSelection {
    Fixed(Theme),
    System(ThemePair),
}

pub fn settings_theme(settings: &Settings) -> ThemeSelection {
    let scale = f32::from(settings.ui_scale_pct) / 100.0;
    match quark_mode(settings.theme_mode) {
        Some(mode) => {
            ThemeSelection::Fixed(theme_for(registry(), &settings.theme_name, mode, scale))
        }
        None => ThemeSelection::System(ThemePair::load(registry(), &settings.theme_name, scale)),
    }
}

pub fn preview_theme(settings: &Settings, mode: ThemeMode) -> Theme {
    theme_for(
        registry(),
        &settings.theme_name,
        mode,
        f32::from(settings.ui_scale_pct) / 100.0,
    )
}

pub fn theme_for(registry: &ThemeRegistry, name: &str, mode: ThemeMode, ui_scale: f32) -> Theme {
    let theme = match registry.get(name) {
        Some(entry) => from_palette(match mode {
            ThemeMode::Dark => &entry.dark,
            ThemeMode::Light => &entry.light,
        }),
        None => base_theme(mode),
    };
    theme.with_ui_scale(ui_scale)
}

pub fn from_palette(palette: &SemanticPalette) -> Theme {
    let mode = if palette.is_dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    let mut theme = base_theme(mode);
    theme.colors = colors(palette);
    theme
}

pub fn default_dark() -> Theme {
    theme_for(registry(), DEFAULT_THEME, ThemeMode::Dark, 1.0)
}

pub fn base_theme(mode: ThemeMode) -> Theme {
    let mut theme = Theme::for_mode(mode);
    theme.metrics = tokens::METRICS;
    theme.components = components();
    theme
}

fn control(height: f32, padding_x: f32, font_size: f32) -> ControlMetrics {
    ControlMetrics {
        height: Some(quark_units(height)),
        padding_x: Some(quark_units(padding_x)),
        font_size: Some(quark_units(font_size)),
        ..ControlMetrics::default()
    }
}

fn components() -> ComponentMetrics {
    ComponentMetrics {
        button: ControlMetrics {
            radius: Some(quark_units(7.0)),
            gap: Some(quark_units(6.0)),
            icon_size: Some(quark_units(14.0)),
            ..control(26.0, 10.0, 12.5)
        },
        button_compact: ControlMetrics {
            radius: Some(quark_units(5.0)),
            gap: Some(quark_units(4.0)),
            icon_size: Some(quark_units(13.0)),
            ..control(22.0, 7.0, 12.0)
        },
        select: control(28.0, 10.0, 12.5),
        option: control(28.0, 8.0, 12.5),
        popover: SurfaceMetrics {
            radius: Some(quark_units(9.0)),
            padding_x: Some(quark_units(4.0)),
            padding_y: Some(quark_units(4.0)),
            font_size: Some(quark_units(12.5)),
            ..SurfaceMetrics::default()
        },
        tooltip: SurfaceMetrics {
            font_size: Some(quark_units(11.5)),
            ..SurfaceMetrics::default()
        },
        toast: SurfaceMetrics {
            font_size: Some(quark_units(12.5)),
            ..SurfaceMetrics::default()
        },
        modal: SurfaceMetrics {
            radius: Some(quark_units(12.0)),
            ..SurfaceMetrics::default()
        },
        ..ComponentMetrics::default()
    }
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
        control_border: rgb!(l.border_variant).lerp(text_muted, CONTROL_BORDER_TINT),
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

const CONTROL_BORDER_TINT: f32 = 0.35;

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
        let base = base_theme(ThemeMode::Dark);
        assert_eq!(
            theme.metrics.title_bar_height,
            base.metrics.title_bar_height * 1.5
        );
    }
}
