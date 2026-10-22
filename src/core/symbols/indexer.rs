use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use diffy_syntax::{GrammarStore, LanguageId, Symbol, Tagger, TaggerLookup};

use super::cache::{BlobKey, SymbolCache};
use super::index::{FileSymbols, SymbolIndex};
use crate::core::error::Result;
use crate::core::vcs::git::{GitService, TreeBlob};

const MAX_FILES: usize = 100_000;
const MAX_SOURCE: usize = 1 << 20;
const BINARY_SNIFF: usize = 8 << 10;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

pub struct Indexer {
    store: GrammarStore,
    cache: Option<SymbolCache>,
    memo: HashMap<BlobKey, Arc<[Symbol]>>,
}

struct Job {
    slot: usize,
    key: BlobKey,
    tagger: Tagger,
    source: String,
}

impl Indexer {
    pub fn new(store: GrammarStore, cache: Option<SymbolCache>) -> Self {
        Self {
            store,
            cache,
            memo: HashMap::new(),
        }
    }

    pub fn run(
        &mut self,
        repo_path: &Path,
        revision: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<Option<SymbolIndex>> {
        let git = GitService::new_with_open(repo_path)?;
        let blobs = git.list_blobs_at(revision)?;
        let mut taggers: HashMap<String, Option<Tagger>> = HashMap::new();
        let mut pending = false;
        let mut entries: Vec<(TreeBlob, Tagger)> = Vec::new();
        for blob in blobs {
            let Some(language) = LanguageId::from_path(&blob.path) else {
                continue;
            };
            let tagger = taggers
                .entry(language.as_str().to_owned())
                .or_insert_with(|| match self.store.tagger(&language) {
                    TaggerLookup::Ready(tagger) => Some(tagger),
                    TaggerLookup::Pending => {
                        pending = true;
                        None
                    }
                    TaggerLookup::Unsupported => None,
                });
            if let Some(tagger) = tagger {
                entries.push((blob, tagger.clone()));
            }
            if entries.len() == MAX_FILES {
                tracing::warn!(limit = MAX_FILES, "symbol index truncated");
                break;
            }
        }
        if cancelled() {
            return Ok(None);
        }
        let total = entries.len();
        let now = unix_now();
        let mut slots: Vec<Option<Arc<[Symbol]>>> = vec![None; total];
        let mut loaded: HashMap<usize, Vec<u8>> = HashMap::new();
        let keys: Vec<Option<BlobKey>> = entries
            .iter()
            .enumerate()
            .map(|(slot, (blob, tagger))| {
                let id = match blob.oid {
                    Some(oid) => oid_bytes(&oid),
                    None => {
                        let bytes = std::fs::read(repo_path.join(&blob.path)).ok()?;
                        let id = git_blob_id(&bytes);
                        loaded.insert(slot, bytes);
                        id
                    }
                };
                Some((id, tagger.fingerprint()))
            })
            .collect();
        let mut lookups = Vec::new();
        for (slot, key) in keys.iter().enumerate() {
            match key.and_then(|key| self.memo.get(&key)) {
                Some(symbols) => slots[slot] = Some(symbols.clone()),
                None if key.is_some() => lookups.push(slot),
                None => {}
            }
        }
        if let Some(cache) = &self.cache {
            let wanted: Vec<BlobKey> = lookups.iter().filter_map(|slot| keys[*slot]).collect();
            match cache.get_many(&wanted, now) {
                Ok(found) => {
                    for (slot, symbols) in lookups.iter().zip(found) {
                        slots[*slot] = symbols.map(Arc::from);
                    }
                }
                Err(error) => tracing::warn!(%error, "symbol cache read failed"),
            }
        }
        let misses: Vec<usize> = (0..total)
            .filter(|slot| slots[*slot].is_none() && keys[*slot].is_some())
            .collect();
        let mut done = total - misses.len();
        progress(done, total);
        let parsed = parse_misses(
            &git,
            &entries,
            &misses,
            &keys,
            loaded,
            cancelled,
            &mut |parsed| progress(done + parsed, total),
        )?;
        let Some(parsed) = parsed else {
            return Ok(None);
        };
        done = total;
        progress(done, total);
        if let Some(cache) = &mut self.cache
            && !parsed.is_empty()
        {
            let rows: Vec<(BlobKey, &[Symbol])> = parsed
                .iter()
                .map(|(_, key, symbols)| (*key, &**symbols))
                .collect();
            if let Err(error) = cache.put_many(&rows, now).and_then(|()| cache.prune(now)) {
                tracing::warn!(%error, "symbol cache write failed");
            }
        }
        for (slot, _, symbols) in parsed {
            slots[slot] = Some(symbols);
        }
        let mut memo = HashMap::with_capacity(total);
        let mut files = Vec::with_capacity(total);
        for ((blob, _), (symbols, key)) in entries.into_iter().zip(slots.into_iter().zip(keys)) {
            let Some(symbols) = symbols else {
                continue;
            };
            if let Some(key) = key {
                memo.insert(key, symbols.clone());
            }
            if !symbols.is_empty() {
                files.push(FileSymbols {
                    path: Arc::from(blob.path),
                    symbols,
                });
            }
        }
        self.memo = memo;
        Ok(Some(SymbolIndex::new(files, pending)))
    }
}

type Parsed = Vec<(usize, BlobKey, Arc<[Symbol]>)>;

fn parse_misses(
    git: &GitService,
    entries: &[(TreeBlob, Tagger)],
    misses: &[usize],
    keys: &[Option<BlobKey>],
    mut loaded: HashMap<usize, Vec<u8>>,
    cancelled: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(usize),
) -> Result<Option<Parsed>> {
    if misses.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let workers = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .saturating_sub(1)
        .clamp(1, 8);
    let parsed = AtomicUsize::new(0);
    let (jobs, queue) = sync_channel::<Job>(workers * 4);
    let queue = Mutex::new(queue);
    let (results, collected) = channel();
    let finished = std::thread::scope(|scope| {
        for _ in 0..workers {
            let results = results.clone();
            let queue = &queue;
            let parsed = &parsed;
            scope.spawn(move || {
                crate::platform::energy::background_thread();
                let _work = crate::platform::energy::work(crate::platform::energy::Kind::Index);
                loop {
                    let job = queue.lock().unwrap_or_else(PoisonError::into_inner).recv();
                    let Ok(job) = job else {
                        break;
                    };
                    let symbols: Arc<[Symbol]> =
                        Arc::from(job.tagger.symbols(&job.source, cancelled));
                    parsed.fetch_add(1, Ordering::Relaxed);
                    if results.send((job.slot, job.key, symbols)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(results);
        let mut last = Instant::now();
        for &slot in misses {
            if cancelled() {
                break;
            }
            let (blob, tagger) = &entries[slot];
            let bytes = match (loaded.remove(&slot), blob.oid) {
                (Some(bytes), _) => Some(bytes),
                (None, Some(oid)) => git.read_blob(oid).ok(),
                (None, None) => None,
            };
            let (Some(bytes), Some(key)) = (bytes, keys[slot]) else {
                continue;
            };
            let source = source_text(bytes).unwrap_or_default();
            let job = Job {
                slot,
                key,
                tagger: tagger.clone(),
                source,
            };
            if jobs.send(job).is_err() {
                break;
            }
            if last.elapsed() >= PROGRESS_INTERVAL {
                last = Instant::now();
                progress(parsed.load(Ordering::Relaxed));
            }
        }
        drop(jobs);
        let mut out = Vec::with_capacity(misses.len());
        for result in collected.iter() {
            out.push(result);
            if last.elapsed() >= PROGRESS_INTERVAL {
                last = Instant::now();
                progress(parsed.load(Ordering::Relaxed));
            }
        }
        out
    });
    if cancelled() {
        return Ok(None);
    }
    Ok(Some(finished))
}

fn source_text(bytes: Vec<u8>) -> Option<String> {
    if bytes.len() > MAX_SOURCE || bytes[..bytes.len().min(BINARY_SNIFF)].contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn oid_bytes(oid: &gix::ObjectId) -> [u8; 20] {
    let mut out = [0; 20];
    let bytes = oid.as_bytes();
    let len = bytes.len().min(20);
    out[..len].copy_from_slice(&bytes[..len]);
    out
}

fn git_blob_id(bytes: &[u8]) -> [u8; 20] {
    let mut context = ring::digest::Context::new(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY);
    context.update(format!("blob {}\0", bytes.len()).as_bytes());
    context.update(bytes);
    let digest = context.finish();
    let mut out = [0; 20];
    out.copy_from_slice(digest.as_ref());
    out
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

pub fn open_cache() -> Option<SymbolCache> {
    if cfg!(test) {
        return None;
    }
    let path = crate::platform::startup::app_cache_dir()?.join("symbols.db");
    match SymbolCache::open(&path) {
        Ok(cache) => Some(cache),
        Err(error) => {
            tracing::warn!(%error, "symbol cache unavailable");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_content_like_git() {
        let hex: String = git_blob_id(b"hello\n")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(hex, "ce013625030ba8dba906f756967f9e9ca394464a");
    }
}
