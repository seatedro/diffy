use std::collections::HashMap;

use qmetaobject::*;
use qttypes::QSettings;
use serde::Deserialize;

#[derive(Clone, Default)]
struct ThemeColors {
    app_bg: QColor,
    canvas: QColor,
    panel: QColor,
    panel_strong: QColor,
    panel_tint: QColor,
    toolbar_bg: QColor,
    border_soft: QColor,
    border_strong: QColor,
    divider: QColor,
    text_strong: QColor,
    text_base: QColor,
    text_muted: QColor,
    text_faint: QColor,
    accent: QColor,
    accent_strong: QColor,
    accent_soft: QColor,
    success_bg: QColor,
    success_border: QColor,
    success_text: QColor,
    danger_bg: QColor,
    danger_border: QColor,
    danger_text: QColor,
    warning_bg: QColor,
    warning_border: QColor,
    warning_text: QColor,
    selection_bg: QColor,
    selection_border: QColor,
    line_context: QColor,
    line_context_alt: QColor,
    line_add: QColor,
    line_add_accent: QColor,
    line_del: QColor,
    line_del_accent: QColor,
    shadow_sm: QColor,
    shadow_md: QColor,
    shadow_lg: QColor,
}

#[derive(Clone)]
struct ThemeVariants {
    dark: ThemeColors,
    light: ThemeColors,
}

impl Default for ThemeVariants {
    fn default() -> Self {
        let colors = built_in_dark_theme_colors();
        Self {
            dark: colors.clone(),
            light: colors,
        }
    }
}

#[derive(Deserialize)]
struct ThemeFile {
    themes: Vec<ThemeDefinition>,
}

#[derive(Deserialize)]
struct ThemeDefinition {
    name: String,
    dark: ThemeColorDefinition,
    light: ThemeColorDefinition,
}

#[derive(Deserialize)]
struct ThemeColorDefinition {
    #[serde(rename = "appBg")]
    app_bg: String,
    canvas: String,
    panel: String,
    #[serde(rename = "panelStrong")]
    panel_strong: String,
    #[serde(rename = "panelTint")]
    panel_tint: String,
    #[serde(rename = "toolbarBg")]
    toolbar_bg: String,
    #[serde(rename = "borderSoft")]
    border_soft: String,
    #[serde(rename = "borderStrong")]
    border_strong: String,
    divider: String,
    #[serde(rename = "textStrong")]
    text_strong: String,
    #[serde(rename = "textBase")]
    text_base: String,
    #[serde(rename = "textMuted")]
    text_muted: String,
    #[serde(rename = "textFaint")]
    text_faint: String,
    accent: String,
    #[serde(rename = "accentStrong")]
    accent_strong: String,
    #[serde(rename = "accentSoft")]
    accent_soft: String,
    #[serde(rename = "successBg")]
    success_bg: String,
    #[serde(rename = "successBorder")]
    success_border: String,
    #[serde(rename = "successText")]
    success_text: String,
    #[serde(rename = "dangerBg")]
    danger_bg: String,
    #[serde(rename = "dangerBorder")]
    danger_border: String,
    #[serde(rename = "dangerText")]
    danger_text: String,
    #[serde(rename = "warningBg")]
    warning_bg: String,
    #[serde(rename = "warningBorder")]
    warning_border: String,
    #[serde(rename = "warningText")]
    warning_text: String,
    #[serde(rename = "selectionBg")]
    selection_bg: String,
    #[serde(rename = "selectionBorder")]
    selection_border: String,
    #[serde(rename = "lineContext")]
    line_context: String,
    #[serde(rename = "lineContextAlt")]
    line_context_alt: String,
    #[serde(rename = "lineAdd")]
    line_add: String,
    #[serde(rename = "lineAddAccent")]
    line_add_accent: String,
    #[serde(rename = "lineDel")]
    line_del: String,
    #[serde(rename = "lineDelAccent")]
    line_del_accent: String,
    #[serde(rename = "shadowSm")]
    shadow_sm: String,
    #[serde(rename = "shadowMd")]
    shadow_md: String,
    #[serde(rename = "shadowLg")]
    shadow_lg: String,
}

#[derive(QObject)]
pub struct ThemeProvider {
    base: qt_base_class!(trait QObject),

    sans: qt_property!(QString; CONST),
    mono: qt_property!(QString; CONST),

    sp1: qt_property!(i32; CONST),
    sp2: qt_property!(i32; CONST),
    sp3: qt_property!(i32; CONST),
    sp4: qt_property!(i32; CONST),
    sp6: qt_property!(i32; CONST),
    sp8: qt_property!(i32; CONST),
    sp12: qt_property!(i32; CONST),

    font_caption: qt_property!(i32; CONST),
    font_small: qt_property!(i32; CONST),
    font_body: qt_property!(i32; CONST),
    font_subtitle: qt_property!(i32; CONST),
    font_title: qt_property!(i32; CONST),
    font_heading: qt_property!(i32; CONST),

    radius_sm: qt_property!(i32; CONST),
    radius_md: qt_property!(i32; CONST),
    radius_lg: qt_property!(i32; CONST),
    radius_xl: qt_property!(i32; CONST),

    current_theme: qt_property!(QString; READ get_current_theme NOTIFY theme_changed),
    current_mode: qt_property!(QString; READ get_current_mode NOTIFY theme_changed),
    available_themes: qt_property!(QVariantList; READ get_available_themes NOTIFY theme_changed),
    available_modes: qt_property!(QVariantList; READ get_available_modes NOTIFY theme_changed),

    app_bg: qt_property!(QColor; READ get_app_bg NOTIFY theme_changed),
    canvas: qt_property!(QColor; READ get_canvas NOTIFY theme_changed),
    panel: qt_property!(QColor; READ get_panel NOTIFY theme_changed),
    panel_strong: qt_property!(QColor; READ get_panel_strong NOTIFY theme_changed),
    panel_tint: qt_property!(QColor; READ get_panel_tint NOTIFY theme_changed),
    toolbar_bg: qt_property!(QColor; READ get_toolbar_bg NOTIFY theme_changed),
    border_soft: qt_property!(QColor; READ get_border_soft NOTIFY theme_changed),
    border_strong: qt_property!(QColor; READ get_border_strong NOTIFY theme_changed),
    divider: qt_property!(QColor; READ get_divider NOTIFY theme_changed),
    text_strong: qt_property!(QColor; READ get_text_strong NOTIFY theme_changed),
    text_base: qt_property!(QColor; READ get_text_base NOTIFY theme_changed),
    text_muted: qt_property!(QColor; READ get_text_muted NOTIFY theme_changed),
    text_faint: qt_property!(QColor; READ get_text_faint NOTIFY theme_changed),
    accent: qt_property!(QColor; READ get_accent NOTIFY theme_changed),
    accent_strong: qt_property!(QColor; READ get_accent_strong NOTIFY theme_changed),
    accent_soft: qt_property!(QColor; READ get_accent_soft NOTIFY theme_changed),
    success_bg: qt_property!(QColor; READ get_success_bg NOTIFY theme_changed),
    success_border: qt_property!(QColor; READ get_success_border NOTIFY theme_changed),
    success_text: qt_property!(QColor; READ get_success_text NOTIFY theme_changed),
    danger_bg: qt_property!(QColor; READ get_danger_bg NOTIFY theme_changed),
    danger_border: qt_property!(QColor; READ get_danger_border NOTIFY theme_changed),
    danger_text: qt_property!(QColor; READ get_danger_text NOTIFY theme_changed),
    warning_bg: qt_property!(QColor; READ get_warning_bg NOTIFY theme_changed),
    warning_border: qt_property!(QColor; READ get_warning_border NOTIFY theme_changed),
    warning_text: qt_property!(QColor; READ get_warning_text NOTIFY theme_changed),
    selection_bg: qt_property!(QColor; READ get_selection_bg NOTIFY theme_changed),
    selection_border: qt_property!(QColor; READ get_selection_border NOTIFY theme_changed),
    line_context: qt_property!(QColor; READ get_line_context NOTIFY theme_changed),
    line_context_alt: qt_property!(QColor; READ get_line_context_alt NOTIFY theme_changed),
    line_add: qt_property!(QColor; READ get_line_add NOTIFY theme_changed),
    line_add_accent: qt_property!(QColor; READ get_line_add_accent NOTIFY theme_changed),
    line_del: qt_property!(QColor; READ get_line_del NOTIFY theme_changed),
    line_del_accent: qt_property!(QColor; READ get_line_del_accent NOTIFY theme_changed),
    shadow_sm: qt_property!(QColor; READ get_shadow_sm NOTIFY theme_changed),
    shadow_md: qt_property!(QColor; READ get_shadow_md NOTIFY theme_changed),
    shadow_lg: qt_property!(QColor; READ get_shadow_lg NOTIFY theme_changed),

    theme_changed: qt_signal!(),

    set_theme: qt_method!(fn(&mut self, name: QString, persist: bool)),
    set_mode: qt_method!(fn(&mut self, mode: QString, persist: bool)),
    toggle_mode: qt_method!(fn(&mut self, persist: bool)),

    settings: QSettings,
    themes: HashMap<String, ThemeVariants>,
    theme_names: Vec<String>,
    current_theme_value: String,
    current_mode_value: String,
    colors: ThemeColors,
}

impl Default for ThemeProvider {
    fn default() -> Self {
        let mut theme_names = Vec::new();
        let mut themes = HashMap::new();
        register_theme(
            &mut themes,
            &mut theme_names,
            "Diffy".to_owned(),
            built_in_dark_theme_colors(),
            built_in_light_theme_colors(),
        );
        if let Ok(parsed) = serde_json::from_str::<ThemeFile>(include_str!("ghostty_themes.json")) {
            for theme in parsed.themes {
                register_theme(
                    &mut themes,
                    &mut theme_names,
                    theme.name.trim().to_owned(),
                    theme.dark.into(),
                    theme.light.into(),
                );
            }
        }

        let settings = QSettings::from_path(&settings_file_path("theme.ini"));
        let fallback_theme = if themes.contains_key("Diffy") {
            "Diffy".to_owned()
        } else {
            theme_names
                .first()
                .cloned()
                .unwrap_or_else(|| "Diffy".to_owned())
        };
        let stored_theme = settings.value_string("theme");
        let stored_mode = settings.value_string("themeMode");
        let current_theme_value =
            resolve_theme_name(&theme_names, &themes, &stored_theme).unwrap_or(fallback_theme);
        let current_mode_value =
            normalize_mode_name(&stored_mode).unwrap_or_else(|| "light".to_owned());
        settings.sync();
        let colors = select_colors(&themes, &current_theme_value, &current_mode_value);

        Self {
            base: Default::default(),
            sans: QString::from("IBM Plex Sans"),
            mono: QString::from("JetBrains Mono"),
            sp1: 4,
            sp2: 8,
            sp3: 12,
            sp4: 16,
            sp6: 24,
            sp8: 32,
            sp12: 48,
            font_caption: 9,
            font_small: 10,
            font_body: 12,
            font_subtitle: 14,
            font_title: 18,
            font_heading: 24,
            radius_sm: 4,
            radius_md: 6,
            radius_lg: 8,
            radius_xl: 12,
            current_theme: Default::default(),
            current_mode: Default::default(),
            available_themes: Default::default(),
            available_modes: Default::default(),
            app_bg: Default::default(),
            canvas: Default::default(),
            panel: Default::default(),
            panel_strong: Default::default(),
            panel_tint: Default::default(),
            toolbar_bg: Default::default(),
            border_soft: Default::default(),
            border_strong: Default::default(),
            divider: Default::default(),
            text_strong: Default::default(),
            text_base: Default::default(),
            text_muted: Default::default(),
            text_faint: Default::default(),
            accent: Default::default(),
            accent_strong: Default::default(),
            accent_soft: Default::default(),
            success_bg: Default::default(),
            success_border: Default::default(),
            success_text: Default::default(),
            danger_bg: Default::default(),
            danger_border: Default::default(),
            danger_text: Default::default(),
            warning_bg: Default::default(),
            warning_border: Default::default(),
            warning_text: Default::default(),
            selection_bg: Default::default(),
            selection_border: Default::default(),
            line_context: Default::default(),
            line_context_alt: Default::default(),
            line_add: Default::default(),
            line_add_accent: Default::default(),
            line_del: Default::default(),
            line_del_accent: Default::default(),
            shadow_sm: Default::default(),
            shadow_md: Default::default(),
            shadow_lg: Default::default(),
            theme_changed: Default::default(),
            set_theme: Default::default(),
            set_mode: Default::default(),
            toggle_mode: Default::default(),
            settings,
            themes,
            theme_names,
            current_theme_value,
            current_mode_value,
            colors,
        }
    }
}

impl ThemeProvider {
    pub fn get_current_theme(&self) -> QString {
        QString::from(self.current_theme_value.as_str())
    }

    pub fn get_current_mode(&self) -> QString {
        QString::from(self.current_mode_value.as_str())
    }

    pub fn get_available_themes(&self) -> QVariantList {
        self.theme_names
            .iter()
            .cloned()
            .map(QString::from)
            .collect()
    }

    pub fn get_available_modes(&self) -> QVariantList {
        [QString::from("dark"), QString::from("light")]
            .into_iter()
            .collect()
    }

    pub fn set_theme(&mut self, name: QString, persist: bool) {
        let Some(resolved) = resolve_theme_name(&self.theme_names, &self.themes, &name.to_string())
        else {
            return;
        };
        if resolved == self.current_theme_value {
            if persist {
                self.persist();
            }
            return;
        }
        self.current_theme_value = resolved;
        self.colors = select_colors(
            &self.themes,
            &self.current_theme_value,
            &self.current_mode_value,
        );
        if persist {
            self.persist();
        }
        self.theme_changed();
    }

    pub fn set_mode(&mut self, mode: QString, persist: bool) {
        let Some(resolved) = normalize_mode_name(&mode.to_string()) else {
            return;
        };
        if resolved == self.current_mode_value {
            if persist {
                self.persist();
            }
            return;
        }
        self.current_mode_value = resolved;
        self.colors = select_colors(
            &self.themes,
            &self.current_theme_value,
            &self.current_mode_value,
        );
        if persist {
            self.persist();
        }
        self.theme_changed();
    }

    pub fn toggle_mode(&mut self, persist: bool) {
        let next = if self.current_mode_value == "dark" {
            "light"
        } else {
            "dark"
        };
        self.set_mode(QString::from(next), persist);
    }

    pub fn get_app_bg(&self) -> QColor {
        self.colors.app_bg.clone()
    }
    pub fn get_canvas(&self) -> QColor {
        self.colors.canvas.clone()
    }
    pub fn get_panel(&self) -> QColor {
        self.colors.panel.clone()
    }
    pub fn get_panel_strong(&self) -> QColor {
        self.colors.panel_strong.clone()
    }
    pub fn get_panel_tint(&self) -> QColor {
        self.colors.panel_tint.clone()
    }
    pub fn get_toolbar_bg(&self) -> QColor {
        self.colors.toolbar_bg.clone()
    }
    pub fn get_border_soft(&self) -> QColor {
        self.colors.border_soft.clone()
    }
    pub fn get_border_strong(&self) -> QColor {
        self.colors.border_strong.clone()
    }
    pub fn get_divider(&self) -> QColor {
        self.colors.divider.clone()
    }
    pub fn get_text_strong(&self) -> QColor {
        self.colors.text_strong.clone()
    }
    pub fn get_text_base(&self) -> QColor {
        self.colors.text_base.clone()
    }
    pub fn get_text_muted(&self) -> QColor {
        self.colors.text_muted.clone()
    }
    pub fn get_text_faint(&self) -> QColor {
        self.colors.text_faint.clone()
    }
    pub fn get_accent(&self) -> QColor {
        self.colors.accent.clone()
    }
    pub fn get_accent_strong(&self) -> QColor {
        self.colors.accent_strong.clone()
    }
    pub fn get_accent_soft(&self) -> QColor {
        self.colors.accent_soft.clone()
    }
    pub fn get_success_bg(&self) -> QColor {
        self.colors.success_bg.clone()
    }
    pub fn get_success_border(&self) -> QColor {
        self.colors.success_border.clone()
    }
    pub fn get_success_text(&self) -> QColor {
        self.colors.success_text.clone()
    }
    pub fn get_danger_bg(&self) -> QColor {
        self.colors.danger_bg.clone()
    }
    pub fn get_danger_border(&self) -> QColor {
        self.colors.danger_border.clone()
    }
    pub fn get_danger_text(&self) -> QColor {
        self.colors.danger_text.clone()
    }
    pub fn get_warning_bg(&self) -> QColor {
        self.colors.warning_bg.clone()
    }
    pub fn get_warning_border(&self) -> QColor {
        self.colors.warning_border.clone()
    }
    pub fn get_warning_text(&self) -> QColor {
        self.colors.warning_text.clone()
    }
    pub fn get_selection_bg(&self) -> QColor {
        self.colors.selection_bg.clone()
    }
    pub fn get_selection_border(&self) -> QColor {
        self.colors.selection_border.clone()
    }
    pub fn get_line_context(&self) -> QColor {
        self.colors.line_context.clone()
    }
    pub fn get_line_context_alt(&self) -> QColor {
        self.colors.line_context_alt.clone()
    }
    pub fn get_line_add(&self) -> QColor {
        self.colors.line_add.clone()
    }
    pub fn get_line_add_accent(&self) -> QColor {
        self.colors.line_add_accent.clone()
    }
    pub fn get_line_del(&self) -> QColor {
        self.colors.line_del.clone()
    }
    pub fn get_line_del_accent(&self) -> QColor {
        self.colors.line_del_accent.clone()
    }
    pub fn get_shadow_sm(&self) -> QColor {
        self.colors.shadow_sm.clone()
    }
    pub fn get_shadow_md(&self) -> QColor {
        self.colors.shadow_md.clone()
    }
    pub fn get_shadow_lg(&self) -> QColor {
        self.colors.shadow_lg.clone()
    }

    fn persist(&mut self) {
        self.settings.set_string("theme", &self.current_theme_value);
        self.settings
            .set_string("themeMode", &self.current_mode_value);
        self.settings.sync();
    }
}

fn register_theme(
    themes: &mut HashMap<String, ThemeVariants>,
    theme_names: &mut Vec<String>,
    name: String,
    dark: ThemeColors,
    light: ThemeColors,
) {
    if name.trim().is_empty() {
        return;
    }
    if !theme_names
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&name))
    {
        theme_names.push(name.clone());
    }
    themes.insert(name, ThemeVariants { dark, light });
}

fn resolve_theme_name(
    names: &[String],
    themes: &HashMap<String, ThemeVariants>,
    candidate: &str,
) -> Option<String> {
    let trimmed = candidate.trim();
    if themes.contains_key(trimmed) {
        return Some(trimmed.to_owned());
    }
    names
        .iter()
        .find(|name| name.eq_ignore_ascii_case(trimmed))
        .cloned()
        .or_else(|| names.first().cloned())
}

fn normalize_mode_name(mode: &str) -> Option<String> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "dark" => Some("dark".to_owned()),
        "light" => Some("light".to_owned()),
        _ => None,
    }
}

fn select_colors(themes: &HashMap<String, ThemeVariants>, theme: &str, mode: &str) -> ThemeColors {
    let fallback = themes
        .get(theme)
        .or_else(|| themes.get("Diffy"))
        .or_else(|| themes.values().next())
        .cloned()
        .unwrap_or_else(|| ThemeVariants {
            dark: built_in_dark_theme_colors(),
            light: built_in_light_theme_colors(),
        });
    if mode == "light" {
        fallback.light
    } else {
        fallback.dark
    }
}

fn color(value: &str) -> QColor {
    QColor::from_name(value)
}

fn settings_file_path(file: &str) -> String {
    let base = std::env::var("DIFFY_SETTINGS_DIR")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("XDG_CONFIG_HOME")
                .ok()
                .map(|value| format!("{value}/diffy"))
        })
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned());
            format!("{home}/.config/diffy")
        });
    let _ = std::fs::create_dir_all(&base);
    format!("{base}/{file}")
}

impl From<ThemeColorDefinition> for ThemeColors {
    fn from(value: ThemeColorDefinition) -> Self {
        Self {
            app_bg: color(&value.app_bg),
            canvas: color(&value.canvas),
            panel: color(&value.panel),
            panel_strong: color(&value.panel_strong),
            panel_tint: color(&value.panel_tint),
            toolbar_bg: color(&value.toolbar_bg),
            border_soft: color(&value.border_soft),
            border_strong: color(&value.border_strong),
            divider: color(&value.divider),
            text_strong: color(&value.text_strong),
            text_base: color(&value.text_base),
            text_muted: color(&value.text_muted),
            text_faint: color(&value.text_faint),
            accent: color(&value.accent),
            accent_strong: color(&value.accent_strong),
            accent_soft: color(&value.accent_soft),
            success_bg: color(&value.success_bg),
            success_border: color(&value.success_border),
            success_text: color(&value.success_text),
            danger_bg: color(&value.danger_bg),
            danger_border: color(&value.danger_border),
            danger_text: color(&value.danger_text),
            warning_bg: color(&value.warning_bg),
            warning_border: color(&value.warning_border),
            warning_text: color(&value.warning_text),
            selection_bg: color(&value.selection_bg),
            selection_border: color(&value.selection_border),
            line_context: color(&value.line_context),
            line_context_alt: color(&value.line_context_alt),
            line_add: color(&value.line_add),
            line_add_accent: color(&value.line_add_accent),
            line_del: color(&value.line_del),
            line_del_accent: color(&value.line_del_accent),
            shadow_sm: color(&value.shadow_sm),
            shadow_md: color(&value.shadow_md),
            shadow_lg: color(&value.shadow_lg),
        }
    }
}

fn built_in_dark_theme_colors() -> ThemeColors {
    ThemeColors {
        app_bg: color("#1b1e24"),
        canvas: color("#20242b"),
        panel: color("#282d36"),
        panel_strong: color("#323844"),
        panel_tint: color("#2b4d6d"),
        toolbar_bg: color("#20242b"),
        border_soft: color("#39414d"),
        border_strong: color("#4e5968"),
        divider: color("#39414d"),
        text_strong: color("#f2f5f8"),
        text_base: color("#e2e7ec"),
        text_muted: color("#a9b3bf"),
        text_faint: color("#7f8894"),
        accent: color("#5da9f6"),
        accent_strong: color("#8cc3ff"),
        accent_soft: color("#23394d"),
        success_bg: color("#24352a"),
        success_border: color("#335843"),
        success_text: color("#7dd69c"),
        danger_bg: color("#3a2728"),
        danger_border: color("#6a3f40"),
        danger_text: color("#f28b82"),
        warning_bg: color("#3d3224"),
        warning_border: color("#705734"),
        warning_text: color("#f3c56c"),
        selection_bg: color("#2f3b4f"),
        selection_border: color("#5da9f6"),
        line_context: color("#20242b"),
        line_context_alt: color("#232831"),
        line_add: color("#24342a"),
        line_add_accent: color("#2d4736"),
        line_del: color("#382728"),
        line_del_accent: color("#4a3133"),
        shadow_sm: color("#1a000000"),
        shadow_md: color("#33000000"),
        shadow_lg: color("#4d000000"),
    }
}

fn built_in_light_theme_colors() -> ThemeColors {
    ThemeColors {
        app_bg: color("#f7f7f5"),
        canvas: color("#fbfbfa"),
        panel: color("#f1f1ef"),
        panel_strong: color("#e6e6e3"),
        panel_tint: color("#d7e4f2"),
        toolbar_bg: color("#fbfbfa"),
        border_soft: color("#d3d3cf"),
        border_strong: color("#b4b4af"),
        divider: color("#d3d3cf"),
        text_strong: color("#1e1f21"),
        text_base: color("#2a2c2f"),
        text_muted: color("#5f646b"),
        text_faint: color("#858b93"),
        accent: color("#0f68a0"),
        accent_strong: color("#0b4f79"),
        accent_soft: color("#d8e7f1"),
        success_bg: color("#e8f1ea"),
        success_border: color("#cfe2d4"),
        success_text: color("#2f6f3e"),
        danger_bg: color("#f6e8e6"),
        danger_border: color("#ecc9c4"),
        danger_text: color("#b63424"),
        warning_bg: color("#f5efe1"),
        warning_border: color("#e7d6b5"),
        warning_text: color("#8a5a18"),
        selection_bg: color("#dbe9f6"),
        selection_border: color("#0f68a0"),
        line_context: color("#fbfbfa"),
        line_context_alt: color("#f7f7f5"),
        line_add: color("#edf5ef"),
        line_add_accent: color("#d8eadf"),
        line_del: color("#f9ecea"),
        line_del_accent: color("#f1d6d1"),
        shadow_sm: color("#0a000000"),
        shadow_md: color("#15000000"),
        shadow_lg: color("#22000000"),
    }
}
