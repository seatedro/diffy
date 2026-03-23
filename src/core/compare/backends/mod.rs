mod difftastic;
mod git_diff;

use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::CompareSpec;
use crate::core::error::Result;
use crate::core::vcs::git::GitService;

pub use difftastic::DifftasticBackend;
pub use git_diff::GitDiffBackend;

pub trait DiffBackend: Send + Sync {
    fn compare(&self, spec: &CompareSpec, git: &GitService) -> Result<Option<CompareOutput>>;
}
