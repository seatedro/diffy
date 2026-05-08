use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use glyphon::{FontSystem, fontdb};
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
}

pub const UI_REGULAR_OTF: &[u8] = include_bytes!("../assets/fonts/Geist-Regular.otf");
pub const UI_MEDIUM_OTF: &[u8] = include_bytes!("../assets/fonts/Geist-Medium.otf");
pub const UI_SEMIBOLD_OTF: &[u8] = include_bytes!("../assets/fonts/Geist-SemiBold.otf");
pub const UI_BOLD_OTF: &[u8] = include_bytes!("../assets/fonts/Geist-Bold.otf");

pub const MONO_REGULAR_OTF: &[u8] = include_bytes!("../assets/fonts/GeistMono-Regular.otf");
pub const MONO_MEDIUM_OTF: &[u8] = include_bytes!("../assets/fonts/GeistMono-Medium.otf");
pub const MONO_SEMIBOLD_OTF: &[u8] = include_bytes!("../assets/fonts/GeistMono-SemiBold.otf");
pub const MONO_BOLD_OTF: &[u8] = include_bytes!("../assets/fonts/GeistMono-Bold.otf");

pub const INTER_VARIABLE_TTF: &[u8] = include_bytes!("../assets/fonts/Inter-Variable.ttf");
pub const IBM_PLEX_SANS_VARIABLE_TTF: &[u8] =
    include_bytes!("../assets/fonts/IBMPlexSans-Variable.ttf");
pub const SOURCE_SANS_3_REGULAR_TTF: &[u8] =
    include_bytes!("../assets/fonts/SourceSans3-Regular.ttf");
pub const SOURCE_SANS_3_MEDIUM_TTF: &[u8] =
    include_bytes!("../assets/fonts/SourceSans3-Medium.ttf");
pub const SOURCE_SANS_3_SEMIBOLD_TTF: &[u8] =
    include_bytes!("../assets/fonts/SourceSans3-Semibold.ttf");
pub const SOURCE_SANS_3_BOLD_TTF: &[u8] = include_bytes!("../assets/fonts/SourceSans3-Bold.ttf");

pub const JETBRAINS_MONO_VARIABLE_TTF: &[u8] =
    include_bytes!("../assets/fonts/JetBrainsMono-Variable.ttf");
pub const JETBRAINS_MONO_ITALIC_TTF: &[u8] =
    include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf");
pub const FIRA_CODE_VARIABLE_TTF: &[u8] = include_bytes!("../assets/fonts/FiraCode-Variable.ttf");
pub const IBM_PLEX_MONO_REGULAR_TTF: &[u8] =
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf");
pub const IBM_PLEX_MONO_ITALIC_TTF: &[u8] =
    include_bytes!("../assets/fonts/IBMPlexMono-Italic.ttf");
pub const IBM_PLEX_MONO_MEDIUM_TTF: &[u8] =
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf");
pub const IBM_PLEX_MONO_SEMIBOLD_TTF: &[u8] =
    include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf");
pub const IBM_PLEX_MONO_BOLD_TTF: &[u8] = include_bytes!("../assets/fonts/IBMPlexMono-Bold.ttf");

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

static FONT_CATALOG: OnceLock<FontCatalog> = OnceLock::new();

const VENDORED_FONT_BYTES: &[&[u8]] = &[
    UI_REGULAR_OTF,
    UI_MEDIUM_OTF,
    UI_SEMIBOLD_OTF,
    UI_BOLD_OTF,
    MONO_REGULAR_OTF,
    MONO_MEDIUM_OTF,
    MONO_SEMIBOLD_OTF,
    MONO_BOLD_OTF,
    INTER_VARIABLE_TTF,
    IBM_PLEX_SANS_VARIABLE_TTF,
    SOURCE_SANS_3_REGULAR_TTF,
    SOURCE_SANS_3_MEDIUM_TTF,
    SOURCE_SANS_3_SEMIBOLD_TTF,
    SOURCE_SANS_3_BOLD_TTF,
    JETBRAINS_MONO_VARIABLE_TTF,
    JETBRAINS_MONO_ITALIC_TTF,
    FIRA_CODE_VARIABLE_TTF,
    IBM_PLEX_MONO_REGULAR_TTF,
    IBM_PLEX_MONO_ITALIC_TTF,
    IBM_PLEX_MONO_MEDIUM_TTF,
    IBM_PLEX_MONO_SEMIBOLD_TTF,
    IBM_PLEX_MONO_BOLD_TTF,
];

pub fn new_font_system() -> FontSystem {
    new_font_system_with_settings(&FontSettings::default())
}

pub fn new_font_system_with_settings(settings: &FontSettings) -> FontSystem {
    let mut font_system = FontSystem::new_with_fonts(vendored_font_sources());
    configure_generic_families(font_system.db_mut(), settings);
    font_system
}

pub fn configure_font_system(font_system: &mut FontSystem) {
    configure_font_system_with_settings(font_system, &FontSettings::default());
}

pub fn configure_font_system_with_settings(font_system: &mut FontSystem, settings: &FontSettings) {
    let db = font_system.db_mut();
    load_vendored_fonts(db);
    configure_generic_families(db, settings);
}

fn configure_generic_families(db: &mut fontdb::Database, settings: &FontSettings) {
    let settings = settings.normalized();
    let ui_family = resolve_family(db, FontRole::Ui, &settings.ui_family);
    let mono_family = resolve_family(db, FontRole::Mono, &settings.mono_family);
    db.set_sans_serif_family(ui_family);
    db.set_monospace_family(mono_family);
}

pub fn bundled_font_options(role: FontRole) -> &'static [FontFamilyOption] {
    match role {
        FontRole::Ui => UI_FONT_OPTIONS,
        FontRole::Mono => MONO_FONT_OPTIONS,
    }
}

pub fn font_family_entries(role: FontRole) -> &'static [FontFamilyEntry] {
    let catalog = FONT_CATALOG.get_or_init(build_font_catalog);
    match role {
        FontRole::Ui => &catalog.ui,
        FontRole::Mono => &catalog.mono,
    }
}

pub fn normalize_font_selection(role: FontRole, family: &str) -> String {
    let trimmed = family.trim();
    if trimmed.is_empty() {
        return default_family(role).to_owned();
    }

    if trimmed == LEGACY_SYSTEM_FONT_SELECTION {
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

fn load_vendored_fonts(db: &mut fontdb::Database) {
    for font_bytes in VENDORED_FONT_BYTES.iter().copied() {
        db.load_font_data(font_bytes.to_vec());
    }
}

fn vendored_font_sources() -> impl Iterator<Item = fontdb::Source> {
    VENDORED_FONT_BYTES
        .iter()
        .copied()
        .map(|bytes| fontdb::Source::Binary(Arc::new(bytes.to_vec())))
}

fn resolve_family(db: &fontdb::Database, role: FontRole, selection: &str) -> String {
    if family_available(db, selection) {
        selection.to_owned()
    } else {
        default_family(role).to_owned()
    }
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

fn family_available(db: &fontdb::Database, family: &str) -> bool {
    db.faces().any(|face| {
        face.families
            .iter()
            .any(|(candidate, _)| candidate == family)
    })
}

#[derive(Debug)]
struct FontCatalog {
    ui: Vec<FontFamilyEntry>,
    mono: Vec<FontFamilyEntry>,
}

#[derive(Debug, Clone, Copy)]
struct CatalogFamily {
    source: FontFamilySource,
    monospaced: bool,
}

impl Default for CatalogFamily {
    fn default() -> Self {
        Self {
            source: FontFamilySource::System,
            monospaced: false,
        }
    }
}

fn build_font_catalog() -> FontCatalog {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    load_vendored_fonts(&mut db);

    let mut families = BTreeMap::<String, CatalogFamily>::new();
    for face in db.faces() {
        let source = match &face.source {
            fontdb::Source::Binary(_) => FontFamilySource::Bundled,
            _ => FontFamilySource::System,
        };
        for (family, _) in &face.families {
            if !show_catalog_family(family, source) {
                continue;
            }
            let entry = families.entry(family.clone()).or_default();
            entry.monospaced |= face.monospaced;
            if source == FontFamilySource::Bundled {
                entry.source = FontFamilySource::Bundled;
            }
        }
    }

    FontCatalog {
        ui: build_role_catalog(FontRole::Ui, &families),
        mono: build_role_catalog(FontRole::Mono, &families),
    }
}

fn show_catalog_family(family: &str, source: FontFamilySource) -> bool {
    !family.is_empty() && (source == FontFamilySource::Bundled || !family.starts_with('.'))
}

fn build_role_catalog(
    role: FontRole,
    families: &BTreeMap<String, CatalogFamily>,
) -> Vec<FontFamilyEntry> {
    let pinned = bundled_font_options(role);
    let mut entries = Vec::new();

    for option in pinned {
        let monospaced = families
            .get(option.family)
            .map(|family| family.monospaced)
            .unwrap_or(matches!(role, FontRole::Mono));
        entries.push(FontFamilyEntry {
            label: option.label.to_owned(),
            family: option.family.to_owned(),
            source: FontFamilySource::Bundled,
            monospaced,
        });
    }

    for (family, info) in families {
        if pinned.iter().any(|option| option.family == family) {
            continue;
        }
        if role == FontRole::Mono && !info.monospaced {
            continue;
        }
        entries.push(FontFamilyEntry {
            label: family.clone(),
            family: family.clone(),
            source: info.source,
            monospaced: info.monospaced,
        });
    }

    entries
}
