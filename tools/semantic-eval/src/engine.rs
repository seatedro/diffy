use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use std::path::Path;

use diffy::core::compare::semantic::{
    ChangedPath, document_from_changed, document_from_native_with,
};
use diffy::core::syntax::grammar::{legacy_storage_dir, trusted_keys};
use diffy_diff::{DEFAULT_CONTEXT, DiffDocument, FileStatus, diff_texts};
use diffy_syntax::{GrammarStore, LanguageId, LegacyPacks, StoreConfig, legacy_pack};
use serde::{Deserialize, Serialize};

use crate::capture;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Difftastic,
    Native,
    Line,
}

impl Engine {
    pub fn name(self) -> &'static str {
        match self {
            Self::Difftastic => "difftastic",
            Self::Native => "native",
            Self::Line => "line",
        }
    }
}

pub struct Run {
    pub doc: DiffDocument,
    pub micros: u64,
    pub fallback: Option<String>,
}

pub fn run(engine: Engine, path: &str, old: &[u8], new: &[u8]) -> Result<Run, String> {
    let changed = || ChangedPath {
        status: FileStatus::Modified,
        old_path: Some(path.to_owned()),
        new_path: Some(path.to_owned()),
        old_content: old.to_vec(),
        new_content: new.to_vec(),
    };
    match engine {
        Engine::Native => {
            let changed = changed();
            let start = Instant::now();
            let (doc, fallback) =
                document_from_native_with(&changed, language).map_err(|error| error.to_string())?;
            Ok(Run {
                doc,
                micros: start.elapsed().as_micros() as u64,
                fallback,
            })
        }
        Engine::Difftastic => {
            let changed = changed();
            capture::take_fallback();
            let start = Instant::now();
            let doc = document_from_changed(&changed).map_err(|error| error.to_string())?;
            let micros = start.elapsed().as_micros() as u64;
            Ok(Run {
                doc,
                micros,
                fallback: capture::take_fallback(),
            })
        }
        Engine::Line => {
            let (old, new) = (String::from_utf8_lossy(old), String::from_utf8_lossy(new));
            let start = Instant::now();
            let doc = diff_texts(
                Some(path),
                Some(path),
                Some(&old),
                Some(&new),
                DEFAULT_CONTEXT,
            );
            Ok(Run {
                doc,
                micros: start.elapsed().as_micros() as u64,
                fallback: None,
            })
        }
    }
}

fn language(name: &str) -> Option<diffy_semantic::Language> {
    let id = LanguageId::from_fence(name)?;
    grammars()
        .tree_sitter_language(&id)
        .or_else(|| direct(name))
}

fn direct(name: &str) -> Option<diffy_semantic::Language> {
    static LOADED: Mutex<Vec<(String, Option<diffy_semantic::Language>)>> = Mutex::new(Vec::new());
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, language)) = loaded.iter().find(|(n, _)| n == name) {
        return language.clone();
    }
    let language = legacy_storage_dir().and_then(|dir| load_pack(&dir.join(name), name));
    loaded.push((name.to_owned(), language.clone()));
    language
}

fn load_pack(dir: &Path, name: &str) -> Option<diffy_semantic::Language> {
    let version = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .max()?;
    let library = version.join(legacy_pack::platform()).join("parser.dylib");
    let symbol = format!("tree_sitter_{}", name.replace('-', "_"));
    unsafe {
        let library: &'static libloading::Library =
            Box::leak(Box::new(libloading::Library::new(library).ok()?));
        let function: libloading::Symbol<'static, unsafe extern "C" fn() -> *const ()> =
            library.get(symbol.as_bytes()).ok()?;
        Some(diffy_semantic::Language::new(
            tree_sitter_language::LanguageFn::from_raw(*function),
        ))
    }
}

fn grammars() -> &'static GrammarStore {
    static STORE: OnceLock<GrammarStore> = OnceLock::new();
    STORE.get_or_init(|| {
        let mut config = StoreConfig::new();
        if let Some(dir) = legacy_storage_dir() {
            config = config.legacy_packs(LegacyPacks::new(dir, &trusted_keys()));
        }
        GrammarStore::new(config)
    })
}
