use std::ops::Range;
use std::rc::Rc;

pub use diffy_diff::{FileId as FileKey, Side};

use crate::contracts::commands::SettingsSnapshot;
use crate::contracts::navigation::RepositoryContext;
use crate::contracts::review::ReviewContext;
use crate::contracts::viewport::CompareManifestSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct WorkspaceEpoch(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CompareGeneration(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct AccountEpoch(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct JobId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct FileRevision(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceTarget {
    pub file: FileKey,
    pub revision: FileRevision,
    pub side: Side,
    pub lines: Range<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HunkTarget {
    pub source: SourceTarget,
    pub hunk: u32,
}

#[derive(Debug, Clone, Default)]
pub struct SharedContext {
    pub workspace: WorkspaceEpoch,
    pub account: AccountEpoch,
    pub revision: u64,
    pub settings: Rc<SettingsSnapshot>,
    pub repository: Option<Rc<RepositoryContext>>,
    pub manifest: Option<Rc<CompareManifestSnapshot>>,
    pub selection: Option<SourceTarget>,
    pub review: Option<Rc<ReviewContext>>,
}
