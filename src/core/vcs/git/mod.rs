pub mod patch;
pub mod service;
pub mod status;

pub use service::{BranchInfo, CommitInfo, GitService, TagInfo, WORKDIR_REF};
pub use status::{StatusItem, StatusOperation, StatusScope};
