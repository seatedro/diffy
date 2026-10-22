use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub use diffy_diff::DiffStats as FileStats;
use diffy_diff::{
    ChangeSummary, Comparison, ComparisonOptions, ContentChange, DiffDocument, FileDiffSnapshot,
    FileStatus, LineGroup, ObjectId, OutlineItem, Placeholder, SourcePurpose,
};
pub use diffy_ui::diff_view::{Preparation, PreparedSegment};

use crate::contracts::shared::{CompareGeneration, FileKey, FileRevision};
use crate::contracts::shell::{Action, AppEvent, Effect};
use crate::contracts::viewport::{ViewportAction, ViewportEffect, ViewportEvent};
use crate::core::compare::stats::CompareFilePaths;
use crate::core::compare::{CompareEntry, ComparePhase, LayoutMode, RendererKind};
use crate::core::vcs::model::{ChangeBucket, FileChange, VcsCompareRequest};

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

#[derive(Debug, Clone)]
pub struct ManifestFile {
    pub key: FileKey,
    pub revision: FileRevision,
    pub paths: CompareFilePaths,
    pub status: FileStatus,
    pub binary: bool,
    pub stats: FileStats,
    pub content: ContentChange,
    pub mode_changed: bool,
    pub old_id: Option<ObjectId>,
    pub new_id: Option<ObjectId>,
    pub buckets: Vec<ChangeBucket>,
    pub outline: Arc<[OutlineItem]>,
    pub line_groups: Arc<[LineGroup]>,
}

impl ManifestFile {
    pub fn new(
        key: FileKey,
        revision: FileRevision,
        paths: CompareFilePaths,
        status: FileStatus,
    ) -> Self {
        Self {
            key,
            revision,
            paths,
            status,
            binary: false,
            stats: FileStats::Pending,
            content: ContentChange::Unknown,
            mode_changed: false,
            old_id: None,
            new_id: None,
            buckets: Vec::new(),
            outline: Arc::from([]),
            line_groups: Arc::from([]),
        }
    }

    pub fn from_entry(key: FileKey, revision: FileRevision, entry: &CompareEntry) -> Self {
        Self {
            key,
            revision,
            paths: entry.paths.clone(),
            status: entry.summary.status,
            binary: entry.summary.binary,
            stats: entry.stats,
            content: entry.summary.content,
            mode_changed: entry.summary.mode_changed,
            old_id: entry.old_id.clone(),
            new_id: entry.new_id.clone(),
            buckets: Vec::new(),
            outline: Arc::from([]),
            line_groups: Arc::from([]),
        }
    }

    pub fn summary(&self) -> ChangeSummary {
        ChangeSummary {
            status: self.status,
            binary: self.binary,
            content: self.content,
            mode_changed: self.mode_changed,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompareManifestSnapshot {
    pub generation: CompareGeneration,
    pub resolved_left: String,
    pub resolved_right: String,
    pub files: Arc<[ManifestFile]>,
    pub totals: FileStats,
    pub working_tree: bool,
    pub repo_path: Option<PathBuf>,
}

impl CompareManifestSnapshot {
    pub fn file(&self, key: FileKey) -> Option<&ManifestFile> {
        let index = usize::try_from(key.0).ok()?;
        self.files
            .get(index)
            .filter(|file| file.key == key)
            .or_else(|| self.files.iter().find(|file| file.key == key))
    }
}

#[derive(Debug, Clone)]
pub struct PreparedDiff {
    key: FileKey,
    generation: CompareGeneration,
    revision: FileRevision,
    snapshot: FileDiffSnapshot,
    segment: Option<Arc<PreparedSegment>>,
    line_groups: Arc<[LineGroup]>,
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
            segment: None,
            line_groups: Arc::from([]),
        }
    }

    pub fn with_segment(mut self, preparation: Option<Preparation>) -> Self {
        self.segment = preparation.map(|preparation| {
            Arc::new(PreparedSegment::prepare(self.snapshot.clone(), preparation))
        });
        let doc = &self.snapshot.diff;
        self.line_groups = match &self.segment {
            Some(segment) => segment.comparison().line_groups(doc, 0).into(),
            None if doc.prefers_structural(0) => Comparison::new(doc, ComparisonOptions::default())
                .line_groups(doc, 0)
                .into(),
            None => Arc::from([]),
        };
        self
    }

    pub fn line_groups(&self) -> &Arc<[LineGroup]> {
        &self.line_groups
    }

    pub fn segment(&self) -> Option<&Arc<PreparedSegment>> {
        self.segment.as_ref()
    }

    pub fn estimated_lines(&self) -> u32 {
        self.snapshot.estimated_lines()
    }

    pub fn evicted_placeholder(&self) -> Placeholder {
        Placeholder::new(
            diffy_diff::Revision(self.revision.0),
            diffy_diff::SlotState::Evicted,
            self.estimated_lines(),
            self.document().files().meta[0].clone(),
        )
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

    pub fn stats(&self) -> FileStats {
        self.snapshot.diff.stats(0)
    }

    pub fn summary(&self) -> ChangeSummary {
        self.snapshot.diff.summary(0)
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
    RequestTotals,
    RequestSources {
        file: FileKey,
        purpose: SourcePurpose,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSizeRequest {
    pub token: u64,
    pub repo_path: PathBuf,
    pub left: String,
    pub right: String,
    pub mode: crate::core::compare::CompareMode,
    /// Where `right` is checked out, to measure its files on disk.
    pub worktree: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffSize {
    pub files: Option<u32>,
    pub additions: u32,
    pub deletions: u32,
    pub commits: u32,
    pub commits_capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareJob {
    pub generation: CompareGeneration,
    pub intent: CompareIntent,
    pub preparation: Option<Preparation>,
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
    pub entry: CompareEntry,
}

impl FileTarget {
    pub fn paths(&self) -> &CompareFilePaths {
        &self.entry.paths
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLoadRequest {
    pub generation: CompareGeneration,
    pub repo_path: PathBuf,
    pub request: VcsCompareRequest,
    pub file: FileTarget,
    pub priority: CompareWorkPriority,
    pub preparation: Option<Preparation>,
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
    pub preparation: Option<Preparation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLoadRequest {
    pub generation: CompareGeneration,
    pub repo_path: PathBuf,
    pub file: FileTarget,
    pub prepared: PreparedDiff,
    pub old_reference: String,
    pub new_reference: String,
    pub purpose: SourcePurpose,
    pub preparation: Option<Preparation>,
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
    ResumeInstalls(CompareGeneration),
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

pub const INSTALL_LINES_PER_FRAME: u64 = 40_000;
pub const INSTALL_ITEMS_PER_FRAME: usize = 256;
pub const WORKING_SET_LINES: u64 = 400_000;
pub const WORKING_SET_FILES: usize = 512;

#[derive(Debug, Clone)]
pub enum PipelineInstall {
    View(crate::contracts::viewport::interaction::ViewportInstall),
    Slot(SlotUpdate),
}

impl PipelineInstall {
    pub fn cost(&self) -> u64 {
        use crate::contracts::viewport::interaction::ViewportInstall;
        match self {
            Self::View(ViewportInstall::File { prepared, .. })
            | Self::View(ViewportInstall::TextCompare { prepared }) => {
                u64::from(prepared.estimated_lines())
            }
            _ => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompareResult {
    pub manifest: Arc<CompareManifestSnapshot>,
    pub entries: Arc<[CompareEntry]>,
    pub positions: HashMap<FileKey, usize>,
    pub loaded: Vec<PreparedDiff>,
    pub placeholders: HashMap<FileKey, Placeholder>,
    pub installs: Vec<PipelineInstall>,
    pub notice: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SlotUpdate {
    Placeholder {
        key: FileKey,
        placeholder: Placeholder,
    },
    Evict {
        key: FileKey,
        revision: FileRevision,
        estimated_lines: u32,
    },
}

#[derive(Debug, Clone)]
pub enum PipelineEvent {
    InstallsResumed {
        generation: CompareGeneration,
    },
    CompareProgress {
        generation: CompareGeneration,
        phase: ComparePhase,
    },
    CompareFinished {
        generation: CompareGeneration,
        request: VcsCompareRequest,
        result: CompareResult,
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
        purpose: SourcePurpose,
        prepared: PreparedDiff,
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
