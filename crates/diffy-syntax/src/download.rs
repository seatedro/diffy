//! Downloading packs from a signed index.
//!
//! The index is one JSON document per target (see [`crate::pack`]), signed
//! with Ed25519 like quark-update's manifests. The first lookup that no
//! local or cached pack answers starts a background thread, which fetches
//! the index once per store, saves it to the cache, and downloads the
//! asked-for pack next to it. Files are resumable and land only once their
//! SHA-256 matches the signed index; a pack is loaded only after the cached
//! index's signature and each file's SHA-256 check again.
//!
//! Cache layout: `<cache>/<target>/index.json` (as signed) and
//! `<cache>/<target>/<language>/<version>-<digest>/<file>`, where the
//! digest covers the SHA-256 of every file of the pack.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Weak};
use std::time::Duration;

use quark_update::PublicKey;

use crate::pack::{self, PackFile, PackManifest};
use crate::store::{Inner, State, Tag, load, lock};

/// Where an app's packs come from and are kept.
#[derive(Debug, Clone)]
pub struct Downloads {
    index_url: String,
    keys: Vec<[u8; 32]>,
    cache_dir: PathBuf,
}

impl Downloads {
    /// Packs listed in the index at `index_url` (`{target}` in it becomes
    /// this build's target triple), trusted when its signature verifies
    /// against one of `keys`, and cached under [`default_cache_dir`].
    pub fn new(app_id: &str, index_url: impl Into<String>, keys: &[PublicKey]) -> Self {
        Self {
            index_url: index_url.into().replace("{target}", pack::TARGET),
            keys: keys.iter().map(public_key_bytes).collect(),
            cache_dir: default_cache_dir(app_id),
        }
    }

    pub fn cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = dir.into();
        self
    }

    fn target_dir(&self) -> PathBuf {
        self.cache_dir.join(pack::TARGET)
    }

    fn index_path(&self) -> PathBuf {
        self.target_dir().join("index.json")
    }

    /// Keyed by content as well as version: a pack rebuilt under the same
    /// version gets its own directory, so a partial file of the old one is
    /// never resumed into the new one.
    fn pack_dir(&self, manifest: &PackManifest) -> PathBuf {
        let mut files = ring::digest::Context::new(&ring::digest::SHA256);
        for file in manifest.files() {
            files.update(file.sha256.to_ascii_lowercase().as_bytes());
            files.update(b"\n");
        }
        let digest = pack::hex(&files.finish().as_ref()[..8]);
        self.target_dir()
            .join(&manifest.language)
            .join(format!("{}-{digest}", manifest.version))
    }

    /// A file's URL: its own, or `<language>/<path>` next to the index.
    fn file_url(&self, manifest: &PackManifest, file: &PackFile) -> String {
        if let Some(url) = &file.url {
            return url.clone();
        }
        let base = self.index_url.rsplit_once('/').map_or("", |(base, _)| base);
        format!("{base}/{}/{}", manifest.language, file.path)
    }
}

fn public_key_bytes(key: &PublicKey) -> [u8; 32] {
    let bytes = pack::unhex(&key.to_hex()).unwrap_or_default();
    bytes.try_into().unwrap_or([0; 32])
}

/// `<data dir>/<app id>/syntax-packs`: `$XDG_DATA_HOME` or
/// `~/.local/share` on Linux, `~/Library/Application Support` on macOS,
/// `%LOCALAPPDATA%` on Windows, and the temp dir when none is set.
pub fn default_cache_dir(app_id: &str) -> PathBuf {
    let var = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let data = if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library").join("Application Support"))
    } else if cfg!(windows) {
        var("LOCALAPPDATA")
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|home| home.join(".local").join("share")))
    };
    data.unwrap_or_else(std::env::temp_dir)
        .join(app_id)
        .join("syntax-packs")
}

/// What the store knows of the index.
#[derive(Default)]
pub(crate) struct Remote {
    /// The cached index's packs: `None` until read, then the verified
    /// packs (empty when there is no valid cached index).
    cached: Option<Vec<Arc<PackManifest>>>,
    /// The fetch thread has started this session's one index fetch.
    started: bool,
    /// This session's index fetch finished (or failed). Until then a tag
    /// missing from `cached` may still be in the fresh index.
    fetched: bool,
}

fn find<'a>(packs: &'a [Arc<PackManifest>], tag: &str) -> Option<&'a Arc<PackManifest>> {
    packs
        .iter()
        .find(|pack| pack.tags().any(|t| t.eq_ignore_ascii_case(tag)))
}

/// Resolves `tag` from the cache, or queues it for the fetch thread.
pub(crate) fn resolve(
    inner: &Arc<Inner>,
    downloads: &Downloads,
    state: &mut State,
    tag: &str,
) -> Tag {
    let cached = state
        .remote
        .cached
        .get_or_insert_with(|| read_cached_index(downloads));
    let entry = find(cached, tag).cloned();
    if let Some(manifest) = &entry {
        let dir = downloads.pack_dir(manifest);
        // Files that are all present and intact decide the outcome: a pack
        // that then fails to load would fail again after a download. Missing
        // or corrupt files are (re)downloaded.
        if manifest.files().all(|f| dir.join(&f.path).is_file())
            && pack::verify_files(&dir, manifest).is_ok()
        {
            return load(state, &dir, manifest);
        }
    }
    // After this session's fetch the cached index is the fresh one, so a
    // tag it lacks is unknown.
    if state.remote.fetched && entry.is_none() {
        return Tag::Unavailable;
    }
    queue(inner, tag);
    Tag::Pending
}

fn read_cached_index(downloads: &Downloads) -> Vec<Arc<PackManifest>> {
    let Ok(bytes) = std::fs::read(downloads.index_path()) else {
        return Vec::new();
    };
    match pack::verify_index(&bytes, &downloads.keys, pack::TARGET) {
        Ok(index) => index.packs.into_iter().map(Arc::new).collect(),
        Err(error) => {
            tracing::warn!(%error, "ignoring cached syntax pack index");
            Vec::new()
        }
    }
}

fn queue(inner: &Arc<Inner>, tag: &str) {
    let mut fetcher = lock(&inner.fetcher);
    if let Some(sender) = &*fetcher
        && sender.send(tag.to_owned()).is_ok()
    {
        return;
    }
    let (sender, receiver) = channel();
    let _ = sender.send(tag.to_owned());
    let weak = Arc::downgrade(inner);
    let spawned = std::thread::Builder::new()
        .name("diffy-syntax-fetch".to_owned())
        .spawn(move || fetch_loop(&weak, &receiver));
    if spawned.is_ok() {
        *fetcher = Some(sender);
    } else {
        // Without the thread the tag stays pending forever; give up on it.
        drop(fetcher);
        lock(&inner.state)
            .tags
            .insert(tag.to_owned(), Tag::Unavailable);
    }
}

/// Serves queued tags until every store handle is gone, which drops the
/// sender and ends the loop.
fn fetch_loop(store: &Weak<Inner>, tags: &Receiver<String>) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(concat!("diffy-syntax/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    while let Ok(tag) = tags.recv() {
        let Some(inner) = store.upgrade() else {
            return;
        };
        let Some(downloads) = &inner.config.downloads else {
            return;
        };
        let outcome = fetch(&inner, downloads, &agent, &tag);
        {
            let mut state = lock(&inner.state);
            match outcome {
                // The next lookup loads it from the cache.
                Ok(()) => state.tags.remove(&tag),
                Err(error) => {
                    tracing::warn!(tag, %error, "syntax pack unavailable");
                    state.tags.insert(tag, Tag::Unavailable)
                }
            };
        }
        inner.notify();
    }
}

#[derive(Debug, thiserror::Error)]
enum FetchError {
    #[error("no pack for this language in the index")]
    NotListed,
    #[error("index download failed: {0}")]
    Index(String),
    #[error(transparent)]
    Pack(#[from] pack::PackError),
    #[error(transparent)]
    Download(#[from] quark_update::DownloadError),
}

fn fetch(
    inner: &Inner,
    downloads: &Downloads,
    agent: &ureq::Agent,
    tag: &str,
) -> Result<(), FetchError> {
    let first = !std::mem::replace(&mut lock(&inner.state).remote.started, true);
    if first {
        let fetched = fetch_index(downloads, agent);
        let mut state = lock(&inner.state);
        match fetched {
            Ok(packs) => state.remote.cached = Some(packs),
            // Keep whatever the cache had; the session does not retry.
            Err(error) => tracing::warn!(%error, "syntax pack index unavailable"),
        }
        state.remote.fetched = true;
    }
    let manifest = {
        let state = lock(&inner.state);
        let packs = state.remote.cached.as_deref().unwrap_or_default();
        find(packs, tag).cloned().ok_or(FetchError::NotListed)?
    };
    let dir = downloads.pack_dir(&manifest);
    for file in manifest.files() {
        // `download` ignores the format; it only describes update installers.
        let artifact = quark_update::Artifact {
            format: quark_update::Format::AppImage,
            url: downloads.file_url(&manifest, file),
            sha256: file.sha256.clone(),
            size: Some(file.size),
        };
        quark_update::download::download(agent, &artifact, &dir.join(&file.path), &mut |_| {})?;
    }
    Ok(())
}

/// Fetches, verifies, and caches the index.
fn fetch_index(
    downloads: &Downloads,
    agent: &ureq::Agent,
) -> Result<Vec<Arc<PackManifest>>, FetchError> {
    let index = |e: &dyn std::fmt::Display| FetchError::Index(e.to_string());
    let response = agent
        .get(&downloads.index_url)
        .call()
        .map_err(|e| index(&e))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(FetchError::Index(format!("HTTP {status}")));
    }
    let bytes = response
        .into_body()
        .with_config()
        .limit(8 << 20)
        .read_to_vec()
        .map_err(|e| index(&e))?;
    let verified = pack::verify_index(&bytes, &downloads.keys, pack::TARGET)?;
    for (language, error) in &verified.rejected {
        tracing::warn!(language, %error, "syntax pack index entry rejected");
    }
    write_atomically(&downloads.index_path(), &bytes).map_err(pack::PackError::from)?;
    Ok(verified.packs.into_iter().map(Arc::new).collect())
}

fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = path.with_extension("json.part");
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, path)
}
