use std::collections::HashMap;

use crate::ui::theme::{Color, ThemeColors};

static THEMES_JSON: &str = include_str!("../../data/themes.json");

pub struct ThemeRegistry {
    entries: Vec<ThemeEntry>,
    index: HashMap<String, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeVariant {
    Dual,
    Dark,
    Light,
}

pub struct ThemeEntry {
    pub name: String,
    pub variant: ThemeVariant,
    pub dark: SemanticPalette,
    pub light: SemanticPalette,
}

pub struct SemanticPalette {
    pub app_bg: Color,
    pub canvas: Color,
    pub panel: Color,
    pub panel_strong: Color,
    pub panel_tint: Color,
    pub border_soft: Color,
    pub border_strong: Color,
    pub text_strong: Color,
    pub text_base: Color,
    pub text_muted: Color,
    pub text_faint: Color,
    pub accent: Color,
    pub accent_strong: Color,
    pub accent_soft: Color,
    pub success_bg: Color,
    pub success_text: Color,
    pub danger_bg: Color,
    pub danger_text: Color,
    pub warning_text: Color,
    pub selection_bg: Color,
    pub line_context: Color,
    pub line_add: Color,
    pub line_add_accent: Color,
    pub line_del: Color,
    pub line_del_accent: Color,
    pub syn_keyword: Color,
    pub syn_string: Color,
    pub syn_comment: Color,
    pub syn_function: Color,
    pub syn_type: Color,
    pub syn_number: Color,
    pub syn_property: Color,
    pub syn_operator: Color,
    pub is_dark: bool,
}

impl ThemeRegistry {
    pub fn load() -> Self {
        let raw: serde_json::Value =
            serde_json::from_str(THEMES_JSON).expect("invalid themes.json");
        let themes_arr = raw["themes"].as_array().expect("themes array missing");
        let mut entries = Vec::with_capacity(themes_arr.len());
        let mut index = HashMap::with_capacity(themes_arr.len());

        for value in themes_arr {
            let name = value["name"].as_str().unwrap_or_default().to_owned();
            let dark = SemanticPalette::from_json(&value["dark"]);
            let light = SemanticPalette::from_json(&value["light"]);
            let idx = entries.len();
            let variant = match (dark.is_dark, light.is_dark) {
                (true, false) => ThemeVariant::Dual,
                (true, true) => ThemeVariant::Dark,
                (false, false) => ThemeVariant::Light,
                (false, true) => ThemeVariant::Dual,
            };
            index.insert(name.clone(), idx);
            entries.push(ThemeEntry {
                name,
                variant,
                dark,
                light,
            });
        }

        tracing::info!(count = entries.len(), "theme registry loaded");
        Self { entries, index }
    }

    pub fn get(&self, name: &str) -> Option<&ThemeEntry> {
        self.index.get(name).map(|&i| &self.entries[i])
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn variant(&self, name: &str) -> ThemeVariant {
        self.index
            .get(name)
            .map_or(ThemeVariant::Dark, |&i| self.entries[i].variant)
    }
}

fn parse_hex(s: &str) -> Color {
    let s = s.strip_prefix('#').unwrap_or(s);
    let bytes = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0);
    match s.len() {
        8 => Color::rgba(bytes(0), bytes(2), bytes(4), bytes(6)),
        6 => Color::rgba(bytes(0), bytes(2), bytes(4), 255),
        _ => Color::rgba(0, 0, 0, 255),
    }
}

fn hex_field(obj: &serde_json::Value, key: &str) -> Color {
    obj[key]
        .as_str()
        .map(parse_hex)
        .unwrap_or(Color::rgba(0, 0, 0, 255))
}

fn relative_luminance(c: Color) -> f32 {
    fn linearize(v: u8) -> f32 {
        let s = v as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linearize(c.r) + 0.7152 * linearize(c.g) + 0.0722 * linearize(c.b)
}

fn contrast_ratio(fg: Color, bg: Color) -> f32 {
    let l1 = relative_luminance(fg);
    let l2 = relative_luminance(bg);
    let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

fn ensure_contrast(text: Color, bg: Color, min_ratio: f32, is_dark: bool) -> Color {
    if contrast_ratio(text, bg) >= min_ratio {
        return text;
    }
    let target = if is_dark {
        Color::rgba(255, 255, 255, 255)
    } else {
        Color::rgba(0, 0, 0, 255)
    };
    let mut lo = 0.0_f32;
    let mut hi = 1.0_f32;
    for _ in 0..12 {
        let mid = (lo + hi) * 0.5;
        let candidate = text.lerp(target, mid);
        if contrast_ratio(candidate, bg) >= min_ratio {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    text.lerp(target, hi)
}

impl SemanticPalette {
    fn hex_field_opt(obj: &serde_json::Value, key: &str) -> Option<Color> {
        obj[key].as_str().map(parse_hex)
    }

    fn from_json(obj: &serde_json::Value) -> Self {
        let app_bg = hex_field(obj, "appBg");
        let is_dark = relative_luminance(app_bg) < 0.5;

        let accent = hex_field(obj, "accent");
        let accent_strong = hex_field(obj, "accentStrong");
        let text_muted = hex_field(obj, "textMuted");
        let text_faint = hex_field(obj, "textFaint");
        let success_text = hex_field(obj, "successText");
        let danger_text = hex_field(obj, "dangerText");
        let warning_text = hex_field(obj, "warningText");

        Self {
            app_bg,
            canvas: hex_field(obj, "canvas"),
            panel: hex_field(obj, "panel"),
            panel_strong: hex_field(obj, "panelStrong"),
            panel_tint: hex_field(obj, "panelTint"),
            border_soft: hex_field(obj, "borderSoft"),
            border_strong: hex_field(obj, "borderStrong"),
            text_strong: hex_field(obj, "textStrong"),
            text_base: hex_field(obj, "textBase"),
            text_muted: hex_field(obj, "textMuted"),
            text_faint: hex_field(obj, "textFaint"),
            accent: hex_field(obj, "accent"),
            accent_strong: hex_field(obj, "accentStrong"),
            accent_soft: hex_field(obj, "accentSoft"),
            success_bg: hex_field(obj, "successBg"),
            success_text: hex_field(obj, "successText"),
            danger_bg: hex_field(obj, "dangerBg"),
            danger_text: hex_field(obj, "dangerText"),
            warning_text: hex_field(obj, "warningText"),
            selection_bg: hex_field(obj, "selectionBg"),
            line_context: hex_field(obj, "lineContext"),
            line_add: hex_field(obj, "lineAdd"),
            line_add_accent: hex_field(obj, "lineAddAccent"),
            line_del: hex_field(obj, "lineDel"),
            line_del_accent: hex_field(obj, "lineDelAccent"),
            syn_keyword: Self::hex_field_opt(obj, "synKeyword").unwrap_or(accent_strong),
            syn_string: Self::hex_field_opt(obj, "synString").unwrap_or(success_text),
            syn_comment: Self::hex_field_opt(obj, "synComment").unwrap_or(text_faint),
            syn_function: Self::hex_field_opt(obj, "synFunction").unwrap_or(accent),
            syn_type: Self::hex_field_opt(obj, "synType").unwrap_or(accent),
            syn_number: Self::hex_field_opt(obj, "synNumber").unwrap_or(warning_text),
            syn_property: Self::hex_field_opt(obj, "synProperty").unwrap_or(danger_text),
            syn_operator: Self::hex_field_opt(obj, "synOperator").unwrap_or(text_muted),
            is_dark,
        }
    }

    pub fn to_theme_colors(&self) -> ThemeColors {
        let s = self;
        let d = s.is_dark;

        let text = ensure_contrast(s.text_base, s.panel, 4.5, d);
        let text_strong = ensure_contrast(s.text_strong, s.panel_strong, 4.5, d);
        let text_muted = ensure_contrast(s.text_muted, s.panel, 3.0, d);

        let element_hover = s.panel.lerp(s.text_base, 0.08);
        let element_active = s.panel.lerp(s.text_base, 0.14);

        let ghost_hover = if d {
            Color::rgba(255, 255, 255, 20)
        } else {
            Color::rgba(0, 0, 0, 15)
        };
        let ghost_active = if d {
            Color::rgba(255, 255, 255, 36)
        } else {
            Color::rgba(0, 0, 0, 31)
        };

        ThemeColors {
            app_bg: s.app_bg,
            canvas: s.canvas,
            panel: s.panel,
            panel_strong: s.panel_strong,
            border_soft: s.border_soft,

            background: s.app_bg,
            surface: s.panel,
            editor_surface: s.canvas,
            elevated_surface: s.panel_strong,
            modal_surface: s.panel_strong,

            title_bar_background: s.panel_strong,
            status_bar_background: s.canvas,
            sidebar_background: s.canvas,
            empty_state_background: s.panel,
            gutter_bg: if d { s.app_bg } else { s.panel_strong },
            file_header_bg: s.panel_strong,
            hunk_header_bg: s.panel_tint,

            element_background: s.panel,
            element_hover,
            element_active,
            element_selected: s.accent_soft,
            ghost_element_hover: ghost_hover,
            ghost_element_active: ghost_active,
            ghost_element_selected: s.accent_soft,
            hover_overlay: if d {
                Color::rgba(255, 255, 255, 14)
            } else {
                Color::rgba(0, 0, 0, 20)
            },

            sidebar_row_hover: ghost_hover,
            sidebar_row_selected: s.accent_soft,

            border: s.border_strong,
            border_variant: s.border_soft,
            focus_border: s.accent,
            empty_state_border: s.border_strong,

            text_strong,
            text,
            text_muted,
            text_accent: s.accent,
            icon: s.text_muted,
            gutter_text: s.text_faint,

            accent: s.accent,
            selection_bg: s.selection_bg,

            overlay_scrim: if d {
                Color::rgba(0, 0, 0, 180)
            } else {
                Color::rgba(0, 0, 0, 80)
            },
            scrollbar_thumb: if d {
                Color::rgba(255, 255, 255, 100)
            } else {
                s.text_muted
            },

            status_info: s.accent,
            status_warning: s.warning_text,
            status_error: s.danger_text,

            line_add: s.line_add,
            line_del: s.line_del,
            line_modified: s.app_bg.lerp(s.accent, 0.15),
            line_add_text: s.success_text,
            line_del_text: s.danger_text,
            line_add_word_bg: s.line_add_accent,
            line_del_word_bg: s.line_del_accent,

            search_match_bg: s.warning_text.with_alpha(if d { 90 } else { 120 }),
            search_match_active_bg: s.warning_text.with_alpha(if d { 180 } else { 200 }),

            syntax_keyword: s.syn_keyword,
            syntax_string: s.syn_string,
            syntax_comment: s.syn_comment,
            syntax_function: s.syn_function,
            syntax_type: s.syn_type,
            syntax_number: s.syn_number,
            syntax_property: s.syn_property,
            syntax_operator: s.syn_operator,
        }
    }
}
