use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::core::error::{DiffyError, Result};
use crate::ui::theme::ThemeMode;

const SETTINGS_FILE_NAME: &str = "settings.json";
const MAX_RECENT_REPOS: usize = 10;

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
pub struct Settings {
    pub theme_mode: ThemeMode,
    pub theme_name: String,
    pub recent_repos: Vec<PathBuf>,
    pub last_compare: Option<PersistedCompare>,
    pub viewport: PersistedViewport,
    pub github_token: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::Dark,
            theme_name: "diffy-default".to_owned(),
            recent_repos: Vec::new(),
            last_compare: None,
            viewport: PersistedViewport::default(),
            github_token: None,
        }
    }
}

impl Settings {
    pub fn remember_repo(&mut self, path: &Path) {
        let normalized = path.to_path_buf();
        self.recent_repos.retain(|entry| entry != &normalized);
        self.recent_repos.insert(0, normalized);
        if self.recent_repos.len() > MAX_RECENT_REPOS {
            self.recent_repos.truncate(MAX_RECENT_REPOS);
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new_default() -> Self {
        let base = dirs::config_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        Self::new_in(base.join("diffy"))
    }

    pub fn new_in(path: impl Into<PathBuf>) -> Self {
        let mut path = path.into();
        if path.extension().is_none() {
            path = path.join(SETTINGS_FILE_NAME);
        }
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Settings> {
        if !self.path.exists() {
            return Ok(Settings::default());
        }

        let contents = fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&contents)?)
    }

    pub fn save(&self, settings: &Settings) -> Result<()> {
        let parent = self.path.parent().ok_or_else(|| {
            DiffyError::General(format!(
                "settings path has no parent directory: {}",
                self.path.display()
            ))
        })?;
        fs::create_dir_all(parent)?;
        fs::write(&self.path, serde_json::to_vec_pretty(settings)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::{PersistedCompare, Settings, SettingsStore};
    use crate::core::compare::{CompareMode, LayoutMode, RendererKind};

    #[test]
    fn remembers_recent_repositories_without_duplicates() {
        let mut settings = Settings::default();
        settings.remember_repo(Path::new("C:\\repo-one"));
        settings.remember_repo(Path::new("C:\\repo-two"));
        settings.remember_repo(Path::new("C:\\repo-one"));

        assert_eq!(settings.recent_repos.len(), 2);
        assert_eq!(settings.recent_repos[0], PathBuf::from("C:\\repo-one"));
    }

    #[test]
    fn round_trips_settings_json() {
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new_in(dir.path());
        let settings = Settings {
            theme_name: "storm".to_owned(),
            github_token: Some("secret".to_owned()),
            last_compare: Some(PersistedCompare {
                repo_path: Some("C:\\repo".into()),
                left_ref: "main".to_owned(),
                right_ref: "feature".to_owned(),
                mode: CompareMode::ThreeDot,
                layout: LayoutMode::Split,
                renderer: RendererKind::Difftastic,
            }),
            ..Settings::default()
        };

        store.save(&settings).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded, settings);
    }
}
