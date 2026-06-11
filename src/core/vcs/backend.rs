use std::path::{Path, PathBuf};

use carbon::TextStore;

use crate::core::compare::{
    CompareFileStatsTarget, CompareFileSummary, CompareOutput, ProgressSink, RendererKind,
};
use crate::core::error::{DiffyError, Result, VcsBackendKind};
use crate::core::forge::github::PullRequestInfo;
use crate::core::vcs::model::{
    FileChange, FileOperation, PublishAction, PublishOutcome, PublishPlan, PullFastForwardOutcome,
    RepoCapabilities, RepoLocation, RevisionId, VcsChange, VcsCompareRequest, VcsKind,
    VcsOperation, VcsSnapshot,
};
use crate::events::RepositorySyncReason;

fn unsupported_operation(location: &RepoLocation, op: &str) -> DiffyError {
    let backend = if location.kind == VcsKind::JJ {
        VcsBackendKind::Jj
    } else {
        VcsBackendKind::Git
    };
    DiffyError::vcs_fatal(backend, op, "not supported by this backend")
}

pub trait VcsBackend: Send + Sync {
    fn kind(&self) -> VcsKind;
    fn owns_location(&self, location: &RepoLocation) -> bool {
        self.kind() == location.kind
    }
    fn detect(&self, path: &Path) -> Result<Option<RepoLocation>>;
    fn open(&self, location: RepoLocation) -> Result<Box<dyn VcsRepository>>;
    fn watch_paths(&self, location: &RepoLocation) -> Result<VcsWatchPaths>;
}

#[derive(Debug, Clone)]
pub struct VcsWatchPaths {
    pub metadata_dir: PathBuf,
    pub workdir: Option<PathBuf>,
    pub worktree_metadata_paths: Vec<PathBuf>,
    pub watched_paths: Vec<PathBuf>,
}

pub trait VcsRepository: Send {
    fn location(&self) -> &RepoLocation;
    fn capabilities(&self) -> RepoCapabilities;
    fn resolve_ref(&mut self, reference: &str) -> Result<(String, String)>;
    fn snapshot(
        &mut self,
        reason: RepositorySyncReason,
        reporter: Option<&dyn ProgressSink>,
    ) -> Result<VcsSnapshot>;
    fn resolve_compare_request(&mut self, request: &VcsCompareRequest) -> Result<(String, String)>;
    fn compare(
        &mut self,
        request: &VcsCompareRequest,
        reporter: Option<&dyn ProgressSink>,
    ) -> Result<CompareOutput>;
    fn compare_stats(&mut self, request: &VcsCompareRequest) -> Result<(i32, i32)>;
    fn compare_history(
        &mut self,
        _left_ref: &str,
        _right_ref: &str,
        _limit: usize,
    ) -> Result<Vec<VcsChange>> {
        Ok(Vec::new())
    }
    fn compare_file_stats(
        &mut self,
        _request: &VcsCompareRequest,
        files: &[CompareFileStatsTarget],
    ) -> Result<Vec<(i32, i32)>> {
        Ok(files
            .iter()
            .map(CompareFileStatsTarget::fallback_stats)
            .collect())
    }
    fn compare_path(
        &mut self,
        request: &VcsCompareRequest,
        path: &str,
        deferred_file: Option<&CompareFileSummary>,
    ) -> Result<CompareOutput>;
    fn file_change_diff(
        &mut self,
        _change: &FileChange,
        _renderer: RendererKind,
    ) -> Result<CompareOutput> {
        Err(unsupported_operation(self.location(), "file-change diff"))
    }
    fn commit_diff(&mut self, _has_staged: bool) -> Result<String> {
        Err(unsupported_operation(self.location(), "commit diff"))
    }
    fn apply_file_operation(
        &mut self,
        _change: &FileChange,
        _operation: FileOperation,
    ) -> Result<()> {
        Err(unsupported_operation(self.location(), "file operation"))
    }
    fn apply_batch_file_operation(
        &mut self,
        changes: &[FileChange],
        operation: FileOperation,
    ) -> Result<()> {
        for change in changes {
            self.apply_file_operation(change, operation)?;
        }
        Ok(())
    }
    fn apply_patch_operation(&mut self, _patch: &str, _operation: FileOperation) -> Result<()> {
        Err(unsupported_operation(self.location(), "patch operation"))
    }
    fn create_commit(&mut self, _message: &str) -> Result<()> {
        Err(unsupported_operation(self.location(), "commit"))
    }
    fn run_operation(&mut self, _operation: &VcsOperation) -> Result<String> {
        Err(unsupported_operation(self.location(), "operation"))
    }
    fn fetch_remote(
        &mut self,
        _remote: &str,
        _progress: &mut dyn FnMut(usize, usize, usize),
    ) -> Result<()> {
        Err(unsupported_operation(self.location(), "fetch"))
    }
    fn push(
        &mut self,
        _remote: &str,
        _refspec: &str,
        _force_with_lease: bool,
        _progress: &mut dyn FnMut(usize, usize, usize),
    ) -> Result<()> {
        Err(unsupported_operation(self.location(), "push"))
    }
    fn publish_plan(&mut self) -> Result<PublishPlan> {
        Err(unsupported_operation(self.location(), "publish"))
    }
    fn publish(&mut self, _action: &PublishAction) -> Result<PublishOutcome> {
        Err(unsupported_operation(self.location(), "publish"))
    }
    fn pull_fast_forward(
        &mut self,
        _remote: &str,
        _branch: &str,
    ) -> Result<PullFastForwardOutcome> {
        Err(unsupported_operation(self.location(), "fast-forward pull"))
    }
    fn resolve_pull_request_comparison(
        &mut self,
        _pull_request_url: &str,
        _github_token: &str,
    ) -> Result<(PullRequestInfo, String, String)> {
        Err(unsupported_operation(
            self.location(),
            "GitHub pull request comparison",
        ))
    }
    fn compare_working_file(&mut self, path: &str) -> Result<CompareOutput>;
    fn read_file_text(&mut self, revision: &RevisionId, path: &str) -> Result<TextStore>;
}
