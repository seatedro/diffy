use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::core::error::{DiffyError, Result};
use crate::core::review::{ReviewSession, ReviewSessionKey, ReviewTarget};
use crate::platform::persistence::SettingsStore;

const REVIEW_STORE_FILE_NAME: &str = "review-sessions.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
struct ReviewStoreData {
    sessions: BTreeMap<String, ReviewSession>,
}

#[derive(Clone)]
pub struct ReviewStore {
    path: Arc<PathBuf>,
    data: Arc<Mutex<ReviewStoreData>>,
}

impl std::fmt::Debug for ReviewStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReviewStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl ReviewStore {
    pub fn new_default() -> Self {
        let base = dirs::config_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        Self::open_or_default(base.join("diffy").join(REVIEW_STORE_FILE_NAME))
    }

    pub fn for_settings_store(settings_store: &SettingsStore) -> Self {
        let path = settings_store
            .path()
            .parent()
            .map(|parent| parent.join(REVIEW_STORE_FILE_NAME))
            .unwrap_or_else(|| PathBuf::from(REVIEW_STORE_FILE_NAME));
        Self::open_or_default(path)
    }

    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = review_store_file_path(path.into());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = match fs::read(&path) {
            Ok(contents) => serde_json::from_slice(&contents).unwrap_or_else(|error| {
                tracing::warn!("corrupt review store at {}: {error}", path.display());
                ReviewStoreData::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ReviewStoreData::default()
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path: Arc::new(path),
            data: Arc::new(Mutex::new(data)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_session(
        &self,
        target: &ReviewTarget,
        head_sha: &str,
    ) -> Result<Option<ReviewSession>> {
        let key = ReviewSessionKey::new(target.clone(), head_sha.to_owned()).storage_key();
        let data = self.lock_data()?;
        Ok(data.sessions.get(&key).cloned())
    }

    pub fn save_session(&self, session: &ReviewSession) -> Result<()> {
        let mut data = self.lock_data()?;
        data.sessions
            .insert(session.key().storage_key(), session.clone());
        self.save_locked(&data)
    }

    pub fn remove_session(&self, key: &ReviewSessionKey) -> Result<()> {
        let mut data = self.lock_data()?;
        data.sessions.remove(&key.storage_key());
        self.save_locked(&data)
    }

    fn open_or_default(path: PathBuf) -> Self {
        match Self::open(&path) {
            Ok(store) => store,
            Err(error) => {
                tracing::warn!("failed to open review store at {}: {error}", path.display());
                Self {
                    path: Arc::new(path),
                    data: Arc::new(Mutex::new(ReviewStoreData::default())),
                }
            }
        }
    }

    fn lock_data(&self) -> Result<std::sync::MutexGuard<'_, ReviewStoreData>> {
        self.data
            .lock()
            .map_err(|_| DiffyError::General("review store lock poisoned".to_owned()))
    }

    fn save_locked(&self, data: &ReviewStoreData) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(self.path.as_ref(), serde_json::to_vec_pretty(data)?)?;
        Ok(())
    }
}

fn review_store_file_path(path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.join(REVIEW_STORE_FILE_NAME)
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::ReviewStore;
    use crate::core::forge::github::PullRequestInfo;
    use crate::core::review::{ReviewSession, ReviewTarget};

    #[test]
    fn review_sessions_round_trip_by_head_sha() {
        let dir = TempDir::new().unwrap();
        let store = ReviewStore::open(dir.path()).unwrap();
        let target = ReviewTarget::github("owner", "repo", 7);
        let session = ReviewSession::new(
            target.clone(),
            PullRequestInfo {
                number: 7,
                head_sha: "abc123".to_owned(),
                ..PullRequestInfo::default()
            },
        );

        store.save_session(&session).unwrap();
        let reopened = ReviewStore::open(dir.path()).unwrap();
        assert_eq!(
            reopened.load_session(&target, "abc123").unwrap(),
            Some(session)
        );
        assert!(
            reopened
                .load_session(&target, "different")
                .unwrap()
                .is_none()
        );
    }
}
