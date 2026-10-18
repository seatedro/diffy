use std::collections::HashSet;
use std::path::Path;

use crate::core::error::{DiffyError, Result};
use crate::core::http;
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

    pub fn missing_grammar_for_path(&self, path: &str) -> Option<String> {
        let highlighter = phosphor::Highlighter::new();
        highlighter
            .guess_language(Path::new(path))
            .filter(|language| !highlighter.is_parser_available(*language))
            .map(|language| language.name().to_owned())
    }

    pub fn missing_grammars_for_paths(&self, paths: &[String]) -> Vec<String> {
        let highlighter = phosphor::Highlighter::new();
        let mut seen = HashSet::new();
        let mut languages = Vec::new();
        for path in paths {
            let Some(language) = highlighter
                .guess_language(Path::new(path))
                .filter(|language| !highlighter.is_parser_available(*language))
            else {
                continue;
            };
            if seen.insert(language) {
                languages.push(language.name().to_owned());
            }
        }
        languages
    }

    pub fn install_common_grammars(&self) -> Result<Vec<String>> {
        let installer = phosphor::PackInstaller::new()
            .map_err(|error| DiffyError::General(error.to_string()))?;
        http::block_on(async {
            installer
                .install_common_packs()
                .await
                .map(|languages| {
                    languages
                        .into_iter()
                        .map(|language| language.name().to_owned())
                        .collect()
                })
                .map_err(|error| DiffyError::General(error.to_string()))
        })
    }

    pub fn ensure_grammar_for_path(&self, path: &str) -> Result<Option<String>> {
        let installer = phosphor::PackInstaller::new()
            .map_err(|error| DiffyError::General(error.to_string()))?;
        http::block_on(async {
            installer
                .ensure_pack_for_path(Path::new(path))
                .await
                .map(|language| language.map(|language| language.name().to_owned()))
                .map_err(|error| DiffyError::General(error.to_string()))
        })
    }

    pub fn ensure_grammars_for_paths(&self, paths: &[String]) -> Result<Vec<String>> {
        let installer = phosphor::PackInstaller::new()
            .map_err(|error| DiffyError::General(error.to_string()))?;
        http::block_on(async {
            installer
                .ensure_packs_for_paths(paths.iter().map(|path| Path::new(path.as_str())))
                .await
                .map(|languages| {
                    languages
                        .into_iter()
                        .map(|language| language.name().to_owned())
                        .collect()
                })
                .map_err(|error| DiffyError::General(error.to_string()))
        })
    }
}
