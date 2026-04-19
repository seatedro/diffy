pub mod patch;
pub mod service;
pub mod status;

pub use service::{
    BranchInfo, CommitInfo, GitService, INDEX_REF, PullError, PullOutcome, TagInfo, WORKDIR_REF,
};
pub use status::{StatusItem, StatusOperation, StatusScope};
