use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use heed::types::{SerdeBincode, Str};
use heed::{Database, EnvOpenOptions};
use serde::{Deserialize, Serialize};

const DECAY_CONSTANT: f64 = 0.0693;
const SECONDS_PER_DAY: f64 = 86400.0;
const STORE_FILE_NAME: &str = "frecency.json";
const LEGACY_STORE_DIR_NAME: &str = "frecency.mdb";
const LEGACY_MAP_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FrecencyEntry {
    access_count: u32,
    last_access_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FrecencyData {
    entries: BTreeMap<String, FrecencyEntry>,
}

#[derive(Clone)]
pub struct FrecencyStore {
    path: Arc<PathBuf>,
    data: Arc<Mutex<FrecencyData>>,
}

impl std::fmt::Debug for FrecencyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrecencyStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl FrecencyStore {
    pub fn open(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let path = frecency_file_path(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let (data, migrated_legacy) = match std::fs::read(&path) {
            Ok(contents) => (
                serde_json::from_slice(&contents).unwrap_or_else(|error| {
                    tracing::warn!("corrupt frecency store at {}: {error}", path.display());
                    FrecencyData::default()
                }),
                false,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let legacy_path = legacy_frecency_dir_for_json_path(&path);
                match legacy_path.as_deref().and_then(|path| {
                    match read_legacy_frecency_store(path) {
                        Ok(data) => data,
                        Err(error) => {
                            tracing::warn!(
                                "failed to migrate legacy frecency store at {}: {error}",
                                path.display()
                            );
                            None
                        }
                    }
                }) {
                    Some(data) => (data, true),
                    None => (FrecencyData::default(), false),
                }
            }
            Err(error) => return Err(Box::new(error)),
        };
        let store = Self {
            path: Arc::new(path),
            data: Arc::new(Mutex::new(data)),
        };
        if migrated_legacy {
            let data = store.data.lock().ok().map(|data| data.clone());
            if let Some(data) = data {
                store.save(data, "legacy migration");
            }
        }
        Ok(store)
    }

    pub fn record_access(&self, key: &str) {
        let now_ms = now_ms();
        let Some(data) = self.update_data(|data| {
            let entry = data.entries.entry(key.to_owned()).or_insert(FrecencyEntry {
                access_count: 0,
                last_access_ms: now_ms,
            });
            entry.access_count = entry.access_count.saturating_add(1);
            entry.last_access_ms = now_ms;
        }) else {
            return;
        };
        self.save(data, "record_access");
    }

    pub fn score(&self, key: &str) -> f64 {
        let Ok(data) = self.data.lock() else {
            tracing::warn!("frecency score failed: store lock poisoned");
            return 0.0;
        };
        data.entries
            .get(key)
            .map(frecency_score)
            .unwrap_or_default()
    }

    pub fn recent(&self, limit: usize) -> Vec<String> {
        let entries = {
            let Ok(data) = self.data.lock() else {
                tracing::warn!("frecency recent failed: store lock poisoned");
                return Vec::new();
            };
            data.entries
                .iter()
                .map(|(key, entry)| (key.clone(), frecency_score(entry)))
                .filter(|(_, score)| *score > 0.001)
                .collect::<Vec<_>>()
        };

        let mut sorted = entries;
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(limit);
        sorted.into_iter().map(|(key, _)| key).collect()
    }

    pub fn remove(&self, key: &str) {
        let Some(data) = self.update_data(|data| {
            data.entries.remove(key);
        }) else {
            return;
        };
        self.save(data, "remove");
    }

    fn update_data(&self, update: impl FnOnce(&mut FrecencyData)) -> Option<FrecencyData> {
        let Ok(mut data) = self.data.lock() else {
            tracing::warn!("frecency update failed: store lock poisoned");
            return None;
        };
        update(&mut data);
        Some(data.clone())
    }

    fn save(&self, data: FrecencyData, operation: &str) {
        if let Err(error) = serde_json::to_vec(&data)
            .map_err(std::io::Error::other)
            .and_then(|contents| std::fs::write(self.path.as_ref(), contents))
        {
            tracing::warn!("frecency {operation} failed: {error}");
        }
    }
}

pub fn open_default_store() -> Option<FrecencyStore> {
    let base = dirs::config_dir()?;
    let path = base.join("diffy").join(STORE_FILE_NAME);
    match FrecencyStore::open(&path) {
        Ok(store) => Some(store),
        Err(error) => {
            tracing::warn!(
                "failed to open frecency store at {}: {error}",
                path.display()
            );
            None
        }
    }
}

fn frecency_file_path(path: &Path) -> PathBuf {
    if path.extension().is_none() {
        path.join(STORE_FILE_NAME)
    } else {
        path.to_path_buf()
    }
}

fn legacy_frecency_dir_for_json_path(path: &Path) -> Option<PathBuf> {
    (path.file_name()? == STORE_FILE_NAME).then(|| path.with_file_name(LEGACY_STORE_DIR_NAME))
}

fn read_legacy_frecency_store(
    path: &Path,
) -> Result<Option<FrecencyData>, Box<dyn std::error::Error>> {
    if !path.is_dir() {
        return Ok(None);
    }
    let env = unsafe {
        let mut opts = EnvOpenOptions::new();
        opts.map_size(LEGACY_MAP_SIZE);
        opts.open(path)?
    };
    let rtxn = env.read_txn()?;
    let Some(db): Option<Database<Str, SerdeBincode<FrecencyEntry>>> =
        env.open_database(&rtxn, None)?
    else {
        return Ok(None);
    };
    let mut data = FrecencyData::default();
    for result in db.iter(&rtxn)? {
        let (key, entry) = result?;
        data.entries.insert(key.to_owned(), entry);
    }
    Ok(Some(data))
}

fn frecency_score(entry: &FrecencyEntry) -> f64 {
    let now = now_ms();
    let days_since = (now.saturating_sub(entry.last_access_ms) as f64) / 1000.0 / SECONDS_PER_DAY;
    entry.access_count as f64 * (-DECAY_CONSTANT * days_since).exp()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn recent_repo_paths(store: Option<&FrecencyStore>, limit: usize) -> Vec<PathBuf> {
    match store {
        Some(store) => store
            .recent(limit)
            .into_iter()
            .filter(|key| key.starts_with("repo:"))
            .map(|key| PathBuf::from(&key["repo:".len()..]))
            .collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{FrecencyEntry, FrecencyStore, LEGACY_MAP_SIZE, recent_repo_paths};

    #[test]
    fn json_store_round_trips_recent_repos() {
        let dir = TempDir::new().unwrap();
        let store = FrecencyStore::open(dir.path()).unwrap();
        store.record_access("repo:/tmp/old");
        store.record_access("repo:/tmp/new");
        store.record_access("repo:/tmp/new");

        let reopened = FrecencyStore::open(dir.path()).unwrap();
        let recents = recent_repo_paths(Some(&reopened), 2);

        assert_eq!(recents[0], std::path::PathBuf::from("/tmp/new"));
        assert!(recents.contains(&std::path::PathBuf::from("/tmp/old")));
    }

    #[test]
    fn remove_deletes_key_from_json_store() {
        let dir = TempDir::new().unwrap();
        let store = FrecencyStore::open(dir.path()).unwrap();
        store.record_access("repo:/tmp/demo");
        store.remove("repo:/tmp/demo");

        let reopened = FrecencyStore::open(dir.path()).unwrap();
        assert!(recent_repo_paths(Some(&reopened), 1).is_empty());
    }

    #[test]
    fn migrates_legacy_lmdb_store_once() {
        let dir = TempDir::new().unwrap();
        let legacy_dir = dir.path().join(super::LEGACY_STORE_DIR_NAME);
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let env = unsafe {
            let mut opts = heed::EnvOpenOptions::new();
            opts.map_size(LEGACY_MAP_SIZE);
            opts.open(&legacy_dir).unwrap()
        };
        let mut wtxn = env.write_txn().unwrap();
        let db: heed::Database<heed::types::Str, heed::types::SerdeBincode<FrecencyEntry>> =
            env.create_database(&mut wtxn, None).unwrap();
        db.put(
            &mut wtxn,
            "repo:/tmp/legacy",
            &FrecencyEntry {
                access_count: 4,
                last_access_ms: super::now_ms(),
            },
        )
        .unwrap();
        wtxn.commit().unwrap();
        drop(env);

        let store = FrecencyStore::open(dir.path()).unwrap();
        let recents = recent_repo_paths(Some(&store), 1);

        assert_eq!(recents, vec![std::path::PathBuf::from("/tmp/legacy")]);
        assert!(dir.path().join(super::STORE_FILE_NAME).is_file());
    }
}
