use std::path::PathBuf;
use std::sync::Arc;

use diffy_ui::SourcePoint;

use crate::contracts::commands::ToastUpdate;
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Intent, Reduction};
use crate::contracts::viewport::files::{FileAt, head_revision};
use crate::contracts::viewport::symbols::{
    DefinitionTarget, IndexRequest, ResolveRequest, SourcePosition, SymbolsEffect, SymbolsEvent,
};
use crate::core::symbols::{Resolution, SymbolIndex};
use crate::core::vcs::git::is_full_hex_oid;
use crate::core::vcs::model::{RevisionId, VcsKind};

const HISTORY: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    Diff {
        scroll: f32,
        caret: Option<SourcePoint>,
    },
    File {
        file: FileAt,
        scroll: f32,
        caret: Option<SourcePoint>,
    },
}

#[derive(Debug, Default)]
pub struct SymbolsController {
    generation: u64,
    key: Option<(PathBuf, RevisionId)>,
    progress: Option<(usize, usize)>,
    index: Option<Arc<SymbolIndex>>,
    resolving: u64,
    jump: Option<DefinitionTarget>,
    back: Vec<Place>,
    forward: Vec<Place>,
}

impl SymbolsController {
    pub fn resolve(&mut self, position: SourcePosition, cx: &SharedContext) -> Reduction {
        let Some((repo_path, head)) = head_revision(cx) else {
            return Reduction::default();
        };
        self.resolving += 1;
        Reduction::default().effect(SymbolsEffect::Resolve(Box::new(ResolveRequest {
            generation: self.resolving,
            repo_path,
            position,
            head,
            index: self.index.clone(),
        })))
    }

    pub fn take_jump(&mut self) -> Option<DefinitionTarget> {
        self.jump.take()
    }

    pub fn record(&mut self, from: Place) {
        if self.back.last() != Some(&from) {
            self.back.push(from);
        }
        if self.back.len() > HISTORY {
            self.back.remove(0);
        }
        self.forward.clear();
    }

    pub fn back(&mut self, here: Option<Place>) -> Option<Place> {
        let to = self.back.pop()?;
        self.forward.extend(here);
        Some(to)
    }

    pub fn forward(&mut self, here: Option<Place>) -> Option<Place> {
        let to = self.forward.pop()?;
        self.back.extend(here);
        Some(to)
    }

    fn resolved(&mut self, result: Result<Resolution, String>) -> Reduction {
        let resolution = match result {
            Ok(resolution) => resolution,
            Err(message) => {
                return Reduction::default().intent(Intent::Toast(ToastUpdate::error(format!(
                    "Couldn't find the definition: {message}"
                ))));
            }
        };
        let word = resolution.word;
        let mut targets = resolution.targets;
        match targets.len() {
            0 if word.is_empty() => Reduction::default(),
            0 if resolution.at_definition => Reduction::default().intent(Intent::Toast(
                ToastUpdate::info(format!("{word} is defined here")),
            )),
            0 => Reduction::default().intent(Intent::Toast(ToastUpdate::info(format!(
                "No definition found for {word}"
            )))),
            1 => {
                self.jump = targets.pop();
                Reduction::changed()
            }
            _ => Reduction::default().intent(Intent::ChooseDefinition(targets)),
        }
    }

    pub fn index(&self) -> Option<&Arc<SymbolIndex>> {
        self.index.as_ref()
    }

    pub fn progress(&self) -> Option<(usize, usize)> {
        self.progress
    }

    fn refresh(&mut self, cx: &SharedContext, force: bool) -> Reduction {
        let Some(key) = head_revision(cx) else {
            return Reduction::default();
        };
        if key.1.backend != VcsKind::GIT {
            return Reduction::default();
        }
        let fresh = self.key.as_ref() == Some(&key) && is_full_hex_oid(&key.1.id);
        if fresh && !force {
            return Reduction::default();
        }
        self.generation += 1;
        self.key = Some(key.clone());
        Reduction::default().effect(SymbolsEffect::Index(Box::new(IndexRequest {
            generation: self.generation,
            repo_path: key.0,
            revision: key.1,
        })))
    }

    pub fn reduce_event(&mut self, event: SymbolsEvent) -> Reduction {
        match event {
            SymbolsEvent::Progress {
                generation,
                done,
                total,
            } if generation == self.generation => {
                let shown = self.progress.is_some();
                self.progress = (done < total).then_some((done, total));
                if shown || self.progress.is_some() {
                    Reduction::changed()
                } else {
                    Reduction::default()
                }
            }
            SymbolsEvent::Indexed { generation, index } if generation == self.generation => {
                self.progress = None;
                self.index = Some(index.clone());
                Reduction::changed().intent(Intent::PublishSymbols(Some(index)))
            }
            SymbolsEvent::Resolved { generation, result } if generation == self.resolving => {
                self.resolved(result)
            }
            SymbolsEvent::Failed {
                generation,
                message,
            } if generation == self.generation => {
                tracing::warn!(%message, "symbol indexing failed");
                self.progress = None;
                Reduction::changed()
            }
            _ => Reduction::default(),
        }
    }

    pub fn apply_intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        match intent {
            Intent::RepositoryChanged(_) | Intent::PublishManifest(_) => self.refresh(cx, false),
            Intent::GrammarAvailable(_) if self.index.as_ref().is_some_and(|i| i.pending()) => {
                self.refresh(cx, true)
            }
            Intent::ActivateWorkspace { .. } => {
                self.generation += 1;
                self.resolving += 1;
                self.key = None;
                self.progress = None;
                self.index = None;
                self.jump = None;
                self.back.clear();
                self.forward.clear();
                Reduction::changed()
            }
            _ => Reduction::default(),
        }
    }
}
