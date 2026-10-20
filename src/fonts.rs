use serde::{Deserialize, Serialize};

pub use quark_text::fonts::{
    FontFamilyEntry, FontFamilySource, FontRole, MONO_FAMILY, UI_FAMILY, bundled_font_options,
    font_family_entries, font_selection_label,
};

const LEGACY_SYSTEM_FONT_SELECTION: &str = "__diffy_system_font__";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FontSettings {
    #[serde(default = "default_ui_family_string")]
    pub ui_family: String,
    #[serde(default = "default_mono_family_string")]
    pub mono_family: String,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            ui_family: UI_FAMILY.to_owned(),
            mono_family: MONO_FAMILY.to_owned(),
        }
    }
}

impl FontSettings {
    pub fn normalized(&self) -> Self {
        Self {
            ui_family: normalize_font_selection(FontRole::Ui, &self.ui_family),
            mono_family: normalize_font_selection(FontRole::Mono, &self.mono_family),
        }
    }

    pub fn family(&self, role: FontRole) -> &str {
        match role {
            FontRole::Ui => &self.ui_family,
            FontRole::Mono => &self.mono_family,
        }
    }
}

pub fn bundled_font_entries(role: FontRole) -> Vec<FontFamilyEntry> {
    bundled_font_options(role)
        .iter()
        .map(|option| FontFamilyEntry {
            label: option.label.to_owned(),
            family: option.family.to_owned(),
            source: FontFamilySource::Bundled,
            monospaced: role == FontRole::Mono,
        })
        .collect()
}

pub fn normalize_font_selection(role: FontRole, family: &str) -> String {
    let trimmed = family.trim();
    if trimmed.is_empty() || trimmed == LEGACY_SYSTEM_FONT_SELECTION {
        return default_family(role).to_owned();
    }
    trimmed.to_owned()
}

fn default_family(role: FontRole) -> &'static str {
    match role {
        FontRole::Ui => UI_FAMILY,
        FontRole::Mono => MONO_FAMILY,
    }
}

fn default_ui_family_string() -> String {
    UI_FAMILY.to_owned()
}

fn default_mono_family_string() -> String {
    MONO_FAMILY.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_selections_normalize_to_bundled_defaults() {
        let settings = FontSettings {
            ui_family: LEGACY_SYSTEM_FONT_SELECTION.to_owned(),
            mono_family: "  ".to_owned(),
        };
        assert_eq!(settings.normalized(), FontSettings::default());
        assert_eq!(font_selection_label("IBM Plex Mono"), "IBM Plex Mono");
    }

    #[test]
    fn missing_fields_deserialize_to_defaults() {
        let settings: FontSettings = serde_json::from_str(r#"{"ui_family":"Inter"}"#).unwrap();
        assert_eq!(settings.ui_family, "Inter");
        assert_eq!(settings.mono_family, MONO_FAMILY);
    }
}
