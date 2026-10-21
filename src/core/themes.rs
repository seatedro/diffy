use std::collections::HashMap;

use crate::core::theme_types::Rgba;

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
    pub app_bg: Rgba,
    pub canvas: Rgba,
    pub panel: Rgba,
    pub panel_strong: Rgba,
    pub panel_tint: Rgba,
    pub border_soft: Rgba,
    pub border_strong: Rgba,
    pub text_strong: Rgba,
    pub text_base: Rgba,
    pub text_muted: Rgba,
    pub text_faint: Rgba,
    pub accent: Rgba,
    pub accent_strong: Rgba,
    pub accent_soft: Rgba,
    pub success_bg: Rgba,
    pub success_text: Rgba,
    pub danger_bg: Rgba,
    pub danger_text: Rgba,
    pub warning_text: Rgba,
    pub selection_bg: Rgba,
    pub selection_fg: Rgba,
    pub line_context: Rgba,
    pub line_add: Rgba,
    pub line_add_accent: Rgba,
    pub line_del: Rgba,
    pub line_del_accent: Rgba,
    pub syn_keyword: Rgba,
    pub syn_string: Rgba,
    pub syn_comment: Rgba,
    pub syn_function: Rgba,
    pub syn_type: Rgba,
    pub syn_number: Rgba,
    pub syn_property: Rgba,
    pub syn_operator: Rgba,
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

fn parse_hex(s: &str) -> Rgba {
    let s = s.strip_prefix('#').unwrap_or(s);
    let bytes = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0);
    match s.len() {
        8 => Rgba::new(bytes(0), bytes(2), bytes(4), bytes(6)),
        6 => Rgba::new(bytes(0), bytes(2), bytes(4), 255),
        _ => Rgba::BLACK,
    }
}

fn hex_field(obj: &serde_json::Value, key: &str) -> Rgba {
    obj[key].as_str().map(parse_hex).unwrap_or(Rgba::BLACK)
}

impl SemanticPalette {
    fn hex_field_opt(obj: &serde_json::Value, key: &str) -> Option<Rgba> {
        obj[key].as_str().map(parse_hex)
    }

    fn from_json(obj: &serde_json::Value) -> Self {
        let app_bg = hex_field(obj, "appBg");
        let is_dark = app_bg.relative_luminance() < 0.5;

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
            selection_fg: Self::hex_field_opt(obj, "selectionFg")
                .unwrap_or_else(|| hex_field(obj, "textBase")),
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
}
