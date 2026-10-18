use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ai::Provider;
use crate::contracts::navigation::RepositoryRevision;
use crate::contracts::review::Secret;
use crate::contracts::shared::{FileKey, FileRevision, HunkTarget, JobId, SourceTarget};
use crate::core::vcs::model::{
    ChangeBucket, FileChange, FileOperation, PublishAction, PublishPlan, VcsOperation,
};
use crate::core::vcs::mutation::{MutationMaterial, PatchSelection, SideRange};
use crate::core::vcs::source::SourceIdentity;
use crate::platform::secrets::AiKeyKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationsAction {
    StageSelectedFile,
    UnstageSelectedFile,
    DiscardSelectedFile,
    StageFile(FileKey),
    UnstageFile(FileKey),
    DiscardFile(FileKey),
    StageAllFiles,
    UnstageAllFiles,
    StageHunk,
    UnstageHunk,
    DiscardHunk,
    StageHunkAt(HunkTarget),
    UnstageHunkAt(HunkTarget),
    DiscardHunkAt(HunkTarget),
    SelectLines(SourceTarget),
    ToggleLineSelection(SourceTarget),
    ExtendLineSelection(SourceTarget),
    ToggleCurrentLineSelection,
    ToggleCurrentLineSelectionRange,
    ClearLineSelection,
    StageSelectedLines,
    UnstageSelectedLines,
    DiscardSelectedLines,
    SubmitCommit,
    RunOperation(VcsOperation),
    FetchRemote(String),
    FetchAllRemotes,
    Push { force_with_lease: bool },
    PublishDefault,
    OpenPublishMenu,
    Publish(Box<PublishAction>),
    PullCurrentBranch,
    SetAiKey { kind: AiKeyKind, value: Secret },
    ClearAiKey { kind: AiKeyKind },
    SetAiKeyEditing { kind: AiKeyKind, editing: bool },
    GenerateCommitMessage,
    ApplyConfirmed(ConfirmedMutation),
    RunConfirmedOperation(VcsOperation),
    OpenCommitComposer,
    CloseCommitComposer,
    ScrollCommitMessage(i32),
    ClosePublishMenu,
    SaveAiKeyField(AiKeyKind),
    ScrollSteeringPrompt(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StagedDigest(pub u64);

impl StagedDigest {
    pub fn of(diff: &[u8]) -> Self {
        use std::hash::{DefaultHasher, Hasher};
        let mut hasher = DefaultHasher::new();
        hasher.write(diff);
        Self(hasher.finish())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MutationSourceKey {
    pub repo_path: PathBuf,
    pub file: FileKey,
    pub revision: FileRevision,
    pub repository: RepositoryRevision,
    pub change: FileChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationSource {
    key: MutationSourceKey,
    material: MutationMaterial,
}

impl MutationSource {
    pub fn new(key: MutationSourceKey, material: MutationMaterial) -> Self {
        Self { key, material }
    }

    pub fn key(&self) -> &MutationSourceKey {
        &self.key
    }

    pub fn repo_path(&self) -> &Path {
        &self.key.repo_path
    }

    pub fn file(&self) -> FileKey {
        self.key.file
    }

    pub fn revision(&self) -> FileRevision {
        self.key.revision
    }

    pub fn repository(&self) -> RepositoryRevision {
        self.key.repository
    }

    pub fn change(&self) -> &FileChange {
        &self.key.change
    }

    pub fn bucket(&self) -> ChangeBucket {
        self.key.change.bucket
    }

    pub fn identity(&self) -> &SourceIdentity {
        &self.material.identity
    }

    pub fn patch(&self) -> &[u8] {
        &self.material.patch
    }

    pub fn material(&self) -> &MutationMaterial {
        &self.material
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MutationSelection {
    Hunks(Vec<HunkTarget>),
    HunksAt(Vec<SourceTarget>),
    Lines(Vec<SourceTarget>),
}

impl MutationSelection {
    pub fn targets(&self) -> impl Iterator<Item = &SourceTarget> {
        let (hunks, lines): (&[HunkTarget], &[SourceTarget]) = match self {
            Self::Hunks(hunks) => (hunks, &[]),
            Self::HunksAt(lines) | Self::Lines(lines) => (&[], lines),
        };
        hunks.iter().map(|hunk| &hunk.source).chain(lines)
    }

    pub fn file(&self) -> Option<(FileKey, FileRevision)> {
        let first = self.targets().next()?;
        let (file, revision) = (first.file, first.revision);
        self.targets()
            .all(|target| target.file == file && target.revision == revision)
            .then_some((file, revision))
    }

    pub fn patch_selection(&self) -> PatchSelection {
        let range = |target: &SourceTarget| SideRange {
            side: target.side,
            lines: target.lines.clone(),
        };
        match self {
            Self::Hunks(hunks) => {
                PatchSelection::Hunks(hunks.iter().map(|h| range(&h.source)).collect())
            }
            Self::HunksAt(lines) => PatchSelection::HunksAt(lines.iter().map(range).collect()),
            Self::Lines(lines) => PatchSelection::Lines(lines.iter().map(range).collect()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmedMutation {
    Selection(MutationSelection),
    Files(Vec<FileKey>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOperationRequest {
    pub job: JobId,
    pub repo_path: PathBuf,
    pub file_changes: Vec<FileChange>,
    pub operation: FileOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchOperationRequest {
    pub job: JobId,
    pub source: Arc<MutationSource>,
    pub selection: MutationSelection,
    pub operation: FileOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateCommitMessageRequest {
    pub generation: u64,
    pub repository: RepositoryRevision,
    pub repo_path: PathBuf,
    pub has_staged: bool,
    pub provider: Provider,
    pub api_key: Secret,
    pub steering_prompt: String,
    pub subject_override: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationsEffect {
    ApplyFileOperation(FileOperationRequest),
    LoadMutationSource(MutationSourceKey),
    ApplyPatch(PatchOperationRequest),
    CreateCommit {
        job: JobId,
        repo_path: PathBuf,
        message: String,
    },
    RunOperation {
        job: JobId,
        repo_path: PathBuf,
        operation: VcsOperation,
    },
    FetchRemote {
        job: JobId,
        repo_path: PathBuf,
        remote: String,
    },
    Push {
        job: JobId,
        repo_path: PathBuf,
        remote: String,
        refspec: String,
        force_with_lease: bool,
    },
    Publish {
        job: JobId,
        repo_path: PathBuf,
        action: Option<Box<PublishAction>>,
    },
    LoadPublishPlan {
        job: Option<JobId>,
        repo_path: PathBuf,
    },
    PullFastForward {
        job: JobId,
        repo_path: PathBuf,
        remote: String,
        branch: String,
    },
    LoadAiKeys,
    SaveAiKey {
        kind: AiKeyKind,
        value: Secret,
    },
    ClearAiKey {
        kind: AiKeyKind,
    },
    GenerateCommitMessage(Box<GenerateCommitMessageRequest>),
    CheckStagedDiff {
        generation: u64,
        repo_path: PathBuf,
        has_staged: bool,
    },
}

#[derive(Debug, Clone)]
pub enum MutationsEvent {
    FileOperationApplied {
        job: JobId,
        path: PathBuf,
    },
    FileOperationFailed {
        job: JobId,
        path: PathBuf,
        message: String,
    },
    CommitCreated {
        job: JobId,
        path: PathBuf,
    },
    CommitFailed {
        job: JobId,
        path: PathBuf,
        message: String,
    },
    OperationComplete {
        job: JobId,
        path: PathBuf,
        operation: VcsOperation,
        message: String,
    },
    OperationFailed {
        job: JobId,
        operation: VcsOperation,
        message: String,
    },
    FetchProgress {
        job: JobId,
        received_objects: usize,
        total_objects: usize,
        received_bytes: usize,
    },
    FetchComplete {
        job: JobId,
        path: PathBuf,
        remote: String,
    },
    FetchFailed {
        job: JobId,
        remote: String,
        message: String,
    },
    PushProgress {
        job: JobId,
        current: usize,
        total: usize,
        bytes: usize,
    },
    PushComplete {
        job: JobId,
        path: PathBuf,
        remote: String,
        branch: String,
    },
    PushFailed {
        job: JobId,
        remote: String,
        message: String,
    },
    PublishComplete {
        job: JobId,
        path: PathBuf,
        label: String,
    },
    PublishFailed {
        job: JobId,
        message: String,
    },
    PublishPlanReady {
        job: Option<JobId>,
        path: PathBuf,
        plan: Box<PublishPlan>,
    },
    PublishPlanFailed {
        job: Option<JobId>,
        message: String,
    },
    PullComplete {
        job: JobId,
        path: PathBuf,
        remote: String,
        branch: String,
        already_up_to_date: bool,
        behind: usize,
    },
    PullFailed {
        job: JobId,
        remote: String,
        branch: String,
        message: String,
    },
    MutationSourceReady {
        source: Arc<MutationSource>,
    },
    MutationSourceFailed {
        key: MutationSourceKey,
        message: String,
    },
    WorkerStopped,
    AiKeysLoaded {
        openai: Option<Secret>,
        anthropic: Option<Secret>,
    },
    AiKeysLoadFailed {
        message: String,
    },
    AiKeySaveFailed {
        message: String,
    },
    CommitMessageStarted {
        generation: u64,
        staged: StagedDigest,
    },
    StagedDiffChecked {
        generation: u64,
        staged: Option<StagedDigest>,
    },
    CommitMessageChunk {
        generation: u64,
        repository: RepositoryRevision,
        chunk: String,
    },
    CommitMessageFinished {
        generation: u64,
        repository: RepositoryRevision,
    },
    CommitMessageFailed {
        generation: u64,
        repository: RepositoryRevision,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::shell::{Action, AppEvent, Effect};

    #[test]
    fn routes_through_root_contracts() {
        assert_eq!(
            Action::from(MutationsAction::StageAllFiles),
            Action::Mutations(MutationsAction::StageAllFiles)
        );
        assert_eq!(
            Effect::from(MutationsEffect::LoadAiKeys),
            Effect::Mutations(MutationsEffect::LoadAiKeys)
        );
        assert!(matches!(
            AppEvent::from(MutationsEvent::WorkerStopped),
            AppEvent::Mutations(MutationsEvent::WorkerStopped)
        ));
    }
}
