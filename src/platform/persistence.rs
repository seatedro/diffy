use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sea_query::{Expr, ExprTrait, OnConflict, Query};
use serde::{Deserialize, Serialize};

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::core::error::Result;
use crate::core::forge::github::GitHubUser;
use crate::core::keymap::KeymapOverride;
use crate::core::theme_types::ThemeMode;
use crate::fonts::FontSettings;
use crate::platform::database::{
    DATABASE_FILE_NAME, Database, Settings as SettingsTable, remove_imported,
};

const SETTINGS_FILE_NAME: &str = "settings.json";
const SETTINGS_ROW: i32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedCompare {
    pub repo_path: Option<PathBuf>,
    pub left_ref: String,
    pub right_ref: String,
    pub mode: CompareMode,
    pub layout: LayoutMode,
    pub renderer: RendererKind,
}

impl Default for PersistedCompare {
    fn default() -> Self {
        Self {
            repo_path: None,
            left_ref: String::new(),
            right_ref: String::new(),
            mode: CompareMode::default(),
            layout: LayoutMode::default(),
            renderer: RendererKind::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedViewport {
    pub wrap_enabled: bool,
    pub wrap_column: u32,
    pub layout: LayoutMode,
}

impl Default for PersistedViewport {
    fn default() -> Self {
        Self {
            wrap_enabled: false,
            wrap_column: 0,
            layout: LayoutMode::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepoDiscovery {
    pub roots: Vec<String>,
    pub depth: u8,
    pub hidden: bool,
}

impl Default for RepoDiscovery {
    fn default() -> Self {
        Self {
            roots: vec!["~".to_owned()],
            depth: 5,
            hidden: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme_mode: ThemeMode,
    pub theme_name: String,
    pub ui_scale_pct: u16,
    pub fonts: FontSettings,
    pub sidebar_width_px: Option<u32>,
    pub last_compare: Option<PersistedCompare>,
    pub viewport: PersistedViewport,
    pub github_user: Option<GitHubUser>,
    pub wheel_scroll_lines: u8,
    pub auto_update: bool,
    pub continuous_scroll: bool,
    pub ai_steering_prompt: String,
    pub ai_provider: Option<crate::ai::Provider>,
    pub ai_use_cli: bool,
    pub keymap_overrides: Vec<KeymapOverride>,
    pub repo_discovery: RepoDiscovery,
    pub github_clones: BTreeMap<String, PathBuf>,
    pub tips: crate::core::tips::TipsMemory,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::Dark,
            theme_name: crate::app::theme::DEFAULT_THEME.to_owned(),
            ui_scale_pct: 100,
            fonts: FontSettings::default(),
            sidebar_width_px: None,
            last_compare: None,
            viewport: PersistedViewport::default(),
            github_user: None,
            wheel_scroll_lines: 3,
            auto_update: true,
            continuous_scroll: false,
            ai_steering_prompt: String::new(),
            ai_provider: None,
            ai_use_cli: false,
            keymap_overrides: Vec::new(),
            repo_discovery: RepoDiscovery::default(),
            github_clones: BTreeMap::new(),
            tips: crate::core::tips::TipsMemory::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    dir: PathBuf,
}

impl SettingsStore {
    pub fn new_default() -> Self {
        let base = crate::platform::startup::app_config_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        Self::new_in(base)
    }

    pub fn new_in(path: impl Into<PathBuf>) -> Self {
        let mut dir = path.into();
        if dir.extension().is_some() {
            dir.pop();
        }
        Self { dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(DATABASE_FILE_NAME)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn load(&self) -> Result<Settings> {
        let db = self.database()?;
        let select = Query::select()
            .column(SettingsTable::Json)
            .from(SettingsTable::Table)
            .and_where(Expr::col(SettingsTable::Id).eq(SETTINGS_ROW))
            .to_owned();
        let Some(json) = db.query(&select, |row| row.get::<_, String>(0))?.pop() else {
            return Ok(Settings::default());
        };
        match serde_json::from_str(&json) {
            Ok(settings) => Ok(settings),
            Err(e) => {
                tracing::warn!("corrupt settings, using defaults: {e}");
                Ok(Settings::default())
            }
        }
    }

    pub fn save(&self, settings: &Settings) -> Result<()> {
        write_settings(&self.database()?, &serde_json::to_string(settings)?)
    }

    fn database(&self) -> Result<Database> {
        let db = Database::in_dir(&self.dir)?;
        let legacy = self.dir.join(SETTINGS_FILE_NAME);
        db.import_once("settings", |db| match fs::read_to_string(&legacy) {
            Ok(json) if serde_json::from_str::<Settings>(&json).is_ok() => {
                write_settings(db, &json)?;
                remove_imported(&legacy);
                Ok(())
            }
            _ => Ok(()),
        });
        Ok(db)
    }
}

fn write_settings(db: &Database, json: &str) -> Result<()> {
    db.execute(
        &Query::insert()
            .into_table(SettingsTable::Table)
            .columns([SettingsTable::Id, SettingsTable::Json])
            .values_panic([SETTINGS_ROW.into(), json.into()])
            .on_conflict(
                OnConflict::column(SettingsTable::Id)
                    .update_column(SettingsTable::Json)
                    .to_owned(),
            )
            .to_owned(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{PersistedCompare, Settings, SettingsStore};
    use crate::core::compare::{CompareMode, LayoutMode, RendererKind};

    #[test]
    fn imports_settings_json_once() {
        let dir = TempDir::new().unwrap();
        let legacy = Settings {
            theme_name: "legacy".to_owned(),
            ..Settings::default()
        };
        std::fs::write(
            dir.path().join("settings.json"),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let store = SettingsStore::new_in(dir.path());
        assert_eq!(store.load().unwrap(), legacy);
        assert!(!dir.path().join("settings.json").exists());
        let next = Settings {
            theme_name: "next".to_owned(),
            ..Settings::default()
        };
        store.save(&next).unwrap();
        assert_eq!(SettingsStore::new_in(dir.path()).load().unwrap(), next);
    }

    #[test]
    fn round_trips_settings() {
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new_in(dir.path());
        let settings = Settings {
            theme_name: "storm".to_owned(),
            last_compare: Some(PersistedCompare {
                repo_path: Some("C:\\repo".into()),
                left_ref: "main".to_owned(),
                right_ref: "feature".to_owned(),
                mode: CompareMode::ThreeDot,
                layout: LayoutMode::Split,
                renderer: RendererKind::Semantic,
            }),
            ..Settings::default()
        };

        store.save(&settings).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded, settings);
    }
}
