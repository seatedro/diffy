use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use heed::types::{SerdeBincode, Str};
use heed::{Database as LmdbDatabase, EnvOpenOptions};
use sea_query::{Expr, ExprTrait, OnConflict, Query};
use serde::{Deserialize, Serialize};

use crate::core::error::Result;
use crate::platform::database::{Database, Frecency};

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

#[derive(Debug, Clone)]
pub struct FrecencyStore {
    db: Database,
}

impl FrecencyStore {
    pub fn open(path: &Path) -> Result<Self> {
        let dir = store_dir(path);
        let db = Database::in_dir(&dir)?;
        db.import_once("frecency", |db| import_legacy(db, &dir));
        Ok(Self { db })
    }

    pub fn record_access(&self, key: &str) {
        let now = now_ms() as i64;
        let insert = Query::insert()
            .into_table(Frecency::Table)
            .columns([Frecency::Key, Frecency::AccessCount, Frecency::LastAccessMs])
            .values_panic([key.into(), 1.into(), now.into()])
            .on_conflict(
                OnConflict::column(Frecency::Key)
                    .value(
                        Frecency::AccessCount,
                        Expr::col(Frecency::AccessCount).add(1),
                    )
                    .update_column(Frecency::LastAccessMs)
                    .to_owned(),
            )
            .to_owned();
        if let Err(error) = self.db.execute(&insert) {
            tracing::warn!("frecency record_access failed: {error}");
        }
    }

    pub fn score(&self, key: &str) -> f64 {
        self.entries(Some(key))
            .first()
            .map(|(_, entry)| frecency_score(entry))
            .unwrap_or_default()
    }

    pub fn recent(&self, limit: usize) -> Vec<String> {
        let mut scored: Vec<(String, f64)> = self
            .entries(None)
            .into_iter()
            .map(|(key, entry)| {
                let score = frecency_score(&entry);
                (key, score)
            })
            .filter(|(_, score)| *score > 0.001)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored.into_iter().map(|(key, _)| key).collect()
    }

    pub fn last_access_secs(&self, key: &str) -> Option<i64> {
        let (_, entry) = self.entries(Some(key)).into_iter().next()?;
        i64::try_from(entry.last_access_ms / 1000).ok()
    }

    pub fn remove(&self, key: &str) {
        let delete = Query::delete()
            .from_table(Frecency::Table)
            .and_where(Expr::col(Frecency::Key).eq(key))
            .to_owned();
        if let Err(error) = self.db.execute(&delete) {
            tracing::warn!("frecency remove failed: {error}");
        }
    }

    fn entries(&self, key: Option<&str>) -> Vec<(String, FrecencyEntry)> {
        let mut select = Query::select()
            .columns([Frecency::Key, Frecency::AccessCount, Frecency::LastAccessMs])
            .from(Frecency::Table)
            .to_owned();
        if let Some(key) = key {
            select.and_where(Expr::col(Frecency::Key).eq(key));
        }
        let rows = self.db.query(&select, |row| {
            Ok((
                row.get::<_, String>(0)?,
                FrecencyEntry {
                    access_count: row.get::<_, u32>(1)?,
                    last_access_ms: u64::try_from(row.get::<_, i64>(2)?).unwrap_or_default(),
                },
            ))
        });
        rows.unwrap_or_else(|error| {
            tracing::warn!("frecency read failed: {error}");
            Vec::new()
        })
    }
}

pub fn open_default_store() -> Option<FrecencyStore> {
    let base = crate::platform::startup::app_config_dir()?;
    match FrecencyStore::open(&base) {
        Ok(store) => Some(store),
        Err(error) => {
            tracing::warn!(
                "failed to open frecency store in {}: {error}",
                base.display()
            );
            None
        }
    }
}

fn store_dir(path: &Path) -> PathBuf {
    if path.extension().is_none() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    }
}

fn import_legacy(db: &Database, dir: &Path) -> Result<()> {
    let json = dir.join(STORE_FILE_NAME);
    let data = match std::fs::read(&json) {
        Ok(contents) => serde_json::from_slice(&contents).unwrap_or_else(|error| {
            tracing::warn!("corrupt frecency store at {}: {error}", json.display());
            FrecencyData::default()
        }),
        Err(_) => match legacy_frecency_dir_for_json_path(&json)
            .as_deref()
            .map(read_legacy_frecency_store)
        {
            Some(Ok(Some(data))) => data,
            Some(Err(error)) => {
                tracing::warn!("failed to read legacy frecency store: {error}");
                FrecencyData::default()
            }
            _ => FrecencyData::default(),
        },
    };
    for (key, entry) in data.entries {
        db.execute(
            &Query::insert()
                .into_table(Frecency::Table)
                .columns([Frecency::Key, Frecency::AccessCount, Frecency::LastAccessMs])
                .values_panic([
                    key.into(),
                    entry.access_count.into(),
                    (entry.last_access_ms as i64).into(),
                ])
                .on_conflict(OnConflict::column(Frecency::Key).do_nothing().to_owned())
                .to_owned(),
        )?;
    }
    Ok(())
}

fn legacy_frecency_dir_for_json_path(path: &Path) -> Option<PathBuf> {
    (path.file_name()? == STORE_FILE_NAME).then(|| path.with_file_name(LEGACY_STORE_DIR_NAME))
}

fn read_legacy_frecency_store(
    path: &Path,
) -> std::result::Result<Option<FrecencyData>, Box<dyn std::error::Error>> {
    if !path.is_dir() {
        return Ok(None);
    }
    let env = unsafe {
        let mut opts = EnvOpenOptions::new();
        opts.map_size(LEGACY_MAP_SIZE);
        opts.open(path)?
    };
    let rtxn = env.read_txn()?;
    let Some(db): Option<LmdbDatabase<Str, SerdeBincode<FrecencyEntry>>> =
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

pub fn repo_last_opened(store: Option<&FrecencyStore>, path: &Path) -> Option<i64> {
    store?.last_access_secs(&format!("repo:{}", path.display()))
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
    fn store_round_trips_recent_repos() {
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
    fn imports_the_json_store_once() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(super::STORE_FILE_NAME),
            format!(
                r#"{{"entries":{{"repo:/tmp/json":{{"access_count":2,"last_access_ms":{}}}}}}}"#,
                super::now_ms()
            ),
        )
        .unwrap();
        let store = FrecencyStore::open(dir.path()).unwrap();
        assert_eq!(
            recent_repo_paths(Some(&store), 1),
            vec![std::path::PathBuf::from("/tmp/json")]
        );
    }

    #[test]
    fn remove_deletes_key() {
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
        assert!(
            dir.path()
                .join(crate::platform::database::DATABASE_FILE_NAME)
                .is_file()
        );
    }
}
