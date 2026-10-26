pub mod adapter;
pub mod service;
pub mod status;

pub use adapter::GitBackend;
pub use service::{
    BranchInfo, CommitInfo, GitService, INDEX_REF, LineHunk, PullError, PullOutcome, RefMetadata,
    TagInfo, TreeBlob, WORKDIR_REF, all_ignored, clone_repository, github_remotes,
    has_uncommitted_changes, init_pull_request_cache, is_full_hex_oid, is_pr_ref,
    is_pull_request_cache, line_hunks, mentions, merge_base_oid, pr_ref_path, pr_ref_paths,
    ref_metadata, ref_oid, uncommitted_count,
};
pub use status::{StatusItem, StatusOperation, StatusScope};
