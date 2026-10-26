use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use diffy_syntax::{Downloads, GrammarStore, HighlightWorker, PublicKey, ScopeWorker, StoreConfig};

const PACK_INDEX_PUBLIC_KEY: &str = match option_env!("PHOSPHOR_PACK_INDEX_PUBLIC_KEY") {
    Some(key) => key,
    None => "1f3d2cb98f99da0321bdfa8549b4a43a36f96d7e25b710fb49739d46065b19f1",
};
const PACK_INDEX_URL: &str = "https://blob.diffygui.com/syntax-packs/v1/{target}/index.json";
const APP_ID: &str = "diffy";
const LOCAL_PACKS_ENV: &str = "DIFFY_SYNTAX_PACKS";

pub const COMMON_LANGUAGES: &[&str] = &[
    "bash",
    "c",
    "cpp",
    "go",
    "javascript",
    "json",
    "python",
    "rust",
    "toml",
    "typescript",
    "tsx",
];

#[derive(Debug, Clone, Default)]
pub struct GrammarConfig {
    pub local_packs: Vec<PathBuf>,
    pub downloads: Option<Downloads>,
}

impl GrammarConfig {
    pub fn product() -> Self {
        let local_packs = std::env::var_os(LOCAL_PACKS_ENV)
            .map(PathBuf::from)
            .into_iter()
            .collect();
        Self {
            local_packs,
            downloads: Some(Downloads::new(APP_ID, PACK_INDEX_URL, &trusted_keys())),
        }
    }

    pub fn store_config(&self) -> StoreConfig {
        let mut config = StoreConfig::new();
        for root in &self.local_packs {
            config = config.local_packs(root);
        }
        if let Some(downloads) = &self.downloads {
            config = config.downloads(downloads.clone());
        }
        config
    }
}

pub fn trusted_keys() -> Vec<PublicKey> {
    PACK_INDEX_PUBLIC_KEY
        .split(',')
        .filter_map(|key| PublicKey::from_hex(key.trim()).ok())
        .collect()
}

#[derive(Clone)]
pub struct Grammars {
    store: GrammarStore,
    worker: Arc<Mutex<HighlightWorker>>,
    scopes: Arc<Mutex<ScopeWorker>>,
}

impl std::fmt::Debug for Grammars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Grammars")
    }
}

impl Grammars {
    pub fn new(config: &GrammarConfig) -> Self {
        Self::with_store(GrammarStore::new(config.store_config()))
    }

    pub fn none() -> Self {
        Self::with_store(GrammarStore::none())
    }

    pub fn with_store(store: GrammarStore) -> Self {
        Self {
            worker: Arc::new(Mutex::new(HighlightWorker::new(store.clone()))),
            scopes: Arc::new(Mutex::new(ScopeWorker::new(store.clone()))),
            store,
        }
    }

    pub fn shared() -> &'static Grammars {
        static SHARED: OnceLock<Grammars> = OnceLock::new();
        SHARED.get_or_init(|| {
            // Tests read the packs `syntax-pack build` writes, never the
            // network or the user's pack cache.
            #[cfg(test)]
            return Grammars::with_store(GrammarStore::new(
                StoreConfig::new().local_packs(diffy_syntax::testing::pack_root()),
            ));
            #[cfg(not(test))]
            Grammars::new(&GrammarConfig::product())
        })
    }

    pub fn store(&self) -> GrammarStore {
        self.store.clone()
    }

    pub fn worker(&self) -> HighlightWorker {
        self.worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .share()
    }

    pub fn worker_waking(&self, wake: impl Fn() + Send + Sync + 'static) -> HighlightWorker {
        let worker = self.worker();
        worker.set_wake(wake);
        worker
    }

    pub fn scope_worker(&self) -> ScopeWorker {
        self.scopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .share()
    }
}
