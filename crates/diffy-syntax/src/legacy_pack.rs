use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Weak};
use std::time::Duration;

use quark_update::PublicKey;
use serde::Deserialize;

use crate::engine::Grammar;
use crate::pack::{self, PackError};
use crate::store::{Inner, State, Tag, lock};

pub const DEFAULT_INDEX_BASE_URL: &str = "https://blob.diffygui.com/phosphor-index";
pub const MANIFEST: &str = "manifest.json";
pub const SCHEMA: u32 = 1;

const INDEX_DIR: &str = ".index";
const INDEX_LIMIT: u64 = 32 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyLanguage {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub common: bool,
}

const fn language_entry(
    name: &'static str,
    extensions: &'static [&'static str],
    common: bool,
) -> LegacyLanguage {
    LegacyLanguage {
        name,
        extensions,
        common,
    }
}

pub const LANGUAGES: &[LegacyLanguage] = &[
    language_entry("bash", &["bash", "sh", "zsh"], true),
    language_entry("c", &["c", "h"], true),
    language_entry("cpp", &["cc", "cpp", "cxx", "hh", "hpp", "hxx"], true),
    language_entry("go", &["go"], true),
    language_entry("javascript", &["js", "jsx", "mjs"], true),
    language_entry("json", &["json"], true),
    language_entry("nix", &["nix"], false),
    language_entry("python", &["py", "pyi"], true),
    language_entry("rust", &["rs"], true),
    language_entry("toml", &["toml"], true),
    language_entry("typescript", &["ts"], true),
    language_entry("tsx", &["tsx"], true),
    language_entry("zig", &["zig"], false),
];

pub fn language(name: &str) -> Option<&'static LegacyLanguage> {
    LANGUAGES
        .iter()
        .find(|language| language.name.eq_ignore_ascii_case(name))
}

pub fn language_for_extension(extension: &str) -> Option<&'static LegacyLanguage> {
    if extension.eq_ignore_ascii_case("js") || extension.eq_ignore_ascii_case("mjs") {
        return language("typescript");
    }
    if extension.eq_ignore_ascii_case("jsx") {
        return language("tsx");
    }
    LANGUAGES.iter().find(|language| {
        language
            .extensions
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(extension))
    })
}

pub fn language_for_tag(tag: &str) -> Option<&'static LegacyLanguage> {
    language(tag).or_else(|| language_for_extension(tag))
}

pub fn language_for_path(path: &str) -> Option<&'static LegacyLanguage> {
    let name = path.rsplit(['/', '\\']).next()?;
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    language_for_extension(extension)
}

pub fn platform() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x86_64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-aarch64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "macos-x86_64"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x86_64"
    } else {
        "unknown"
    }
}

pub fn expected_abi() -> u32 {
    u32::try_from(tree_sitter::LANGUAGE_VERSION).unwrap_or(u32::MAX)
}

pub fn library_suffix(platform: &str) -> &'static str {
    if platform.starts_with("windows-") {
        ".dll"
    } else if platform.starts_with("macos-") {
        ".dylib"
    } else {
        ".so"
    }
}

pub fn default_index_url() -> String {
    format!("{DEFAULT_INDEX_BASE_URL}/{}.json", platform())
}

#[derive(Debug, Clone)]
pub struct LegacyPacks {
    storage_dir: PathBuf,
    keys: Vec<[u8; 32]>,
    index_url: Option<String>,
}

impl LegacyPacks {
    pub fn new(storage_dir: impl Into<PathBuf>, keys: &[PublicKey]) -> Self {
        Self {
            storage_dir: storage_dir.into(),
            keys: keys.iter().map(crate::download::public_key_bytes).collect(),
            index_url: None,
        }
    }

    pub fn downloads(mut self, index_url: impl Into<String>) -> Self {
        self.index_url = Some(index_url.into().replace("{platform}", platform()));
        self
    }

    pub fn storage_dir(&self) -> &Path {
        &self.storage_dir
    }

    pub fn downloads_enabled(&self) -> bool {
        self.index_url.is_some() && !self.keys.is_empty()
    }

    fn index_path(&self) -> PathBuf {
        self.storage_dir
            .join(INDEX_DIR)
            .join(format!("{}.json", platform()))
    }

    fn pack_dir(&self, entry: &LegacyEntry) -> PathBuf {
        self.storage_dir
            .join(&entry.language)
            .join(&entry.version)
            .join(platform())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LegacyIndex {
    pub schema_version: u32,
    pub platform: String,
    pub tree_sitter_abi: u32,
    pub packs: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LegacyEntry {
    pub language: String,
    pub version: String,
    #[serde(default)]
    pub common: bool,
    pub symbol: String,
    pub manifest: RemoteFile,
    pub parser: RemoteFile,
    pub highlights: RemoteFile,
    #[serde(default)]
    pub injections: Option<RemoteFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RemoteFile {
    pub url: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LegacyManifest {
    pub schema_version: u32,
    pub language: String,
    pub version: String,
    pub platform: String,
    pub tree_sitter_abi: u32,
    pub symbol: String,
    pub parser: LocalFile,
    pub highlights: LocalFile,
    #[serde(default)]
    pub injections: Option<LocalFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LocalFile {
    pub path: String,
    pub sha256: String,
}

impl LegacyEntry {
    fn files(&self) -> impl Iterator<Item = &RemoteFile> {
        [&self.parser, &self.highlights]
            .into_iter()
            .chain(self.injections.as_ref())
    }

    fn validate(&self, platform: &str) -> Result<(), PackError> {
        if language(&self.language).is_none() {
            return Err(pack::unsafe_field("language", &self.language));
        }
        pack::check_segment("version", &self.version)?;
        if !pack::is_c_identifier(&self.symbol) {
            return Err(pack::unsafe_field("symbol", &self.symbol));
        }
        if self.manifest.path != MANIFEST {
            return Err(pack::unsafe_field("manifest.path", &self.manifest.path));
        }
        check_remote("manifest", &self.manifest)?;
        check_remote("parser", &self.parser)?;
        if !self.parser.path.ends_with(library_suffix(platform)) {
            return Err(pack::unsafe_field("parser.path", &self.parser.path));
        }
        check_remote("highlights", &self.highlights)?;
        if let Some(injections) = &self.injections {
            check_remote("injections", injections)?;
        }
        Ok(())
    }
}

fn check_remote(field: &'static str, file: &RemoteFile) -> Result<(), PackError> {
    pack::check_segment(field, &file.path)?;
    if !is_sha256(&file.sha256) {
        return Err(pack::unsafe_field(field, &file.sha256));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn same_file(local: &LocalFile, remote: &RemoteFile) -> bool {
    local.path == remote.path && local.sha256.eq_ignore_ascii_case(&remote.sha256)
}

#[derive(Debug)]
pub struct VerifiedLegacyIndex {
    pub packs: Vec<LegacyEntry>,
    pub rejected: Vec<(String, PackError)>,
}

pub fn verify_legacy_index(
    bytes: &[u8],
    keys: &[[u8; 32]],
    platform: &str,
) -> Result<VerifiedLegacyIndex, PackError> {
    let payload = pack::verify_signed(bytes, keys)?;
    let index: LegacyIndex =
        serde_json::from_value(payload).map_err(|e| PackError::Malformed(e.to_string()))?;
    if index.schema_version != SCHEMA {
        return Err(PackError::Schema(index.schema_version));
    }
    if index.platform != platform {
        return Err(PackError::WrongTarget {
            expected: platform.to_owned(),
            found: index.platform,
        });
    }
    let abi = expected_abi();
    if index.tree_sitter_abi != abi {
        return Err(PackError::Abi {
            abi: index.tree_sitter_abi,
            min: abi,
            max: abi,
        });
    }
    let mut verified = VerifiedLegacyIndex {
        packs: Vec::new(),
        rejected: Vec::new(),
    };
    for value in index.packs {
        let name = value
            .get("language")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if language(&name).is_none() {
            continue;
        }
        let entry = match serde_json::from_value::<LegacyEntry>(value) {
            Ok(entry) => entry,
            Err(error) => {
                verified
                    .rejected
                    .push((name, PackError::Malformed(error.to_string())));
                continue;
            }
        };
        match entry.validate(platform) {
            Ok(()) => verified.packs.push(entry),
            Err(error) => verified.rejected.push((name, error)),
        }
    }
    Ok(verified)
}

pub fn read_built_manifest(dir: &Path, platform: &str) -> Result<LegacyManifest, PackError> {
    let bytes = std::fs::read(dir.join(MANIFEST))?;
    let manifest: LegacyManifest =
        serde_json::from_slice(&bytes).map_err(|e| PackError::Malformed(e.to_string()))?;
    if manifest.schema_version != SCHEMA {
        return Err(PackError::Schema(manifest.schema_version));
    }
    if manifest.platform != platform {
        return Err(PackError::WrongTarget {
            expected: platform.to_owned(),
            found: manifest.platform,
        });
    }
    pack::check_segment("language", &manifest.language)?;
    pack::check_segment("version", &manifest.version)?;
    if !pack::is_c_identifier(&manifest.symbol) {
        return Err(pack::unsafe_field("symbol", &manifest.symbol));
    }
    let files = [&manifest.parser, &manifest.highlights]
        .into_iter()
        .chain(manifest.injections.as_ref());
    for file in files {
        pack::check_segment("path", &file.path)?;
        let path = dir.join(&file.path);
        if pack::sha256_file(&path)? != file.sha256.to_ascii_lowercase() {
            return Err(PackError::ShaMismatch { path });
        }
    }
    if !manifest.parser.path.ends_with(library_suffix(platform)) {
        return Err(pack::unsafe_field("parser.path", &manifest.parser.path));
    }
    Ok(manifest)
}

pub fn check_built_pack(
    dir: &Path,
    typescript: Option<&Path>,
) -> Result<LegacyManifest, PackError> {
    let manifest = read_built_manifest(dir, platform())?;
    if manifest.tree_sitter_abi != expected_abi() {
        let abi = expected_abi();
        return Err(PackError::Abi {
            abi: manifest.tree_sitter_abi,
            min: abi,
            max: abi,
        });
    }
    let own = std::fs::read_to_string(dir.join(&manifest.highlights.path))?;
    let inherited = match (manifest.language.as_str(), typescript) {
        ("tsx", Some(base)) => {
            let base_manifest = read_built_manifest(base, platform())?;
            let text = std::fs::read_to_string(base.join(&base_manifest.highlights.path))?;
            Some(compose_highlights(&text, None))
        }
        _ => None,
    };
    let injections = match &manifest.injections {
        Some(file) => Some(std::fs::read_to_string(dir.join(&file.path))?),
        None => None,
    };
    Grammar::compile(
        &dir.join(&manifest.parser.path),
        &manifest.symbol,
        None,
        &compose_highlights(&own, inherited.as_deref()),
        injections.as_deref(),
    )?;
    Ok(manifest)
}

pub fn verify_installed(dir: &Path, entry: &LegacyEntry) -> Result<LegacyManifest, PackError> {
    let manifest_path = dir.join(MANIFEST);
    let bytes = std::fs::read(&manifest_path)?;
    if sha256_hex(&bytes) != entry.manifest.sha256.to_ascii_lowercase() {
        return Err(PackError::ShaMismatch {
            path: manifest_path,
        });
    }
    let manifest: LegacyManifest =
        serde_json::from_slice(&bytes).map_err(|e| PackError::Malformed(e.to_string()))?;
    let consistent = manifest.schema_version == SCHEMA
        && manifest.language == entry.language
        && manifest.version == entry.version
        && manifest.platform == platform()
        && manifest.tree_sitter_abi == expected_abi()
        && manifest.symbol == entry.symbol
        && same_file(&manifest.parser, &entry.parser)
        && same_file(&manifest.highlights, &entry.highlights)
        && match (&manifest.injections, &entry.injections) {
            (Some(local), Some(remote)) => same_file(local, remote),
            (None, None) => true,
            _ => false,
        };
    if !consistent {
        return Err(pack::unsafe_field("manifest", &entry.language));
    }
    for file in entry.files() {
        let path = dir.join(&file.path);
        if pack::sha256_file(&path)? != file.sha256.to_ascii_lowercase() {
            return Err(PackError::ShaMismatch { path });
        }
    }
    Ok(manifest)
}

fn sha256_hex(bytes: &[u8]) -> String {
    pack::hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrammarState {
    Ready,
    Installed,
    Downloading,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarStatus {
    pub language: &'static str,
    pub state: GrammarState,
}

#[derive(Default)]
pub(crate) struct LegacyState {
    index: Option<Arc<[LegacyEntry]>>,
    fetched: bool,
    index_error: Option<String>,
    pub(crate) status: Statuses,
}

#[derive(Default)]
pub(crate) struct Statuses {
    states: HashMap<&'static str, GrammarState>,
    pub(crate) revision: u64,
}

impl Statuses {
    fn get(&self, language: &str) -> Option<&GrammarState> {
        self.states.get(language)
    }

    fn insert(&mut self, language: &'static str, state: GrammarState) {
        if self.states.get(language) != Some(&state) {
            self.states.insert(language, state);
            self.revision += 1;
        }
    }
}

pub(crate) fn status(legacy: &LegacyPacks, state: &mut State) -> Vec<GrammarStatus> {
    let entries = state
        .legacy
        .index
        .get_or_insert_with(|| read_cached_index(legacy))
        .clone();
    LANGUAGES
        .iter()
        .filter_map(|language| {
            let found = state
                .legacy
                .status
                .get(language.name)
                .cloned()
                .or_else(|| {
                    find(&entries, language.name)
                        .filter(|entry| legacy.pack_dir(entry).join(MANIFEST).is_file())
                        .map(|_| GrammarState::Installed)
                })?;
            Some(GrammarStatus {
                language: language.name,
                state: found,
            })
        })
        .collect()
}

fn find<'a>(entries: &'a [LegacyEntry], name: &str) -> Option<&'a LegacyEntry> {
    entries.iter().find(|entry| entry.language == name)
}

fn dependencies(language: &'static str) -> impl Iterator<Item = &'static str> {
    (language == "tsx")
        .then_some("typescript")
        .into_iter()
        .chain([language])
}

struct InstalledPack {
    dir: PathBuf,
    library: PathBuf,
    symbol: String,
    highlights: String,
    injections: Option<String>,
}

impl InstalledPack {
    fn compile(&self) -> Result<Grammar, PackError> {
        match Grammar::compile(
            &self.library,
            &self.symbol,
            None,
            &self.highlights,
            self.injections.as_deref(),
        ) {
            Err(PackError::InjectionQuery(error)) => {
                tracing::warn!(pack = %self.dir.display(), %error, "legacy injection query skipped");
                Grammar::compile(&self.library, &self.symbol, None, &self.highlights, None)
            }
            other => other,
        }
    }
}

fn installed(
    legacy: &LegacyPacks,
    entries: &[LegacyEntry],
    language: &'static LegacyLanguage,
) -> Result<InstalledPack, PackError> {
    let missing = || pack::unsafe_field("language", language.name);
    let entry = find(entries, language.name).ok_or_else(missing)?;
    let dir = legacy.pack_dir(entry);
    let manifest = verify_installed(&dir, entry)?;
    let own = std::fs::read_to_string(dir.join(&manifest.highlights.path))?;
    let typescript = if language.name == "tsx" {
        let base = find(entries, "typescript").ok_or_else(missing)?;
        let base_dir = legacy.pack_dir(base);
        let base_manifest = verify_installed(&base_dir, base)?;
        let text = std::fs::read_to_string(base_dir.join(&base_manifest.highlights.path))?;
        Some(compose_highlights(&text, None))
    } else {
        None
    };
    let injections = match &manifest.injections {
        Some(file) => Some(std::fs::read_to_string(dir.join(&file.path))?),
        None => None,
    };
    Ok(InstalledPack {
        library: dir.join(&manifest.parser.path),
        symbol: manifest.symbol,
        highlights: compose_highlights(&own, typescript.as_deref()),
        injections,
        dir,
    })
}

pub fn compose_highlights(highlights: &str, typescript: Option<&str>) -> String {
    let mut out = String::new();
    for inherited in inherited_queries(highlights) {
        match inherited {
            "ecma" => {
                out.push_str(ECMA_HIGHLIGHTS);
                out.push('\n');
            }
            "typescript" => {
                if let Some(typescript) = typescript {
                    out.push_str(typescript);
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
    out.push_str(highlights);
    out
}

fn inherited_queries(query: &str) -> Vec<&str> {
    query
        .lines()
        .take_while(|line| line.trim_start().starts_with(';'))
        .filter_map(|line| line.trim_start().strip_prefix("; inherits:"))
        .flat_map(|names| names.split(','))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect()
}

pub(crate) fn resolve(
    inner: &Arc<Inner>,
    legacy: &LegacyPacks,
    state: &mut State,
    language: &'static LegacyLanguage,
) -> Tag {
    match state.legacy.status.get(language.name) {
        Some(GrammarState::Failed(_)) => return Tag::Unavailable,
        Some(GrammarState::Downloading) => return Tag::Pending,
        _ => {}
    }
    let entries = state
        .legacy
        .index
        .get_or_insert_with(|| read_cached_index(legacy))
        .clone();
    match installed(legacy, &entries, language) {
        Ok(pack) => load(state, language, &pack),
        Err(error) => {
            if !legacy.downloads_enabled() {
                tracing::debug!(language = language.name, %error, "legacy syntax pack not installed");
                return Tag::Unavailable;
            }
            state
                .legacy
                .status
                .insert(language.name, GrammarState::Downloading);
            queue(inner, language.name);
            Tag::Pending
        }
    }
}

fn load(state: &mut State, language: &'static LegacyLanguage, pack: &InstalledPack) -> Tag {
    let grammar = match state.loaded.get(&pack.dir) {
        Some(found) => found.clone(),
        None => {
            let compiled = match pack.compile() {
                Ok(grammar) => Some(Arc::new(grammar)),
                Err(error) => {
                    tracing::warn!(pack = %pack.dir.display(), %error, "legacy syntax pack failed to load");
                    let reason = error.to_string();
                    let reason = reason.lines().next().unwrap_or_default();
                    state.legacy.status.insert(
                        language.name,
                        GrammarState::Failed(format!(
                            "the published grammar does not load: {reason}"
                        )),
                    );
                    None
                }
            };
            state.loaded.insert(pack.dir.clone(), compiled.clone());
            compiled
        }
    };
    match grammar {
        Some(grammar) => {
            state
                .legacy
                .status
                .insert(language.name, GrammarState::Ready);
            Tag::Ready(grammar)
        }
        None => Tag::Unavailable,
    }
}

pub(crate) fn retry(
    legacy: &LegacyPacks,
    state: &mut State,
    language: &'static LegacyLanguage,
) -> bool {
    if !legacy.downloads_enabled()
        || !matches!(
            state.legacy.status.get(language.name),
            Some(GrammarState::Failed(_))
        )
    {
        return false;
    }
    state
        .legacy
        .status
        .insert(language.name, GrammarState::Downloading);
    state.legacy.fetched = false;
    state.legacy.index_error = None;
    state.legacy.index = None;
    forget_tags(state, language.name);
    state
        .loaded
        .retain(|dir, grammar| grammar.is_some() || !is_pack_of(dir, language.name));
    true
}

fn is_pack_of(dir: &Path, language: &str) -> bool {
    dir.parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .is_some_and(|name| name == language)
}

fn forget_tags(state: &mut State, language: &str) {
    state
        .tags
        .retain(|tag, _| language_for_tag(tag).is_none_or(|found| found.name != language));
}

fn read_cached_index(legacy: &LegacyPacks) -> Arc<[LegacyEntry]> {
    let Ok(bytes) = std::fs::read(legacy.index_path()) else {
        return Arc::from([]);
    };
    match verify_legacy_index(&bytes, &legacy.keys, platform()) {
        Ok(index) => index.packs.into(),
        Err(error) => {
            tracing::warn!(%error, "ignoring cached legacy syntax pack index");
            Arc::from([])
        }
    }
}

pub(crate) fn queue(inner: &Arc<Inner>, language: &'static str) {
    let mut fetcher = lock(&inner.legacy_fetcher);
    if let Some(sender) = &*fetcher
        && sender.send(language).is_ok()
    {
        return;
    }
    let (sender, receiver) = channel();
    let _ = sender.send(language);
    let weak = Arc::downgrade(inner);
    let spawned = std::thread::Builder::new()
        .name("diffy-syntax-legacy-fetch".to_owned())
        .spawn(move || fetch_loop(&weak, &receiver));
    if spawned.is_ok() {
        *fetcher = Some(sender);
    } else {
        drop(fetcher);
        lock(&inner.state).legacy.status.insert(
            language,
            GrammarState::Failed("the download thread could not start".to_owned()),
        );
    }
}

fn fetch_loop(store: &Weak<Inner>, languages: &Receiver<&'static str>) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(concat!("diffy-syntax/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    while let Ok(language) = languages.recv() {
        let Some(inner) = store.upgrade() else {
            return;
        };
        let Some(legacy) = &inner.config.legacy else {
            return;
        };
        let outcome = fetch(&inner, legacy, &agent, language);
        let installed = outcome.is_ok();
        {
            let mut state = lock(&inner.state);
            match outcome {
                Ok(()) => {
                    for name in dependencies(language) {
                        state.legacy.status.insert(name, GrammarState::Installed);
                        forget_tags(&mut state, name);
                    }
                }
                Err(error) => {
                    tracing::warn!(language, %error, "legacy syntax pack unavailable");
                    state
                        .legacy
                        .status
                        .insert(language, GrammarState::Failed(error.to_string()));
                    forget_tags(&mut state, language);
                }
            }
        }
        if installed {
            for name in dependencies(language) {
                inner.lookup(name);
            }
        }
        inner.notify();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("not published for {0}")]
    NotListed(&'static str),
    #[error("index download failed: {0}")]
    Index(String),
    #[error(transparent)]
    Pack(#[from] PackError),
    #[error("pack download failed: {0}")]
    Download(String),
}

impl From<quark_update::DownloadError> for FetchError {
    fn from(error: quark_update::DownloadError) -> Self {
        use quark_update::DownloadError;
        Self::Download(match error {
            DownloadError::Http(error) => error.to_string(),
            DownloadError::Status(status) => format!("HTTP {status}"),
            DownloadError::Incomplete { received, expected } => {
                format!("stopped after {received} of {expected} bytes")
            }
            DownloadError::ShaMismatch { .. } => "a file failed SHA-256 verification".to_owned(),
            DownloadError::Io(error) => error.to_string(),
        })
    }
}

fn fetch(
    inner: &Inner,
    legacy: &LegacyPacks,
    agent: &ureq::Agent,
    language: &'static str,
) -> Result<(), FetchError> {
    let first = !std::mem::replace(&mut lock(&inner.state).legacy.fetched, true);
    if first {
        let fetched = fetch_index(legacy, agent);
        let mut state = lock(&inner.state);
        match fetched {
            Ok(entries) => {
                state.legacy.index = Some(entries.into());
                state.legacy.index_error = None;
            }
            Err(error) => {
                tracing::warn!(%error, "legacy syntax pack index unavailable");
                if state.legacy.index.is_none() {
                    state.legacy.index = Some(read_cached_index(legacy));
                }
                state.legacy.index_error = Some(error.to_string());
            }
        }
    }
    let (entries, index_error) = {
        let state = lock(&inner.state);
        (
            state.legacy.index.clone().unwrap_or_else(|| Arc::from([])),
            state.legacy.index_error.clone(),
        )
    };
    for name in dependencies(language) {
        let Some(entry) = find(&entries, name) else {
            return Err(match index_error {
                Some(error) => FetchError::Index(error),
                None => FetchError::NotListed(platform()),
            });
        };
        let dir = legacy.pack_dir(entry);
        if verify_installed(&dir, entry).is_ok() {
            continue;
        }
        install(agent, entry, &dir)?;
        prune(legacy, entry);
    }
    Ok(())
}

fn install(agent: &ureq::Agent, entry: &LegacyEntry, dir: &Path) -> Result<(), FetchError> {
    std::fs::create_dir_all(dir).map_err(PackError::from)?;
    match std::fs::remove_file(dir.join(MANIFEST)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(PackError::from(error).into());
        }
        _ => {}
    }
    for file in entry.files().chain([&entry.manifest]) {
        let artifact = quark_update::Artifact {
            format: quark_update::Format::AppImage,
            url: file.url.clone(),
            sha256: file.sha256.to_ascii_lowercase(),
            size: None,
        };
        quark_update::download::download(agent, &artifact, &dir.join(&file.path), &mut |_| {})?;
    }
    verify_installed(dir, entry)?;
    Ok(())
}

fn prune(legacy: &LegacyPacks, entry: &LegacyEntry) {
    let language_dir = legacy.storage_dir.join(&entry.language);
    let Ok(versions) = std::fs::read_dir(&language_dir) else {
        return;
    };
    for version in versions.flatten() {
        if version.file_name() == entry.version.as_str()
            || !version.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        let stale = version.path().join(platform());
        if stale.is_dir()
            && let Err(error) = std::fs::remove_dir_all(&stale)
        {
            tracing::debug!(path = %stale.display(), %error, "stale legacy syntax pack kept");
            continue;
        }
        let _ = std::fs::remove_dir(version.path());
    }
}

fn fetch_index(legacy: &LegacyPacks, agent: &ureq::Agent) -> Result<Vec<LegacyEntry>, FetchError> {
    let Some(url) = &legacy.index_url else {
        return Err(FetchError::Index("downloads are not configured".to_owned()));
    };
    let index = |e: &dyn std::fmt::Display| FetchError::Index(e.to_string());
    let response = agent.get(url).call().map_err(|e| index(&e))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(FetchError::Index(format!("HTTP {status}")));
    }
    let bytes = response
        .into_body()
        .with_config()
        .limit(INDEX_LIMIT)
        .read_to_vec()
        .map_err(|e| index(&e))?;
    let verified = verify_legacy_index(&bytes, &legacy.keys, platform())?;
    for (language, error) in &verified.rejected {
        tracing::warn!(language, %error, "legacy syntax pack index entry rejected");
    }
    crate::download::write_atomically(&legacy.index_path(), &bytes).map_err(PackError::from)?;
    Ok(verified.packs)
}

pub const ECMA_HIGHLIGHTS: &str = r#"
; Base JavaScript/ECMAScript captures used by javascript/typescript queries.
(identifier) @variable

[
  (property_identifier)
  (shorthand_property_identifier)
  (private_property_identifier)
] @variable.member

(object_pattern
  (shorthand_property_identifier_pattern) @variable)

(object_pattern
  (object_assignment_pattern
    (shorthand_property_identifier_pattern) @variable))

(function_expression
  name: (identifier) @function)

(function_declaration
  name: (identifier) @function)

(generator_function
  name: (identifier) @function)

(generator_function_declaration
  name: (identifier) @function)

(method_definition
  name: [
    (property_identifier)
    (private_property_identifier)
  ] @function.method)

(pair
  key: (property_identifier) @function.method
  value: (function_expression))

(pair
  key: (property_identifier) @function.method
  value: (arrow_function))

(variable_declarator
  name: (identifier) @function
  value: (arrow_function))

(variable_declarator
  name: (identifier) @function
  value: (function_expression))

(assignment_expression
  left: (identifier) @function
  right: (arrow_function))

(assignment_expression
  left: (identifier) @function
  right: (function_expression))

(assignment_expression
  left: (member_expression
    property: [
      (property_identifier)
      (private_property_identifier)
    ] @function.method)
  right: (arrow_function))

(assignment_expression
  left: (member_expression
    property: [
      (property_identifier)
      (private_property_identifier)
    ] @function.method)
  right: (function_expression))

(member_expression
  property: [
    (property_identifier)
    (private_property_identifier)
  ] @variable.member)

(call_expression
  function: (identifier) @function.call)

(call_expression
  function: (member_expression
    property: [
      (property_identifier)
      (private_property_identifier)
    ] @function.method.call))

(new_expression
  constructor: (identifier) @constructor)

(decorator
  "@" @attribute
  (identifier) @attribute)

(decorator
  "@" @attribute
  (call_expression
    (identifier) @attribute))

(decorator
  "@" @attribute
  (member_expression
    (property_identifier) @attribute))

[
  "import"
  "from"
  "export"
  "default"
  "const"
  "let"
  "var"
  "function"
  "return"
  "if"
  "else"
  "for"
  "while"
  "do"
  "switch"
  "case"
  "break"
  "continue"
  "class"
  "extends"
  "new"
  "try"
  "catch"
  "finally"
  "throw"
  "await"
  "async"
] @keyword

(string) @string
(template_string) @string
(number) @number
(comment) @comment

[
  ";"
  "."
  ","
  ":"
] @punctuation.delimiter

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  "--"
  "-"
  "-="
  "&&"
  "+"
  "++"
  "+="
  "&="
  "/="
  "**="
  "<<="
  "<"
  "<="
  "<<"
  "="
  "=="
  "==="
  "!="
  "!=="
  "=>"
  ">"
  ">="
  ">>"
  "||"
  "%"
  "%="
  "*"
  "**"
  ">>>"
  "&"
  "|"
  "^"
  "??"
  "*="
  ">>="
  ">>>="
  "^="
  "|="
  "&&="
  "||="
  "??="
  "~"
  "!"
  "/"
] @operator
"#;
