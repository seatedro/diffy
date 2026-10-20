use std::path::Path;
use std::sync::Arc;

use crate::apprt::services::open_compare_repository;
use crate::contracts::navigation::HistoryRequest;
use crate::core::error::Result;
use crate::core::vcs::discovery;
use crate::core::vcs::model::VcsChange;

const HISTORY_LIMIT: usize = 500;

#[derive(Debug, Clone, Copy, Default)]
pub struct NavigationServices;

impl NavigationServices {
    pub fn load_history(&self, request: &HistoryRequest) -> Result<Arc<[VcsChange]>> {
        let mut repo = open_compare_repository(
            &request.repo_path,
            [request.left_ref.as_str(), request.right_ref.as_str()],
        )?;
        let commits = repo.compare_history(&request.left_ref, &request.right_ref, HISTORY_LIMIT)?;
        Ok(commits.into())
    }

    pub fn resolve_ref(&self, repo_path: &Path, reference: &str) -> Result<(String, String)> {
        let mut repo = discovery::open_repository(repo_path)?;
        repo.resolve_ref(reference)
    }
}
