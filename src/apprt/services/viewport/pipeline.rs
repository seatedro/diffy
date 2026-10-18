use std::sync::Arc;

use diffy_diff::{
    DiffDocument, DocumentBuilder, FileDiffSnapshot, FileMeta, FileSources, FileStatus, Revision,
    SourceCoverage, TextStore,
};

use crate::apprt::services::open_compare_repository;
use crate::contracts::shared::{CompareGeneration, FileKey, FileRevision};
use crate::contracts::viewport::pipeline::{
    CompareJob, CompareManifestSnapshot, CompareResult, FileLoadRequest, FileStat,
    FileStatsRequest, ManifestFile, PipelineFailure, PreparedDiff, SourceLoadRequest,
    StatusDiffRequest, TextCompareRequest, TotalStatsRequest,
};
use crate::core::compare::service::matching_document;
use crate::core::compare::{CompareOutput, ComparePhase, ProgressSink, compare_text};
use crate::core::error::DiffyError;
use crate::core::vcs::model::RevisionId;
use crate::core::vcs::source::SourceText;

#[derive(Debug, Clone, Copy, Default)]
pub struct PipelineServices;

fn failed(error: DiffyError) -> PipelineFailure {
    PipelineFailure::Failed(error.user_message())
}

fn invalid(error: impl std::fmt::Display) -> PipelineFailure {
    PipelineFailure::Failed(error.to_string())
}

pub fn file_revision(generation: CompareGeneration) -> FileRevision {
    FileRevision(generation.0)
}

pub fn key_at(index: usize) -> FileKey {
    FileKey(index as u64)
}

pub fn prepare(
    key: FileKey,
    generation: CompareGeneration,
    revision: FileRevision,
    doc: Arc<DiffDocument>,
) -> Result<PreparedDiff, PipelineFailure> {
    let snapshot = FileDiffSnapshot::new(key, Revision(revision.0), doc).map_err(invalid)?;
    Ok(PreparedDiff::new(key, generation, revision, snapshot))
}

pub fn compare_result(
    generation: CompareGeneration,
    resolved: (String, String),
    output: CompareOutput,
) -> Result<CompareResult, PipelineFailure> {
    let revision = file_revision(generation);
    let files: Arc<[ManifestFile]> = output
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| ManifestFile::from_entry(key_at(index), revision, entry))
        .collect();
    let prepared = output
        .documents
        .iter()
        .enumerate()
        .filter_map(|(index, doc)| Some((index, doc.clone()?)))
        .map(|(index, doc)| prepare(key_at(index), generation, revision, doc))
        .collect::<Result<Arc<[PreparedDiff]>, _>>()?;
    let notice = output
        .used_fallback
        .then(|| output.fallback_message.clone())
        .filter(|message| !message.is_empty());
    Ok(CompareResult {
        manifest: Arc::new(CompareManifestSnapshot {
            generation,
            resolved_left: resolved.0,
            resolved_right: resolved.1,
            files,
        }),
        entries: output.entries.into(),
        prepared,
        notice,
    })
}

fn unchanged_document(path: &str, text: &str) -> Result<Arc<DiffDocument>, PipelineFailure> {
    let store = TextStore::new(text);
    let mut builder = DocumentBuilder::new();
    builder
        .file(
            FileMeta {
                old_path: Some(path.into()),
                new_path: Some(path.into()),
                status: FileStatus::Modified,
                ..FileMeta::default()
            },
            SourceCoverage::Full,
            store.clone(),
            store,
        )
        .finish()
        .map_err(invalid)?;
    Ok(Arc::new(builder.build()))
}

impl PipelineServices {
    pub fn run_compare(
        &self,
        job: &CompareJob,
        progress: &dyn ProgressSink,
    ) -> Result<CompareResult, PipelineFailure> {
        progress.phase(ComparePhase::OpeningRepo);
        let request = &job.intent.request;
        let mut repo =
            open_compare_repository(&job.intent.repo_path, request.spec.refs()).map_err(failed)?;
        progress.phase(ComparePhase::ResolvingRefs);
        let resolved = repo.resolve_compare_request(request).map_err(failed)?;
        let output = repo.compare(request, Some(progress)).map_err(failed)?;
        compare_result(job.generation, resolved, output)
    }

    pub fn run_text_compare(
        &self,
        request: &TextCompareRequest,
    ) -> Result<PreparedDiff, PipelineFailure> {
        let output = compare_text(
            &request.left_text,
            &request.right_text,
            &request.display_path,
            request.renderer,
        )
        .map_err(failed)?;
        let doc = match output.document(0) {
            Some(doc) => doc.clone(),
            None => unchanged_document(&request.display_path, &request.left_text)?,
        };
        let revision = file_revision(request.generation);
        prepare(key_at(0), request.generation, revision, doc)
    }

    pub fn load_total_stats(
        &self,
        request: &TotalStatsRequest,
    ) -> Result<(u32, u32), PipelineFailure> {
        let mut repo = open_compare_repository(&request.repo_path, request.request.spec.refs())
            .map_err(failed)?;
        let (additions, deletions) = repo.compare_stats(&request.request).map_err(failed)?;
        Ok((
            u32::try_from(additions).unwrap_or(0),
            u32::try_from(deletions).unwrap_or(0),
        ))
    }

    pub fn load_file(&self, request: &FileLoadRequest) -> Result<PreparedDiff, PipelineFailure> {
        let entry = &request.file.entry;
        let mut repo = open_compare_repository(&request.repo_path, request.request.spec.refs())
            .map_err(failed)?;
        let path = entry.path().into_owned();
        let deferred = entry.stats.is_pending().then_some(entry);
        let output = repo
            .compare_path(&request.request, &path, deferred)
            .map_err(failed)?;
        let doc = matching_document(output, entry).map_err(failed)?;
        prepare(
            request.file.key,
            request.generation,
            request.file.revision,
            doc,
        )
    }

    pub fn load_file_stats(
        &self,
        request: &FileStatsRequest,
    ) -> Result<Vec<FileStat>, PipelineFailure> {
        let mut repo = open_compare_repository(&request.repo_path, request.request.spec.refs())
            .map_err(failed)?;
        let entries: Vec<_> = request
            .files
            .iter()
            .map(|file| file.entry.clone())
            .collect();
        let stats = repo
            .compare_file_stats(&request.request, &entries)
            .map_err(failed)?;
        Ok(request
            .files
            .iter()
            .zip(stats)
            .map(|(file, (additions, deletions))| FileStat {
                key: file.key,
                additions: u32::try_from(additions).unwrap_or(0),
                deletions: u32::try_from(deletions).unwrap_or(0),
            })
            .collect())
    }

    pub fn load_status_diff(
        &self,
        request: &StatusDiffRequest,
    ) -> Result<PreparedDiff, PipelineFailure> {
        let mut repo =
            open_compare_repository(&request.repo_path, std::iter::empty()).map_err(failed)?;
        let output = repo
            .file_change_diff(&request.change, request.renderer)
            .map_err(failed)?;
        let doc = output
            .document(0)
            .cloned()
            .ok_or_else(|| PipelineFailure::Failed("No changes in this file.".to_owned()))?;
        prepare(
            request.key,
            request.generation,
            file_revision(request.generation),
            doc,
        )
    }

    pub fn load_sources(
        &self,
        request: &SourceLoadRequest,
    ) -> Result<PreparedDiff, PipelineFailure> {
        let mut repo =
            open_compare_repository(&request.repo_path, std::iter::empty()).map_err(failed)?;
        let kind = repo.location().kind;
        let paths = request.file.paths();
        let mut read = |reference: &str, path: Option<std::borrow::Cow<'_, str>>| {
            let Some(path) = path else {
                return Ok(None);
            };
            let revision = RevisionId {
                backend: kind,
                id: reference.to_owned(),
            };
            match repo.read_file_source(&revision, &path).map_err(failed)? {
                SourceText::Text(text) => Ok(Some(text)),
                SourceText::Binary(_) => Err(PipelineFailure::Failed(
                    "Binary files have no text to expand.".to_owned(),
                )),
            }
        };
        let sources = FileSources {
            old: read(&request.old_reference, paths.old_path())?,
            new: read(&request.new_reference, paths.new_path())?,
        };
        let prepared = &request.prepared;
        let doc = prepared
            .document()
            .hydrate_file(0, sources)
            .map_err(invalid)?;
        prepare(
            prepared.key(),
            prepared.generation(),
            prepared.revision(),
            Arc::new(doc),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use diffy_diff::{ContentChange, DiffStats, FileStatus, ObjectId};

    use super::{PipelineServices, compare_result, key_at};
    use crate::contracts::shared::{CompareGeneration, FileRevision};
    use crate::contracts::viewport::pipeline::TextCompareRequest;
    use crate::core::compare::{CompareEntry, CompareOutput, LayoutMode, RendererKind};

    #[test]
    fn compare_results_key_manifest_files_by_position_and_prepare_loaded_documents() {
        let mut output = CompareOutput::from_patch(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-x\n+y\n",
        )
        .unwrap();
        output.push_entry(
            CompareEntry::deferred(Some("b.rs"), Some("c.rs"), FileStatus::Renamed)
                .with_ids(ObjectId::parse("abc1234"), ObjectId::parse("abc1234")),
        );
        let result = compare_result(
            CompareGeneration(4),
            ("left".into(), "right".into()),
            output,
        )
        .unwrap();
        let files = &result.manifest.files;
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].key, key_at(1));
        assert_eq!(files[1].revision, FileRevision(4));
        assert_eq!(files[1].stats, DiffStats::Pending);
        assert_eq!(files[1].content, ContentChange::Unchanged);
        assert!(files[1].summary().is_rename_only());
        assert_eq!(
            result.manifest.file(key_at(1)).unwrap().status,
            FileStatus::Renamed
        );
        assert_eq!(result.prepared.len(), 1);
        assert_eq!(result.prepared[0].key(), key_at(0));
        assert_eq!(result.prepared[0].stats().known(), Some((1, 1)));
        assert_eq!(result.entries.len(), 2);
    }

    #[test]
    fn text_compares_prepare_a_document_even_without_changes() {
        let request = |left: &str, right: &str| TextCompareRequest {
            generation: CompareGeneration(2),
            left_text: Arc::from(left),
            right_text: Arc::from(right),
            display_path: "note.txt".to_owned(),
            renderer: RendererKind::Builtin,
            layout: LayoutMode::Unified,
        };
        let changed = PipelineServices
            .run_text_compare(&request("a\n", "b\n"))
            .unwrap();
        assert_eq!(changed.stats().known(), Some((1, 1)));
        assert_eq!(changed.generation(), CompareGeneration(2));
        let same = PipelineServices
            .run_text_compare(&request("same\n", "same\n"))
            .unwrap();
        assert_eq!(same.stats().known(), Some((0, 0)));
        assert_eq!(
            same.document().text(0, diffy_diff::Side::New).as_str(),
            "same\n"
        );
    }

    struct Phases;

    impl crate::core::compare::ProgressSink for Phases {
        fn phase(&self, _phase: crate::core::compare::ComparePhase) {}
    }

    #[test]
    fn working_copy_compares_publish_a_manifest_and_load_files() {
        use crate::contracts::viewport::pipeline::{
            CompareIntent, CompareJob, CompareWorkPriority, FileLoadRequest, FileTarget,
        };
        use crate::core::vcs::model::{VcsCompareRequest, VcsCompareSpec};

        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        crate::core::compare::service::tests::commit_file(&repo, "src/a.rs", "one\ntwo\n", "a");
        std::fs::write(dir.path().join("src/a.rs"), "one\nthree\n").unwrap();
        let request = VcsCompareRequest {
            spec: VcsCompareSpec::WorkingCopy,
            layout: LayoutMode::Unified,
            renderer: RendererKind::Builtin,
        };
        let job = CompareJob {
            generation: CompareGeneration(7),
            intent: CompareIntent {
                repo_path: dir.path().to_path_buf(),
                request: request.clone(),
            },
        };
        let result = PipelineServices.run_compare(&job, &Phases).unwrap();
        assert_eq!(result.manifest.generation, CompareGeneration(7));
        assert_eq!(result.manifest.files.len(), 1);
        assert_eq!(result.manifest.files[0].paths.display_path(), "src/a.rs");
        assert_eq!(result.prepared[0].stats().known(), Some((1, 1)));
        let loaded = PipelineServices
            .load_file(&FileLoadRequest {
                generation: CompareGeneration(7),
                repo_path: dir.path().to_path_buf(),
                request,
                file: FileTarget {
                    key: key_at(0),
                    revision: FileRevision(7),
                    entry: result.entries[0].clone(),
                },
                priority: CompareWorkPriority::InteractiveSelectedFile,
            })
            .unwrap();
        assert_eq!(loaded.key(), key_at(0));
        assert_eq!(loaded.revision(), FileRevision(7));
        assert_eq!(
            loaded.document().text(0, diffy_diff::Side::New).line(1),
            Some("three")
        );
    }
}
