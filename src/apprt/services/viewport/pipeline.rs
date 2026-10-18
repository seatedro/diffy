use std::sync::Arc;

use crate::apprt::services::open_compare_repository;
use crate::contracts::viewport::pipeline::{
    CompareJob, CompareManifestSnapshot, FileLoadRequest, FileStat, FileStatsRequest,
    LoadedSources, PipelineFailure, PreparedDiff, SourceLoadRequest, StatusDiffRequest,
    TextCompareRequest, TotalStatsRequest,
};
use crate::core::compare::ProgressSink;

#[derive(Debug, Clone, Copy, Default)]
pub struct PipelineServices;

impl PipelineServices {
    pub fn run_compare(
        &self,
        job: &CompareJob,
        _progress: &dyn ProgressSink,
    ) -> Result<Arc<CompareManifestSnapshot>, PipelineFailure> {
        tracing::debug!(generation = job.generation.0, "compare: not yet ported");
        Err(PipelineFailure::NotPorted)
    }

    pub fn run_text_compare(
        &self,
        request: &TextCompareRequest,
    ) -> Result<PreparedDiff, PipelineFailure> {
        tracing::debug!(
            generation = request.generation.0,
            "text compare: not yet ported"
        );
        Err(PipelineFailure::NotPorted)
    }

    pub fn load_total_stats(
        &self,
        request: &TotalStatsRequest,
    ) -> Result<(u32, u32), PipelineFailure> {
        let mut repo = open_compare_repository(&request.repo_path, request.request.spec.refs())
            .map_err(|error| PipelineFailure::Failed(error.user_message()))?;
        let (additions, deletions) = repo
            .compare_stats(&request.request)
            .map_err(|error| PipelineFailure::Failed(error.user_message()))?;
        Ok((
            u32::try_from(additions).unwrap_or(0),
            u32::try_from(deletions).unwrap_or(0),
        ))
    }

    pub fn load_file(&self, request: &FileLoadRequest) -> Result<PreparedDiff, PipelineFailure> {
        tracing::debug!(
            generation = request.generation.0,
            file = request.file.key.0,
            "file load: not yet ported"
        );
        Err(PipelineFailure::NotPorted)
    }

    pub fn load_file_stats(
        &self,
        request: &FileStatsRequest,
    ) -> Result<Vec<FileStat>, PipelineFailure> {
        tracing::debug!(
            generation = request.generation.0,
            files = request.files.len(),
            "file stats: not yet ported"
        );
        Err(PipelineFailure::NotPorted)
    }

    pub fn load_status_diff(
        &self,
        request: &StatusDiffRequest,
    ) -> Result<PreparedDiff, PipelineFailure> {
        tracing::debug!(
            generation = request.generation.0,
            file = request.key.0,
            "status diff: not yet ported"
        );
        Err(PipelineFailure::NotPorted)
    }

    pub fn load_sources(
        &self,
        request: &SourceLoadRequest,
    ) -> Result<LoadedSources, PipelineFailure> {
        tracing::debug!(
            generation = request.generation.0,
            file = request.file.key.0,
            "sources: not yet ported"
        );
        Err(PipelineFailure::NotPorted)
    }
}
