use std::path::PathBuf;

use crate::core::compare::CompareSpec;
use crate::core::vcs::git::StatusItem;
use crate::events::RepositorySyncReason;
use crate::platform::persistence::Settings;

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
    RunCompare {
        generation: u64,
        request: CompareRequest,
    },
    LoadStatusDiff {
        generation: u64,
        index: usize,
        request: StatusDiffRequest,
    },
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
}
