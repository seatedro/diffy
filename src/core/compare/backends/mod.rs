#[cfg(feature = "difftastic")]
mod difftastic;
mod git_diff;

use crate::core::compare::progress::ProgressSink;
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::CompareSpec;
use crate::core::error::Result;
use crate::core::vcs::git::GitService;

/// Keep rename detection bounded on monorepo-scale diffs. This mirrors Git's
/// `diff.renameLimit` behavior closely enough for UI use: preserve rename
/// detection for ordinary diffs, but skip exhaustive similarity scans when a
/// range touches thousands of files.
const RENAME_DETECTION_LIMIT: usize = 1000;

#[cfg(feature = "difftastic")]
pub use difftastic::DifftasticBackend;
pub use git_diff::GitDiffBackend;
pub(crate) use git_diff::compare_output_from_diff;

pub trait DiffBackend: Send + Sync {
    fn compare(
        &self,
        spec: &CompareSpec,
        git: &GitService,
        reporter: Option<&dyn ProgressSink>,
    ) -> Result<Option<CompareOutput>>;
}

pub(crate) fn find_similar_bounded(diff: &mut git2::Diff<'_>) -> Result<()> {
    let mut options = git2::DiffFindOptions::new();
    options.renames(true).rename_limit(RENAME_DETECTION_LIMIT);
    diff.find_similar(Some(&mut options))?;
    Ok(())
}

#[cfg(not(feature = "difftastic"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct DifftasticBackend;

#[cfg(not(feature = "difftastic"))]
impl DifftasticBackend {
    pub const fn is_available() -> bool {
        false
    }

    pub fn compare_path(
        &self,
        _spec: &CompareSpec,
        _path: &str,
        _git: &GitService,
    ) -> Result<Option<CompareOutput>> {
        Ok(None)
    }
}

#[cfg(not(feature = "difftastic"))]
impl DiffBackend for DifftasticBackend {
    fn compare(
        &self,
        _spec: &CompareSpec,
        _git: &GitService,
        _reporter: Option<&dyn ProgressSink>,
    ) -> Result<Option<CompareOutput>> {
        Ok(None)
    }
}
