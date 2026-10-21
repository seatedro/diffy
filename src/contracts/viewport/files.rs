use std::path::PathBuf;
use std::sync::Arc;

use diffy_diff::DiffDocument;

use crate::contracts::shell::{Action, AppEvent, Effect};
use crate::contracts::viewport::interaction::DiffInput;
use crate::contracts::viewport::{ViewportAction, ViewportEffect, ViewportEvent};
use crate::core::ref_catalog::WORKING_TREE_LABEL;
use crate::core::vcs::git::{INDEX_REF, WORKDIR_REF};
use crate::core::vcs::model::RevisionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

pub fn revision_label(id: &str) -> String {
    match id {
        WORKDIR_REF => WORKING_TREE_LABEL.to_owned(),
        INDEX_REF => "Index".to_owned(),
        id => id.chars().take(8).collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileAt {
    pub path: String,
    pub revision: RevisionId,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesAction {
    Open(FileAt),
    Activate(TabId),
    ShowDiff,
    Close(TabId),
    NextTab,
    PreviousTab,
    Input(DiffInput),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLoadRequest {
    pub tab: TabId,
    pub repo_path: PathBuf,
    pub file: FileAt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesEffect {
    Load(Box<FileLoadRequest>),
}

#[derive(Debug, Clone)]
pub enum FilesEvent {
    Loaded {
        tab: TabId,
        document: Arc<DiffDocument>,
    },
    Failed {
        tab: TabId,
        message: String,
    },
}

impl From<FilesAction> for ViewportAction {
    fn from(value: FilesAction) -> Self {
        Self::Files(value)
    }
}

impl From<FilesAction> for Action {
    fn from(value: FilesAction) -> Self {
        Self::Viewport(ViewportAction::Files(value))
    }
}

impl From<FilesEffect> for ViewportEffect {
    fn from(value: FilesEffect) -> Self {
        Self::Files(value)
    }
}

impl From<FilesEffect> for Effect {
    fn from(value: FilesEffect) -> Self {
        Self::Viewport(ViewportEffect::Files(value))
    }
}

impl From<FilesEvent> for ViewportEvent {
    fn from(value: FilesEvent) -> Self {
        Self::Files(value)
    }
}

impl From<FilesEvent> for AppEvent {
    fn from(value: FilesEvent) -> Self {
        Self::Viewport(ViewportEvent::Files(value))
    }
}
