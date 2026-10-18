use std::collections::BTreeMap;

use glyphon::fontdb;
use serde::{Deserialize, Serialize};

const LEGACY_SYSTEM_FONT_SELECTION: &str = "__diffy_system_font__";

pub const UI_FAMILY: &str = "Geist";
pub const MONO_FAMILY: &str = "Geist Mono";
pub const INTER_FAMILY: &str = "Inter";
pub const IBM_PLEX_SANS_FAMILY: &str = "IBM Plex Sans";
pub const SOURCE_SANS_3_FAMILY: &str = "Source Sans 3";
pub const JETBRAINS_MONO_FAMILY: &str = "JetBrains Mono";
pub const IBM_PLEX_MONO_FAMILY: &str = "IBM Plex Mono";
pub const FIRA_CODE_FAMILY: &str = "Fira Code";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontRole {
    Ui,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontFamilyOption {
    pub label: &'static str,
    pub family: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFamilySource {
    Bundled,
    System,
}

impl FontFamilySource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bundled => "Bundled",
            Self::System => "System",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFamilyEntry {
    pub label: String,
    pub family: String,
    pub source: FontFamilySource,
    pub monospaced: bool,
}

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

const UI_FONT_OPTIONS: &[FontFamilyOption] = &[
    FontFamilyOption {
        label: "Geist",
        family: UI_FAMILY,
    },
    FontFamilyOption {
        label: "Inter",
        family: INTER_FAMILY,
    },
    FontFamilyOption {
        label: "IBM Plex Sans",
        family: IBM_PLEX_SANS_FAMILY,
    },
    FontFamilyOption {
        label: "Source Sans 3",
        family: SOURCE_SANS_3_FAMILY,
    },
];

const MONO_FONT_OPTIONS: &[FontFamilyOption] = &[
    FontFamilyOption {
        label: "Geist Mono",
        family: MONO_FAMILY,
    },
    FontFamilyOption {
        label: "JetBrains Mono",
        family: JETBRAINS_MONO_FAMILY,
    },
    FontFamilyOption {
        label: "IBM Plex Mono",
        family: IBM_PLEX_MONO_FAMILY,
    },
    FontFamilyOption {
        label: "Fira Code",
        family: FIRA_CODE_FAMILY,
    },
];

pub fn bundled_font_options(role: FontRole) -> &'static [FontFamilyOption] {
    match role {
        FontRole::Ui => UI_FONT_OPTIONS,
        FontRole::Mono => MONO_FONT_OPTIONS,
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

pub fn font_selection_label(selection: &str) -> String {
    let selection = selection.trim();
    UI_FONT_OPTIONS
        .iter()
        .chain(MONO_FONT_OPTIONS.iter())
        .find(|option| option.family == selection)
        .map(|option| option.label.to_owned())
        .unwrap_or_else(|| selection.to_owned())
}

pub fn default_family(role: FontRole) -> &'static str {
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

#[derive(Debug, Clone, Copy, Default)]
struct CatalogFamily {
    monospaced: bool,
}

pub fn scan_font_catalog() -> (Vec<FontFamilyEntry>, Vec<FontFamilyEntry>) {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut families = BTreeMap::<String, CatalogFamily>::new();
    for face in db.faces() {
        for (family, _) in &face.families {
            if family.is_empty() || family.starts_with('.') {
                continue;
            }
            families.entry(family.clone()).or_default().monospaced |= face.monospaced;
        }
    }
    (
        role_catalog(FontRole::Ui, &families),
        role_catalog(FontRole::Mono, &families),
    )
}

fn role_catalog(
    role: FontRole,
    families: &BTreeMap<String, CatalogFamily>,
) -> Vec<FontFamilyEntry> {
    let pinned = bundled_font_options(role);
    let mut entries = bundled_font_entries(role);
    entries.extend(
        families
            .iter()
            .filter(|(family, info)| {
                !pinned.iter().any(|option| option.family == family.as_str())
                    && (role == FontRole::Ui || info.monospaced)
            })
            .map(|(family, info)| FontFamilyEntry {
                label: family.clone(),
                family: family.clone(),
                source: FontFamilySource::System,
                monospaced: info.monospaced,
            }),
    );
    entries
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
        assert_eq!(font_selection_label(IBM_PLEX_MONO_FAMILY), "IBM Plex Mono");
    }

    #[test]
    fn missing_fields_deserialize_to_defaults() {
        let settings: FontSettings = serde_json::from_str(r#"{"ui_family":"Inter"}"#).unwrap();
        assert_eq!(settings.ui_family, INTER_FAMILY);
        assert_eq!(settings.mono_family, MONO_FAMILY);
    }
}
