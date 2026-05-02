use std::path::Path;

use crate::core::error::Result;
use crate::core::vcs::backend::{VcsBackend, VcsRepository, VcsWatchPaths};
use crate::core::vcs::git::GitBackend;
use crate::core::vcs::jj::JjBackend;
use crate::core::vcs::model::{RepoLocation, VcsKind};

pub fn discover_repository(path: &Path) -> Result<Option<RepoLocation>> {
    let jj = JjBackend;
    if let Some(location) = jj.detect(path)? {
        return Ok(Some(location));
    }
    let git = GitBackend;
    if let Some(location) = git.detect(path)? {
        return Ok(Some(location));
    }
    Ok(None)
}

pub fn is_repository(path: &Path) -> bool {
    discover_repository(path).ok().flatten().is_some()
}

pub fn open_repository(path: &Path) -> Result<Box<dyn VcsRepository>> {
    let location = discover_repository(path)?.ok_or_else(|| {
        crate::core::error::DiffyError::General(format!(
            "{} is not a supported repository",
            path.display()
        ))
    })?;
    open_location(location)
}

pub fn open_location(location: RepoLocation) -> Result<Box<dyn VcsRepository>> {
    match location.kind {
        VcsKind::Git => GitBackend.open(location),
        VcsKind::Jj => JjBackend.open(location),
    }
}

pub fn watch_paths_for_repository(path: &Path) -> Result<VcsWatchPaths> {
    let location = discover_repository(path)?.ok_or_else(|| {
        crate::core::error::DiffyError::General(format!(
            "{} is not a supported repository",
            path.display()
        ))
    })?;
    match location.kind {
        VcsKind::Git => GitBackend.watch_paths(&location),
        VcsKind::Jj => JjBackend.watch_paths(&location),
    }
}

pub fn open_git_repository(path: &Path) -> Result<Box<dyn VcsRepository>> {
    let git = GitBackend;
    let location = git.detect(path)?.ok_or_else(|| {
        crate::core::error::DiffyError::General(format!(
            "{} is not a Git repository",
            path.display()
        ))
    })?;
    git.open(location)
}
