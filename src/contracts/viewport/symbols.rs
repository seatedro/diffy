use std::path::PathBuf;
use std::sync::Arc;

use crate::contracts::shell::{Action, AppEvent, Effect};
use crate::contracts::viewport::{ViewportAction, ViewportEffect, ViewportEvent};
use crate::core::symbols::SymbolIndex;
pub use crate::core::symbols::{DefinitionTarget, Description, Resolution, SourcePosition};
use crate::core::vcs::model::RevisionId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolsAction {
    GoToDefinition,
    Jump(DefinitionTarget),
    Back,
    Forward,
}

#[derive(Debug, Clone)]
pub struct ResolveRequest {
    pub generation: u64,
    pub repo_path: PathBuf,
    pub position: SourcePosition,
    pub head: RevisionId,
    pub index: Option<Arc<SymbolIndex>>,
}

impl PartialEq for ResolveRequest {
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.repo_path == other.repo_path
            && self.position == other.position
            && self.head == other.head
            && match (&self.index, &other.index) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (a, b) => a.is_none() && b.is_none(),
            }
    }
}

impl Eq for ResolveRequest {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRequest {
    pub generation: u64,
    pub repo_path: PathBuf,
    pub revision: RevisionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolsEffect {
    Index(Box<IndexRequest>),
    Resolve(Box<ResolveRequest>),
    Describe(Box<ResolveRequest>),
}

#[derive(Debug, Clone)]
pub enum SymbolsEvent {
    Progress {
        generation: u64,
        done: usize,
        total: usize,
    },
    Indexed {
        generation: u64,
        index: Arc<SymbolIndex>,
    },
    Failed {
        generation: u64,
        message: String,
    },
    Resolved {
        generation: u64,
        result: Result<Resolution, String>,
    },
    Described {
        generation: u64,
        description: Option<Description>,
    },
}

impl From<SymbolsAction> for ViewportAction {
    fn from(value: SymbolsAction) -> Self {
        Self::Symbols(value)
    }
}

impl From<SymbolsAction> for Action {
    fn from(value: SymbolsAction) -> Self {
        Self::Viewport(ViewportAction::Symbols(value))
    }
}

impl From<SymbolsEffect> for ViewportEffect {
    fn from(value: SymbolsEffect) -> Self {
        Self::Symbols(value)
    }
}

impl From<SymbolsEffect> for Effect {
    fn from(value: SymbolsEffect) -> Self {
        Self::Viewport(ViewportEffect::Symbols(value))
    }
}

impl From<SymbolsEvent> for ViewportEvent {
    fn from(value: SymbolsEvent) -> Self {
        Self::Symbols(value)
    }
}

impl From<SymbolsEvent> for AppEvent {
    fn from(value: SymbolsEvent) -> Self {
        Self::Viewport(ViewportEvent::Symbols(value))
    }
}
