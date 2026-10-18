use std::path::PathBuf;
use std::sync::Arc;

use diffy_diff::{DiffDocument, FileDiffSnapshot, FileStatus, SourcePurpose};

use crate::contracts::shared::{CompareGeneration, FileKey, FileRevision};
use crate::contracts::shell::{Action, AppEvent, Effect};
use crate::contracts::viewport::{ViewportAction, ViewportEffect, ViewportEvent};
use crate::core::compare::stats::CompareFilePaths;
use crate::core::compare::{ComparePhase, LayoutMode, RendererKind};
use crate::core::vcs::model::{FileChange, VcsCompareRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareWorkPriority {
    InteractiveSelectedFile,
    VisibleSidebarStats,
    VisibleViewportDiff,
    Overscan,
    TotalStats,
    Warmup,
}

impl CompareWorkPriority {
    pub const fn rank(self) -> u8 {
        match self {
            Self::InteractiveSelectedFile => 60,
            Self::VisibleViewportDiff => 50,
            Self::VisibleSidebarStats => 40,
            Self::Overscan => 30,
            Self::TotalStats => 20,
            Self::Warmup => 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareIntent {
    pub repo_path: PathBuf,
    pub request: VcsCompareRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileStats {
    Pending,
    Known { additions: u32, deletions: u32 },
}

#[derive(Debug, Clone)]
pub struct ManifestFile {
    pub key: FileKey,
    pub revision: FileRevision,
    pub paths: CompareFilePaths,
    pub status: FileStatus,
    pub binary: bool,
    pub stats: FileStats,
}

#[derive(Debug, Clone, Default)]
pub struct CompareManifestSnapshot {
    pub generation: CompareGeneration,
    pub resolved_left: String,
    pub resolved_right: String,
    pub files: Arc<[ManifestFile]>,
}

#[derive(Debug, Clone)]
pub struct PreparedDiff {
    key: FileKey,
    generation: CompareGeneration,
    revision: FileRevision,
    snapshot: FileDiffSnapshot,
}

impl PreparedDiff {
    pub fn new(
        key: FileKey,
        generation: CompareGeneration,
        revision: FileRevision,
        snapshot: FileDiffSnapshot,
    ) -> Self {
        Self {
            key,
            generation,
            revision,
            snapshot,
        }
    }

    pub fn key(&self) -> FileKey {
        self.key
    }

    pub fn generation(&self) -> CompareGeneration {
        self.generation
    }

    pub fn revision(&self) -> FileRevision {
        self.revision
    }

    pub fn snapshot(&self) -> &FileDiffSnapshot {
        &self.snapshot
    }

    pub fn document(&self) -> &Arc<DiffDocument> {
        &self.snapshot.diff
    }
}

impl PartialEq for PreparedDiff {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.generation == other.generation
            && self.revision == other.revision
    }
}

impl Eq for PreparedDiff {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineFailure {
    NotPorted,
    Failed(String),
}

impl PipelineFailure {
    pub fn message(&self) -> &str {
        match self {
            Self::NotPorted => "This comparison is not available yet.",
            Self::Failed(message) => message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineAction {
    SetRenderer(RendererKind),
    RequestFiles {
        files: Vec<FileKey>,
        priority: CompareWorkPriority,
    },
    RequestStats {
        files: Vec<FileKey>,
        priority: CompareWorkPriority,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareJob {
    pub generation: CompareGeneration,
    pub intent: CompareIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCompareRequest {
    pub generation: CompareGeneration,
    pub left_text: Arc<str>,
    pub right_text: Arc<str>,
    pub display_path: String,
    pub renderer: RendererKind,
    pub layout: LayoutMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTarget {
    pub key: FileKey,
    pub revision: FileRevision,
    pub paths: CompareFilePaths,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLoadRequest {
    pub generation: CompareGeneration,
    pub repo_path: PathBuf,
    pub request: VcsCompareRequest,
    pub file: FileTarget,
    pub priority: CompareWorkPriority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatsRequest {
    pub generation: CompareGeneration,
    pub repo_path: PathBuf,
    pub request: VcsCompareRequest,
    pub files: Vec<FileTarget>,
    pub priority: CompareWorkPriority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotalStatsRequest {
    pub generation: CompareGeneration,
    pub repo_path: PathBuf,
    pub request: VcsCompareRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusDiffRequest {
    pub generation: CompareGeneration,
    pub repo_path: PathBuf,
    pub key: FileKey,
    pub change: FileChange,
    pub renderer: RendererKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLoadRequest {
    pub generation: CompareGeneration,
    pub repo_path: PathBuf,
    pub file: FileTarget,
    pub old_reference: String,
    pub new_reference: String,
    pub purpose: SourcePurpose,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineEffect {
    RunCompare(Box<CompareJob>),
    RunTextCompare(Box<TextCompareRequest>),
    SupersedeCompare(CompareGeneration),
    LoadTotalStats(Box<TotalStatsRequest>),
    LoadFile(Box<FileLoadRequest>),
    LoadFileStats(Box<FileStatsRequest>),
    LoadStatusDiff(Box<StatusDiffRequest>),
    LoadSources(Box<SourceLoadRequest>),
}

impl From<PipelineAction> for ViewportAction {
    fn from(value: PipelineAction) -> Self {
        Self::Pipeline(value)
    }
}

impl From<PipelineAction> for Action {
    fn from(value: PipelineAction) -> Self {
        Self::Viewport(ViewportAction::Pipeline(value))
    }
}

impl From<PipelineEffect> for ViewportEffect {
    fn from(value: PipelineEffect) -> Self {
        Self::Pipeline(value)
    }
}

impl From<PipelineEffect> for Effect {
    fn from(value: PipelineEffect) -> Self {
        Self::Viewport(ViewportEffect::Pipeline(value))
    }
}

impl From<PipelineEvent> for ViewportEvent {
    fn from(value: PipelineEvent) -> Self {
        Self::Pipeline(value)
    }
}

impl From<PipelineEvent> for AppEvent {
    fn from(value: PipelineEvent) -> Self {
        Self::Viewport(ViewportEvent::Pipeline(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub key: FileKey,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone)]
pub struct LoadedSources {
    pub old: Option<Arc<str>>,
    pub new: Option<Arc<str>>,
}

#[derive(Debug, Clone)]
pub enum PipelineEvent {
    CompareProgress {
        generation: CompareGeneration,
        phase: ComparePhase,
    },
    CompareFinished {
        generation: CompareGeneration,
        request: VcsCompareRequest,
        manifest: Arc<CompareManifestSnapshot>,
    },
    CompareFailed {
        generation: CompareGeneration,
        failure: PipelineFailure,
    },
    TextCompareFinished {
        generation: CompareGeneration,
        prepared: PreparedDiff,
    },
    TextCompareFailed {
        generation: CompareGeneration,
        failure: PipelineFailure,
    },
    TotalStatsReady {
        generation: CompareGeneration,
        additions: u32,
        deletions: u32,
    },
    TotalStatsFailed {
        generation: CompareGeneration,
        failure: PipelineFailure,
    },
    FileLoaded {
        generation: CompareGeneration,
        prepared: PreparedDiff,
    },
    FileLoadFailed {
        generation: CompareGeneration,
        key: FileKey,
        revision: FileRevision,
        failure: PipelineFailure,
    },
    FileStatsReady {
        generation: CompareGeneration,
        stats: Arc<[FileStat]>,
        complete: bool,
    },
    FileStatsFailed {
        generation: CompareGeneration,
        failure: PipelineFailure,
    },
    StatusDiffReady {
        generation: CompareGeneration,
        prepared: PreparedDiff,
    },
    StatusDiffFailed {
        generation: CompareGeneration,
        key: FileKey,
        failure: PipelineFailure,
    },
    SourcesLoaded {
        generation: CompareGeneration,
        key: FileKey,
        revision: FileRevision,
        purpose: SourcePurpose,
        sources: LoadedSources,
    },
    SourcesFailed {
        generation: CompareGeneration,
        key: FileKey,
        revision: FileRevision,
        failure: PipelineFailure,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_through_root_contracts() {
        assert_eq!(
            Action::from(PipelineAction::SetRenderer(RendererKind::Builtin)),
            Action::Viewport(ViewportAction::Pipeline(PipelineAction::SetRenderer(
                RendererKind::Builtin
            )))
        );
        assert_eq!(
            Effect::from(PipelineEffect::SupersedeCompare(CompareGeneration(2))),
            Effect::Viewport(ViewportEffect::Pipeline(PipelineEffect::SupersedeCompare(
                CompareGeneration(2)
            )))
        );
        assert!(matches!(
            AppEvent::from(PipelineEvent::CompareFailed {
                generation: CompareGeneration(2),
                failure: PipelineFailure::NotPorted
            }),
            AppEvent::Viewport(ViewportEvent::Pipeline(PipelineEvent::CompareFailed { .. }))
        ));
    }
}
