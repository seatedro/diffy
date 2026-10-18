use crate::core::theme_types::Rgba;
use crate::core::themes::SemanticPalette;
use crate::ui::palette::{self, Scale, Step};

pub use halogen::Color;

pub use crate::core::theme_types::ThemeMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeColors {
    pub app_bg: Color,
    pub canvas: Color,
    pub panel: Color,
    pub panel_strong: Color,
    pub border_soft: Color,
    pub text_strong: Color,
    pub accent: Color,
    pub accent_strong: Color,
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
    pub line_add_word_bg: Color,
    pub line_del_word_bg: Color,
    pub hover_overlay: Color,
    pub search_match_bg: Color,
    pub search_match_active_bg: Color,
    pub syntax_keyword: Color,
    pub syntax_string: Color,
    pub syntax_comment: Color,
    pub syntax_function: Color,
    pub syntax_type: Color,
    pub syntax_number: Color,
    pub syntax_property: Color,
    pub syntax_operator: Color,
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
    pub ui_row_height: f32,
    pub code_row_height: f32,
}

impl ThemeMetrics {
    pub fn ui_scale(&self) -> f32 {
        (self.ui_font_size / 16.0).max(0.7)
    }

    pub fn scaled(self, scale: f32) -> Self {
        let scale = scale.clamp(0.5, 4.0);
        Self {
            title_bar_height: self.title_bar_height * scale,
            status_bar_height: self.status_bar_height * scale,
            sidebar_width: self.sidebar_width * scale,
            panel_radius: self.panel_radius * scale,
            control_radius: self.control_radius * scale,
            modal_radius: self.modal_radius * scale,
            spacing_xs: self.spacing_xs * scale,
            spacing_sm: self.spacing_sm * scale,
            spacing_md: self.spacing_md * scale,
            spacing_lg: self.spacing_lg * scale,
            ui_font_size: self.ui_font_size * scale,
            ui_small_font_size: self.ui_small_font_size * scale,
            heading_font_size: self.heading_font_size * scale,
            mono_font_size: self.mono_font_size * scale,
            ui_row_height: self.ui_row_height * scale,
            code_row_height: self.code_row_height * scale,
        }
    }
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

    pub fn from_registry(
        name: &str,
        mode: ThemeMode,
        registry: &crate::core::themes::ThemeRegistry,
    ) -> Self {
        let entry = match registry.get(name) {
            Some(e) => e,
            None => return Self::for_mode(mode),
        };
        let palette = match mode {
            ThemeMode::Dark => &entry.dark,
            ThemeMode::Light => &entry.light,
        };
        Self {
            mode,
            sans_family: default_sans_family(),
            mono_family: default_mono_family(),
            colors: palette.to_theme_colors(),
            metrics: default_metrics(),
        }
    }

    pub fn with_ui_scale(mut self, scale: f32) -> Self {
        self.metrics = self.metrics.scaled(scale);
        self
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
        let purple = palette::dark_scale(palette::PURPLE_HUE, palette::PURPLE_CHROMA);
        let teal = palette::dark_scale(palette::TEAL_HUE, palette::TEAL_CHROMA);
        let orange = palette::dark_scale(palette::ORANGE_HUE, palette::ORANGE_CHROMA);

        Self {
            mode: ThemeMode::Dark,
            sans_family: default_sans_family(),
            mono_family: default_mono_family(),
            colors: dark_colors(&n, &blue, &red, &green, &yellow, &purple, &teal, &orange),
            metrics: default_metrics(),
        }
    }

    pub fn default_light() -> Self {
        let n = palette::light_scale(palette::NEUTRAL_HUE, palette::NEUTRAL_CHROMA);
        let blue = palette::light_scale(palette::BLUE_HUE, palette::BLUE_CHROMA);
        let red = palette::light_scale(palette::RED_HUE, palette::RED_CHROMA);
        let green = palette::light_scale(palette::GREEN_HUE, palette::GREEN_CHROMA);
        let yellow = palette::light_scale(palette::YELLOW_HUE, palette::YELLOW_CHROMA);
        let purple = palette::light_scale(palette::PURPLE_HUE, palette::PURPLE_CHROMA);
        let teal = palette::light_scale(palette::TEAL_HUE, palette::TEAL_CHROMA);
        let orange = palette::light_scale(palette::ORANGE_HUE, palette::ORANGE_CHROMA);

        Self {
            mode: ThemeMode::Light,
            sans_family: default_sans_family(),
            mono_family: default_mono_family(),
            colors: light_colors(&n, &blue, &red, &green, &yellow, &purple, &teal, &orange),
            metrics: default_metrics(),
        }
    }
}

fn default_metrics() -> ThemeMetrics {
    ThemeMetrics {
        title_bar_height: 44.0,
        status_bar_height: 40.0,
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
        ui_row_height: 40.0,
        code_row_height: 24.0,
    }
}

/// Build dark-mode theme colors from perceptual scales.
///
/// Mapping convention — `n` is the 12-step neutral, `b` blue accent,
/// `r` red, `g` green, `y` yellow. Indexed via `Step` enum.
fn dark_colors(
    n: &Scale,
    b: &Scale,
    r: &Scale,
    g: &Scale,
    y: &Scale,
    purple: &Scale,
    teal: &Scale,
    orange: &Scale,
) -> ThemeColors {
    use Step::*;

    ThemeColors {
        // Backgrounds — clear visual hierarchy between layers.
        app_bg: n[Bg],
        canvas: n[BgAlt],
        panel: n[Element],
        panel_strong: n[ElementHover],
        background: n[Bg],
        surface: n[Element],
        editor_surface: n[BgAlt],
        elevated_surface: n[ElementHover],
        modal_surface: n[ElementActive],
        title_bar_background: n[Element],
        status_bar_background: n[BgAlt],
        sidebar_background: n[BgAlt],
        empty_state_background: n[Element],
        gutter_bg: n[Bg],
        file_header_bg: n[ElementHover],
        hunk_header_bg: b[ElementHover],

        // Interactive elements — clear affordance
        element_background: n[ElementHover],
        element_hover: n[ElementActive],
        element_active: n[BorderSubtle],
        element_selected: b[BorderSubtle],

        // Ghost elements — accent-tinted so they feel integrated with the theme
        ghost_element_hover: b[ElementHover],
        ghost_element_active: b[ElementActive],
        ghost_element_selected: b[ElementActive],
        hover_overlay: b[ElementHover],

        sidebar_row_hover: b[ElementHover],
        sidebar_row_selected: b[ElementActive],

        // Borders — subtle but visible separation
        border_soft: n[ElementHover],
        border: n[ElementActive],
        border_variant: n[ElementHover],
        focus_border: b[Solid],
        empty_state_border: n[ElementActive],

        // Text — readable hierarchy
        text_strong: Color::rgba(240, 240, 245, 255),
        text: n[Text],
        text_muted: n[Solid],
        text_accent: b[TextSubtle],
        icon: n[TextSubtle],
        gutter_text: n[BorderStrong],

        // Accent — vibrant blue
        accent: b[Solid],
        accent_strong: b[TextSubtle],
        selection_bg: b[ElementActive],

        // Overlay
        overlay_scrim: Color::rgba(0, 0, 0, 180),

        // Scrollbar
        scrollbar_thumb: Color::rgba(255, 255, 255, 100),

        // Status indicators
        status_info: b[Solid],
        status_warning: y[Solid],
        status_error: r[Solid],

        // Diff colors
        line_add: g[Element],
        line_del: r[Element],
        line_modified: b[Element],
        line_add_text: g[Solid],
        line_del_text: r[Solid],
        line_add_word_bg: g[ElementActive],
        line_del_word_bg: r[ElementActive],

        search_match_bg: Color::rgba(y[Border].r, y[Border].g, y[Border].b, 90),
        search_match_active_bg: Color::rgba(y[Solid].r, y[Solid].g, y[Solid].b, 180),

        syntax_keyword: purple[TextSubtle],
        syntax_string: g[TextSubtle],
        syntax_comment: n[Solid],
        syntax_function: b[TextSubtle],
        syntax_type: teal[TextSubtle],
        syntax_number: orange[TextSubtle],
        syntax_property: r[TextSubtle],
        syntax_operator: n[TextSubtle],
    }
}

/// Build light-mode theme colors from perceptual scales.
fn light_colors(
    n: &Scale,
    b: &Scale,
    r: &Scale,
    g: &Scale,
    y: &Scale,
    purple: &Scale,
    teal: &Scale,
    orange: &Scale,
) -> ThemeColors {
    use Step::*;

    ThemeColors {
        // Backgrounds (lightest first in light mode)
        app_bg: n[Bg],
        canvas: n[BgAlt],
        panel: n[Element],
        panel_strong: n[ElementHover],
        background: n[Bg],
        surface: n[Element],
        editor_surface: n[BgAlt],
        elevated_surface: n[Element],
        modal_surface: n[Element],
        title_bar_background: n[ElementHover],
        status_bar_background: n[ElementHover],
        sidebar_background: n[BgAlt],
        empty_state_background: n[Element],
        gutter_bg: n[ElementHover],
        file_header_bg: n[ElementHover],
        hunk_header_bg: b[Element],

        // Interactive elements
        element_background: n[ElementHover],
        element_hover: n[ElementActive],
        element_active: n[BorderSubtle],
        element_selected: b[ElementActive],

        // Ghost elements — accent-tinted so they feel integrated with the theme
        ghost_element_hover: b[Element],
        ghost_element_active: b[ElementHover],
        ghost_element_selected: b[ElementHover],
        hover_overlay: b[Element],

        sidebar_row_hover: b[Element],
        sidebar_row_selected: b[ElementHover],

        // Borders
        border_soft: n[Border],
        border: n[Border],
        border_variant: n[BorderSubtle],
        focus_border: b[Solid],
        empty_state_border: n[Border],

        // Text (darkest in light mode)
        text_strong: n[TextStrong],
        text: n[TextStrong],
        text_muted: n[TextSubtle],
        text_accent: b[TextSubtle],
        icon: n[TextSubtle],
        gutter_text: n[Solid],

        // Accent
        accent: b[Solid],
        accent_strong: b[TextSubtle],
        selection_bg: b[ElementHover],

        // Overlay
        overlay_scrim: Color::rgba(11, 21, 32, 51),

        // Scrollbar
        scrollbar_thumb: n[BorderStrong],

        // Status
        status_info: b[Solid],
        status_warning: y[Solid],
        status_error: r[Solid],

        // Diff
        line_add: g[Element],
        line_del: r[Element],
        line_modified: b[Element],
        line_add_text: g[TextSubtle],
        line_del_text: r[TextSubtle],
        line_add_word_bg: g[ElementActive],
        line_del_word_bg: r[ElementActive],

        search_match_bg: Color::rgba(
            y[ElementActive].r,
            y[ElementActive].g,
            y[ElementActive].b,
            120,
        ),
        search_match_active_bg: Color::rgba(y[Border].r, y[Border].g, y[Border].b, 200),

        syntax_keyword: purple[TextSubtle],
        syntax_string: g[TextSubtle],
        syntax_comment: n[Solid],
        syntax_function: b[TextSubtle],
        syntax_type: teal[TextSubtle],
        syntax_number: orange[TextSubtle],
        syntax_property: r[TextSubtle],
        syntax_operator: n[TextSubtle],
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

impl From<Rgba> for Color {
    fn from(value: Rgba) -> Self {
        Color::rgba(value.r, value.g, value.b, value.a)
    }
}

impl SemanticPalette {
    pub fn to_theme_colors(&self) -> ThemeColors {
        let s = self;
        let d = s.is_dark;

        let text = s.text_base.ensure_contrast(s.panel, 4.5, d);
        let text_strong = s.text_strong.ensure_contrast(s.panel_strong, 4.5, d);
        let text_muted = s.text_muted.ensure_contrast(s.panel, 3.0, d);
        let icon = s.text_base.ensure_contrast(s.panel, 4.5, d);

        // element_* are opaque tiers above panel (Radix step 3→4→5 pattern).
        // Ghost_* use alpha overlays so they work on any surface (sidebar rows,
        // toolbar slots) — the element renders as the parent bg + overlay.
        // Dark themes brighten with white, light themes darken with black.
        let overlay = if d {
            Rgba::rgba(255, 255, 255, 255)
        } else {
            Rgba::rgba(0, 0, 0, 255)
        };

        let element_hover = s.panel.lerp(overlay, if d { 0.12 } else { 0.08 });
        let element_active = s.panel.lerp(overlay, if d { 0.20 } else { 0.14 });
        let ghost_hover = overlay.with_alpha(if d { 18 } else { 22 });
        let ghost_active = overlay.with_alpha(if d { 32 } else { 40 });

        // Accent-tinted hover for "primary" slots (sidebar rows, selected
        // items). Alpha so it tints whatever surface it sits on.
        let accent_tint_hover = s.accent.with_alpha(if d { 38 } else { 48 });
        let accent_tint_selected = s.accent.with_alpha(if d { 66 } else { 76 });

        let c = Color::from;
        ThemeColors {
            app_bg: c(s.app_bg),
            canvas: c(s.canvas),
            panel: c(s.panel),
            panel_strong: c(s.panel_strong),
            border_soft: c(s.border_soft),

            background: c(s.app_bg),
            surface: c(s.panel),
            editor_surface: c(s.canvas),
            elevated_surface: c(s.panel_strong),
            modal_surface: c(s.panel_strong),

            title_bar_background: c(s.panel_strong),
            status_bar_background: c(s.canvas),
            sidebar_background: c(s.canvas),
            empty_state_background: c(s.panel),
            gutter_bg: c(if d { s.app_bg } else { s.panel_strong }),
            file_header_bg: c(s.panel_strong),
            hunk_header_bg: c(s.panel_tint),

            element_background: c(s.panel),
            element_hover: c(element_hover),
            element_active: c(element_active),
            element_selected: c(accent_tint_selected),
            ghost_element_hover: c(ghost_hover),
            ghost_element_active: c(ghost_active),
            ghost_element_selected: c(accent_tint_selected),
            hover_overlay: c(accent_tint_hover),

            sidebar_row_hover: c(accent_tint_hover),
            sidebar_row_selected: c(accent_tint_selected),

            border: c(s.border_strong),
            border_variant: c(s.border_soft),
            focus_border: c(s.accent),
            empty_state_border: c(s.border_strong),

            text_strong: c(text_strong),
            text: c(text),
            text_muted: c(text_muted),
            text_accent: c(s.accent),
            icon: c(icon),
            gutter_text: c(s.text_faint),

            accent: c(s.accent),
            accent_strong: c(s.accent_strong),
            selection_bg: c(s.selection_bg),

            overlay_scrim: c(if d {
                Rgba::rgba(0, 0, 0, 180)
            } else {
                Rgba::rgba(0, 0, 0, 80)
            }),
            scrollbar_thumb: c(if d {
                Rgba::rgba(255, 255, 255, 100)
            } else {
                s.text_muted
            }),

            status_info: c(s.accent),
            status_warning: c(s.warning_text),
            status_error: c(s.danger_text),

            line_add: c(s.line_add),
            line_del: c(s.line_del),
            line_modified: c(s.app_bg.lerp(s.accent, 0.15)),
            line_add_text: c(s.success_text),
            line_del_text: c(s.danger_text),
            line_add_word_bg: c(s.line_add_accent),
            line_del_word_bg: c(s.line_del_accent),

            search_match_bg: c(s.warning_text.with_alpha(if d { 90 } else { 120 })),
            search_match_active_bg: c(s.warning_text.with_alpha(if d { 180 } else { 200 })),

            syntax_keyword: c(s.syn_keyword),
            syntax_string: c(s.syn_string),
            syntax_comment: c(s.syn_comment),
            syntax_function: c(s.syn_function),
            syntax_type: c(s.syn_type),
            syntax_number: c(s.syn_number),
            syntax_property: c(s.syn_property),
            syntax_operator: c(s.syn_operator),
        }
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
        assert!(
            c.b > c.r && c.b > c.g,
            "focus_border should be distinctly blue"
        );
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
                i,
                i - 1,
                curr,
                prev,
            );
        }
    }
}
