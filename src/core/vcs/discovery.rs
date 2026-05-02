use std::path::Path;

use crate::core::error::Result;
use crate::core::vcs::backend::{VcsBackend, VcsRepository, VcsWatchPaths};
use crate::core::vcs::git::GitBackend;
use crate::core::vcs::jj::JjBackend;
use crate::core::vcs::model::RepoLocation;

pub fn backends() -> Vec<Box<dyn VcsBackend>> {
    vec![Box::new(JjBackend), Box::new(GitBackend)]
}

pub fn discover_repository(path: &Path) -> Result<Option<RepoLocation>> {
    for backend in &backends() {
        if let Some(location) = backend.detect(path)? {
            return Ok(Some(location));
        }
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
    let backend = backend_for_location(&location)?;
    backend.open(location)
}

pub fn watch_paths_for_repository(path: &Path) -> Result<VcsWatchPaths> {
    let location = discover_repository(path)?.ok_or_else(|| {
        crate::core::error::DiffyError::General(format!(
            "{} is not a supported repository",
            path.display()
        ))
    })?;
    backend_for_location(&location)?.watch_paths(&location)
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

fn backend_for_location(location: &RepoLocation) -> Result<Box<dyn VcsBackend>> {
    backends()
        .into_iter()
        .find(|backend| backend.owns_location(location))
        .ok_or_else(|| {
            crate::core::error::DiffyError::General(format!(
                "no backend registered for {}",
                location.kind
            ))
        })
}
