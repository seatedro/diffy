use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ai::Provider;
use crate::contracts::navigation::RepositoryRevision;
use crate::contracts::review::Secret;
use crate::contracts::shared::{FileKey, FileRevision, HunkTarget, JobId, SourceTarget};
use crate::core::vcs::model::{
    ChangeBucket, FileChange, FileOperation, PublishAction, PublishPlan, VcsOperation,
};
use crate::platform::secrets::AiKeyKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationsAction {
    StageSelectedFile,
    UnstageSelectedFile,
    DiscardSelectedFile,
    StageFile(FileKey),
    UnstageFile(FileKey),
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
}

#[derive(Clone, PartialEq, Eq)]
pub struct MutationSource {
    repo_path: PathBuf,
    file: FileKey,
    revision: FileRevision,
    repository: RepositoryRevision,
    bucket: ChangeBucket,
    patch: Arc<[u8]>,
}

impl MutationSource {
    pub fn new(
        repo_path: PathBuf,
        file: FileKey,
        revision: FileRevision,
        repository: RepositoryRevision,
        bucket: ChangeBucket,
        patch: Arc<[u8]>,
    ) -> Self {
        Self {
            repo_path,
            file,
            revision,
            repository,
            bucket,
            patch,
        }
    }

    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    pub fn file(&self) -> FileKey {
        self.file
    }

    pub fn revision(&self) -> FileRevision {
        self.revision
    }

    pub fn repository(&self) -> RepositoryRevision {
        self.repository
    }

    pub fn bucket(&self) -> ChangeBucket {
        self.bucket
    }

    pub fn patch(&self) -> &[u8] {
        &self.patch
    }
}

impl fmt::Debug for MutationSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MutationSource")
            .field("repo_path", &self.repo_path)
            .field("file", &self.file)
            .field("revision", &self.revision)
            .field("repository", &self.repository)
            .field("bucket", &self.bucket)
            .field("patch_len", &self.patch.len())
            .finish()
    }
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
    pub source: MutationSource,
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
