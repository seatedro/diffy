use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use diffy_syntax::legacy_pack::{self, LegacyLanguage};
use diffy_syntax::{
    GrammarStore, HighlightWorker, LegacyPacks, PublicKey, ScopeWorker, StoreConfig,
};

const PACK_INDEX_PUBLIC_KEY: &str = match option_env!("PHOSPHOR_PACK_INDEX_PUBLIC_KEY") {
    Some(key) => key,
    None => "1f3d2cb98f99da0321bdfa8549b4a43a36f96d7e25b710fb49739d46065b19f1",
};
const LOCAL_PACKS_ENV: &str = "DIFFY_SYNTAX_PACKS";

#[derive(Debug, Clone, Default)]
pub struct GrammarConfig {
    pub local_packs: Vec<PathBuf>,
    pub legacy: Option<LegacyPacks>,
}

impl GrammarConfig {
    pub fn product() -> Self {
        let local_packs = std::env::var_os(LOCAL_PACKS_ENV)
            .map(PathBuf::from)
            .into_iter()
            .collect();
        let legacy = legacy_storage_dir().map(|dir| {
            LegacyPacks::new(dir, &trusted_keys()).downloads(legacy_pack::default_index_url())
        });
        Self {
            local_packs,
            legacy,
        }
    }

    pub fn store_config(&self) -> StoreConfig {
        let mut config = StoreConfig::new();
        for root in &self.local_packs {
            config = config.local_packs(root);
        }
        if let Some(legacy) = &self.legacy {
            config = config.legacy_packs(legacy.clone());
        }
        config
    }
}

pub fn legacy_storage_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|base| base.join("diffy").join("phosphor").join("languages"))
}

pub fn trusted_keys() -> Vec<PublicKey> {
    PACK_INDEX_PUBLIC_KEY
        .split(',')
        .filter_map(|key| PublicKey::from_hex(key.trim()).ok())
        .collect()
}

pub fn languages() -> &'static [LegacyLanguage] {
    legacy_pack::LANGUAGES
}

pub fn common_languages() -> impl Iterator<Item = &'static LegacyLanguage> {
    legacy_pack::LANGUAGES
        .iter()
        .filter(|language| language.common)
}

pub fn language_for_path(path: &str) -> Option<&'static LegacyLanguage> {
    legacy_pack::language_for_path(path)
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
        SHARED.get_or_init(|| Grammars::new(&GrammarConfig::product()))
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
