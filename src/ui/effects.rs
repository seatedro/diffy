use std::path::PathBuf;

use crate::core::compare::CompareSpec;
use crate::platform::persistence::Settings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareRequest {
    pub repo_path: PathBuf,
    pub spec: CompareSpec,
    pub github_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    OpenRepositoryDialog,
    LoadRepository {
        path: PathBuf,
    },
    RunCompare {
        generation: u64,
        request: CompareRequest,
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
    SaveSettings(Settings),
    OpenBrowser {
        url: String,
    },
}
