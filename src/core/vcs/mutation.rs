use std::ops::Range;
use std::sync::Arc;

pub use diffy_diff::Side;

use crate::core::error::{DiffyError, Result};
use crate::core::vcs::backend::VcsRepository;
use crate::core::vcs::model::{ChangeBucket, FileChange, FileOperation};
use crate::core::vcs::patch::{PatchError, build_selection_patch, parse_exact_patch, select_file};
use crate::core::vcs::source::SourceIdentity;

#[derive(Clone, PartialEq, Eq)]
pub struct MutationMaterial {
    pub patch: Arc<[u8]>,
    pub identity: SourceIdentity,
}

impl std::fmt::Debug for MutationMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MutationMaterial")
            .field("patch_len", &self.patch.len())
            .field("identity", &self.identity)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApplyTarget {
    Index,
    Worktree,
    IndexAndWorktree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PatchApplication {
    pub target: ApplyTarget,
    pub reverse: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SideRange {
    pub side: Side,
    pub lines: Range<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchSelection {
    Hunks(Vec<SideRange>),
    Lines(Vec<SideRange>),
}

impl PatchSelection {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Hunks(ranges) | Self::Lines(ranges) => ranges.iter().all(|r| r.lines.is_empty()),
        }
    }
}

pub fn plan_application(
    bucket: ChangeBucket,
    operation: FileOperation,
    staging_area: bool,
) -> std::result::Result<PatchApplication, &'static str> {
    use ApplyTarget::*;
    let plan = |target, reverse| Ok(PatchApplication { target, reverse });
    match (staging_area, bucket, operation) {
        (true, ChangeBucket::Unstaged | ChangeBucket::Untracked, FileOperation::Stage) => {
            plan(Index, false)
        }
        (true, ChangeBucket::Unstaged | ChangeBucket::Untracked, FileOperation::Discard) => {
            plan(Worktree, true)
        }
        (true, ChangeBucket::Staged, FileOperation::Unstage) => plan(Index, true),
        (true, ChangeBucket::Staged, FileOperation::Discard) => plan(IndexAndWorktree, true),
        (false, ChangeBucket::WorkingCopy, FileOperation::Discard) => plan(Worktree, true),
        (_, ChangeBucket::Conflicted, _) => Err("Resolve the conflict before changing its lines."),
        (true, ChangeBucket::Staged, FileOperation::Stage) => {
            Err("These lines are already staged.")
        }
        (true, _, FileOperation::Unstage) => Err("These lines are not staged."),
        (false, _, FileOperation::Stage | FileOperation::Unstage) => {
            Err("This repository has no staging area.")
        }
        _ => Err("This change cannot be modified by line."),
    }
}

pub fn apply_selection(
    repo: &mut dyn VcsRepository,
    material: &MutationMaterial,
    change: &FileChange,
    selection: &PatchSelection,
    operation: FileOperation,
) -> Result<()> {
    let capabilities = repo.capabilities();
    if !capabilities.partial_hunk_mutation {
        return Err(DiffyError::General(
            "This repository backend does not support line operations.".to_owned(),
        ));
    }
    let application = plan_application(change.bucket, operation, capabilities.staging_area)
        .map_err(|message| DiffyError::General(message.to_owned()))?;
    let files = parse_exact_patch(&material.patch).map_err(patch_error)?;
    let file = select_file(&files, &change.path).map_err(patch_error)?;
    let patch = build_selection_patch(file, selection, application.reverse).map_err(patch_error)?;
    let current = repo.mutation_material(change)?;
    if current.patch != material.patch || current.identity != material.identity {
        return Err(patch_error(PatchError::Stale));
    }
    repo.apply_patch(&patch, application)
}

fn patch_error(error: PatchError) -> DiffyError {
    DiffyError::General(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_follow_bucket_and_operation() {
        let stage = plan_application(ChangeBucket::Unstaged, FileOperation::Stage, true).unwrap();
        assert_eq!(stage.target, ApplyTarget::Index);
        assert!(!stage.reverse);
        let unstage = plan_application(ChangeBucket::Staged, FileOperation::Unstage, true).unwrap();
        assert_eq!(unstage.target, ApplyTarget::Index);
        assert!(unstage.reverse);
        let discard =
            plan_application(ChangeBucket::Unstaged, FileOperation::Discard, true).unwrap();
        assert_eq!(discard.target, ApplyTarget::Worktree);
        assert!(discard.reverse);
        let jj = plan_application(ChangeBucket::WorkingCopy, FileOperation::Discard, false);
        assert_eq!(jj.unwrap().target, ApplyTarget::Worktree);
        assert!(plan_application(ChangeBucket::Staged, FileOperation::Stage, true).is_err());
        assert!(plan_application(ChangeBucket::WorkingCopy, FileOperation::Stage, false).is_err());
    }
}
