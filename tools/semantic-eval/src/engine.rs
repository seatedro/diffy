use std::sync::OnceLock;
use std::time::Instant;

use diffy::core::compare::semantic::{
    ChangedPath, document_from_difftastic, document_from_native_with,
};
use diffy::core::syntax::grammar::GrammarConfig;
use diffy_diff::{DEFAULT_CONTEXT, DiffDocument, FileStatus, diff_texts};
use diffy_syntax::{GrammarStore, LanguageId};
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
            let doc = document_from_difftastic(&changed).map_err(|error| error.to_string())?;
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

pub fn language(name: &str) -> Option<diffy_semantic::Language> {
    grammars().tree_sitter_language(&LanguageId::from_fence(name)?)
}

fn grammars() -> &'static GrammarStore {
    static STORE: OnceLock<GrammarStore> = OnceLock::new();
    STORE.get_or_init(|| GrammarStore::new(GrammarConfig::product().store_config()))
}
