use std::path::{Path, PathBuf};

use carbon::TextStore;

use crate::core::compare::{CompareOutput, CompareSpec, ProgressSink};
use crate::core::error::Result;
use crate::core::vcs::model::{
    RepoCapabilities, RepoLocation, RevisionId, VcsCompareRequest, VcsKind, VcsSnapshot,
};
use crate::events::RepositorySyncReason;

pub trait VcsBackend: Sync {
    fn kind(&self) -> VcsKind;
    fn detect(&self, path: &Path) -> Result<Option<RepoLocation>>;
    fn open(&self, location: RepoLocation) -> Result<Box<dyn VcsRepository>>;
    fn watch_paths(&self, location: &RepoLocation) -> Result<VcsWatchPaths>;
}

#[derive(Debug, Clone)]
pub struct VcsWatchPaths {
    pub metadata_dir: PathBuf,
    pub workdir: Option<PathBuf>,
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
    fn resolve_compare_spec(
        &mut self,
        spec: &CompareSpec,
    ) -> Result<(String, String, VcsCompareRequest)>;
    fn compare(
        &mut self,
        request: &VcsCompareRequest,
        reporter: Option<&dyn ProgressSink>,
    ) -> Result<CompareOutput>;
    fn compare_stats(&mut self, request: &VcsCompareRequest) -> Result<(i32, i32)>;
    fn compare_path(
        &mut self,
        request: &VcsCompareRequest,
        path: &str,
        deferred_file: Option<&carbon::FileDiff>,
    ) -> Result<CompareOutput>;
    fn compare_working_file(&mut self, path: &str) -> Result<CompareOutput>;
    fn read_file_text(&mut self, revision: &RevisionId, path: &str) -> Result<TextStore>;
}
