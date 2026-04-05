use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use heed::types::{SerdeBincode, Str};
use heed::{Database, Env, EnvOpenOptions};
use serde::{Deserialize, Serialize};

const DECAY_CONSTANT: f64 = 0.0693;
const SECONDS_PER_DAY: f64 = 86400.0;
const MAP_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FrecencyEntry {
    access_count: u32,
    last_access_ms: u64,
}

#[derive(Clone)]
pub struct FrecencyStore {
    env: Env,
    db: Database<Str, SerdeBincode<FrecencyEntry>>,
}

impl std::fmt::Debug for FrecencyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrecencyStore").finish_non_exhaustive()
    }
}

impl FrecencyStore {
    pub fn open(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(path)?;
        let env = unsafe {
            let mut opts = EnvOpenOptions::new();
            opts.map_size(MAP_SIZE);
            opts.open(path)?
        };
        let mut wtxn = env.write_txn()?;
        let db = env.create_database(&mut wtxn, None)?;
        wtxn.commit()?;
        Ok(Self { env, db })
    }

    pub fn record_access(&self, key: &str) {
        let now_ms = now_ms();
        let result: Result<(), heed::Error> = (|| {
            let mut wtxn = self.env.write_txn()?;
            let entry = match self.db.get(&wtxn, key)? {
                Some(mut e) => {
                    e.access_count += 1;
                    e.last_access_ms = now_ms;
                    e
                }
                None => FrecencyEntry {
                    access_count: 1,
                    last_access_ms: now_ms,
                },
            };
            self.db.put(&mut wtxn, key, &entry)?;
            wtxn.commit()?;
            Ok(())
        })();
        if let Err(e) = result {
            log::warn!("frecency record_access failed: {e}");
        }
    }

    pub fn score(&self, key: &str) -> f64 {
        let entry = match (|| -> Result<Option<FrecencyEntry>, heed::Error> {
            let rtxn = self.env.read_txn()?;
            self.db.get(&rtxn, key).map_err(Into::into)
        })() {
            Ok(Some(e)) => e,
            _ => return 0.0,
        };
        let now = now_ms();
        let days_since = (now.saturating_sub(entry.last_access_ms) as f64) / 1000.0
            / SECONDS_PER_DAY;
        entry.access_count as f64 * (-DECAY_CONSTANT * days_since).exp()
    }

    pub fn recent(&self, limit: usize) -> Vec<String> {
        let entries = match (|| -> Result<Vec<(String, f64)>, heed::Error> {
            let rtxn = self.env.read_txn()?;
            let mut out = Vec::new();
            let iter = self.db.iter(&rtxn)?;
            let now = now_ms();
            for result in iter {
                let (key, entry) = result?;
                let days_since = (now.saturating_sub(entry.last_access_ms) as f64)
                    / 1000.0
                    / SECONDS_PER_DAY;
                let score =
                    entry.access_count as f64 * (-DECAY_CONSTANT * days_since).exp();
                if score > 0.001 {
                    out.push((key.to_owned(), score));
                }
            }
            Ok(out)
        })() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("frecency recent() failed: {e}");
                return Vec::new();
            }
        };
        let mut sorted = entries;
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(limit);
        sorted.into_iter().map(|(k, _)| k).collect()
    }

    pub fn remove(&self, key: &str) {
        let result: Result<(), heed::Error> = (|| {
            let mut wtxn = self.env.write_txn()?;
            self.db.delete(&mut wtxn, key)?;
            wtxn.commit()?;
            Ok(())
        })();
        if let Err(e) = result {
            log::warn!("frecency remove failed: {e}");
        }
    }
}

pub fn open_default_store() -> Option<FrecencyStore> {
    let base = dirs::config_dir()?;
    let path = base.join("diffy").join("frecency.mdb");
    match FrecencyStore::open(&path) {
        Ok(store) => Some(store),
        Err(e) => {
            log::warn!("failed to open frecency store at {}: {e}", path.display());
            None
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn recent_repo_paths(store: Option<&FrecencyStore>, limit: usize) -> Vec<PathBuf> {
    match store {
        Some(s) => s
            .recent(limit)
            .into_iter()
            .filter(|k| k.starts_with("repo:"))
            .map(|k| PathBuf::from(&k["repo:".len()..]))
            .collect(),
        None => Vec::new(),
    }
}
