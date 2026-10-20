use std::sync::Arc;

use diffy_syntax::{Catalog, GrammarStore, LanguageId};

use crate::core::error::Result;
use crate::core::syntax::Grammars;
use crate::core::update::{self, AvailableUpdate, StagedUpdate, UpdateCheck};

#[derive(Debug, Clone, Copy, Default)]
pub struct DeliveryServices;

impl DeliveryServices {
    pub fn check_for_updates(&self, current_version: &str) -> Result<UpdateCheck> {
        update::check_for_update(current_version)
    }

    pub fn stage_update(&self, update: &AvailableUpdate) -> Result<StagedUpdate> {
        update::download_and_stage(update)
    }

    pub fn apply_staged_update(&self, staged: &StagedUpdate) -> Result<()> {
        update::apply_staged_update(staged)
    }

    pub fn grammar_store(&self) -> GrammarStore {
        Grammars::shared().store()
    }
}

pub fn grammar_status(store: &GrammarStore) -> Arc<Catalog> {
    Arc::new(store.catalog())
}

pub fn ensure_grammars(store: &GrammarStore, languages: &[impl AsRef<str>]) {
    for language in languages {
        if let Some(tag) = LanguageId::from_fence(language.as_ref()) {
            store.status(&tag);
        }
    }
}

pub fn retry_grammar(store: &GrammarStore, language: &str) {
    if store.retry(language) {
        ensure_grammars(store, &[language]);
    }
}
