use std::path::PathBuf;

use crate::core::compare::{CompareOutput, CompareSpec};
use crate::core::vcs::git::{BranchInfo, CommitInfo, TagInfo};
use crate::core::vcs::github::{DeviceFlowState, PullRequestInfo};

#[derive(Debug, Clone)]
pub struct RepositoryLoaded {
    pub path: PathBuf,
    pub branches: Vec<BranchInfo>,
    pub tags: Vec<TagInfo>,
    pub commits: Vec<CommitInfo>,
}

#[derive(Debug, Clone)]
pub struct CompareFinished {
    pub generation: u64,
    pub spec: CompareSpec,
    pub resolved_left: String,
    pub resolved_right: String,
    pub output: CompareOutput,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    RepositoryDialogClosed {
        path: Option<PathBuf>,
    },
    RepositoryLoaded(RepositoryLoaded),
    RepositoryLoadFailed {
        path: PathBuf,
        message: String,
    },
    CompareFinished(CompareFinished),
    CompareFailed {
        generation: u64,
        message: String,
    },
    PullRequestLoaded {
        url: String,
        info: PullRequestInfo,
        left_ref: String,
        right_ref: String,
    },
    PullRequestLoadFailed {
        url: String,
        message: String,
    },
    DeviceFlowStarted(DeviceFlowState),
    DeviceFlowStartFailed {
        message: String,
    },
    DeviceFlowCompleted {
        token: String,
    },
    DeviceFlowFailed {
        message: String,
    },
    SettingsSaved,
    SettingsSaveFailed {
        message: String,
    },
    BrowserOpenFailed {
        message: String,
    },
}
