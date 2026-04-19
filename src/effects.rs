use std::path::PathBuf;

use crate::ai::Provider;
use crate::core::compare::{CompareSpec, RendererKind};
use crate::core::vcs::git::status::StatusScope;
use crate::core::vcs::git::{StatusItem, StatusOperation};
use crate::events::RepositorySyncReason;
use crate::platform::persistence::Settings;
use crate::platform::secrets::AiKeyKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareRequest {
    pub repo_path: PathBuf,
    pub spec: CompareSpec,
    pub github_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusDiffRequest {
    pub repo_path: PathBuf,
    pub item: StatusItem,
    pub renderer: RendererKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusOperationRequest {
    pub repo_path: PathBuf,
    pub item: StatusItem,
    pub operation: StatusOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchStatusOperationRequest {
    pub repo_path: PathBuf,
    pub items: Vec<StatusItem>,
    pub operation: StatusOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchOperationRequest {
    pub repo_path: PathBuf,
    pub patch: String,
    pub scope: StatusScope,
    pub operation: StatusOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    pub repo_path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRemoteRequest {
    pub repo_path: PathBuf,
    pub remote: String,
    pub toast_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRequest {
    pub repo_path: PathBuf,
    pub remote: String,
    pub refspec: String,
    pub force_with_lease: bool,
    pub toast_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullFfRequest {
    pub repo_path: PathBuf,
    pub remote: String,
    pub branch: String,
    pub toast_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    OpenRepositoryDialog,
    WatchRepository {
        path: Option<PathBuf>,
    },
    SyncRepository {
        path: PathBuf,
        reason: RepositorySyncReason,
    },
    FetchRemote(FetchRemoteRequest),
    Push(PushRequest),
    PullFf(PullFfRequest),
    RunCompare {
        generation: u64,
        request: CompareRequest,
    },
    LoadStatusDiff {
        generation: u64,
        index: usize,
        request: StatusDiffRequest,
    },
    ApplyStatusOperation(StatusOperationRequest),
    ApplyBatchStatusOperation(BatchStatusOperationRequest),
    ApplyPatchOperation(PatchOperationRequest),
    CreateCommit(CommitRequest),
    LoadPullRequest {
        url: String,
        repo_path: PathBuf,
        github_token: Option<String>,
    },
    StartDeviceFlow {
        client_id: String,
    },
    PollDeviceFlow {
        client_id: String,
        device_code: String,
        interval_seconds: u32,
    },
    FetchGitHubUser {
        token: String,
    },
    FetchAvatar {
        url: String,
    },
    PeekPullRequest {
        owner: String,
        repo: String,
        number: i32,
        github_token: Option<String>,
    },
    LoadGitHubToken,
    SaveGitHubToken(String),
    ClearGitHubToken,
    ResolveRef {
        repo_path: PathBuf,
        query: String,
        generation: u64,
    },
    SaveSettings(Settings),
    OpenBrowser {
        url: String,
    },
    SetClipboard(String),
    FetchContextLines(FetchContextLinesRequest),
    LoadAiKeys,
    SaveAiKey {
        kind: AiKeyKind,
        value: String,
    },
    ClearAiKey {
        kind: AiKeyKind,
    },
    GenerateCommitMessage(GenerateCommitMessageRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateCommitMessageRequest {
    pub repo_path: PathBuf,
    pub has_staged: bool,
    pub provider: Provider,
    pub api_key: String,
    pub steering_prompt: String,
    pub subject_override: Option<String>,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchContextLinesRequest {
    pub repo_path: PathBuf,
    pub reference: String,
    pub path: String,
    pub generation: u64,
    pub file_index: usize,
    pub hunk_index: usize,
    pub direction: crate::events::ContextDirection,
    pub amount: u32,
}
