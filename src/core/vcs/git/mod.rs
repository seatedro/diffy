pub mod adapter;
pub mod service;
pub mod status;

pub use adapter::GitBackend;
pub use service::{
    BranchInfo, CommitInfo, GitService, INDEX_REF, PullError, PullOutcome, RefMetadata, TagInfo,
    WORKDIR_REF, is_pr_ref, mentions, pr_ref_path, ref_metadata,
};
pub use status::{StatusItem, StatusOperation, StatusScope};
