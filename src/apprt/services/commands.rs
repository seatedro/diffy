use crate::core::error::Result;
use crate::platform::persistence::{Settings, SettingsStore};

#[derive(Debug, Clone)]
pub struct CommandsServices {
    settings_store: SettingsStore,
}

impl CommandsServices {
    pub fn new(settings_store: SettingsStore) -> Self {
        Self { settings_store }
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.settings_store.save(settings)
    }
}
