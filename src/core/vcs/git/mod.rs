pub mod adapter;
pub mod service;
pub mod status;

pub use adapter::GitBackend;
pub use service::{
    BranchInfo, CommitInfo, GitService, INDEX_REF, PatchApplyTarget, PullError, PullOutcome,
    TagInfo, WORKDIR_REF, pr_ref_path,
};
pub use status::{StatusItem, StatusOperation, StatusScope};
