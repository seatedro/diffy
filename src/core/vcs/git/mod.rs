pub mod adapter;
pub mod service;
pub mod status;

pub use adapter::GitBackend;
pub use service::{
    BranchInfo, CommitInfo, GitService, INDEX_REF, PullError, PullOutcome, RefMetadata, TagInfo,
    TreeBlob, WORKDIR_REF, all_ignored, clone_repository, has_uncommitted_changes, is_full_hex_oid,
    is_pr_ref, mentions, pr_ref_path, ref_metadata, uncommitted_count,
};
pub use status::{StatusItem, StatusOperation, StatusScope};
