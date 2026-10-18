use crate::core::compare::git::resolve_compare_refs;
use crate::core::compare::spec::CompareSpec;
use crate::core::error::Result;
use crate::core::vcs::git::GitService;

#[derive(Debug, Default, Clone, Copy)]
pub struct GitDiffBackend;

impl GitDiffBackend {
    pub fn compare_stats(
        &self,
        spec: &CompareSpec,
        git: &GitService,
    ) -> Result<Option<(i32, i32)>> {
        let (left, right) = resolve_compare_refs(spec, git)?;
        Ok(Some(git.diff_shortstat_find_renames(&left, &right)?))
    }
}
