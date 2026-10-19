use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use diffy_diff::{
    DiffDocument, DocumentBuilder, FileDiffSnapshot, FileMeta, FileSources, FileStatus,
    Placeholder, Revision, SlotState, SourceCoverage, TextStore,
};

use crate::apprt::services::open_compare_repository;
use crate::contracts::shared::{CompareGeneration, FileKey, FileRevision};
use crate::contracts::viewport::interaction::ViewportInstall;
use crate::contracts::viewport::pipeline::{
    CompareJob, CompareManifestSnapshot, CompareResult, DiffSize, DiffSizeRequest, FileLoadRequest,
    FileStat, FileStatsRequest, ManifestFile, PipelineFailure, PipelineInstall, Preparation,
    PreparedDiff, SlotUpdate, SourceLoadRequest, StatusDiffRequest, TextCompareRequest,
    TotalStatsRequest, WORKING_SET_FILES, WORKING_SET_LINES,
};
use crate::core::compare::git::{compare_worktree, compare_worktree_entry};
use crate::core::compare::service::matching_document;
use crate::core::compare::{
    CompareEntry, CompareOutput, ComparePhase, ProgressSink, RendererKind, compare_text,
};
use crate::core::compare::{CompareMode, LayoutMode};
use crate::core::error::DiffyError;
use crate::core::vcs::git::GitService;
use crate::core::vcs::model::RevisionId;
use crate::core::vcs::model::{VcsCompareRequest, VcsCompareSpec, VcsKind};
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

pub fn estimated_lines(stats: diffy_diff::DiffStats) -> u32 {
    stats.known().map_or(1, |(additions, deletions)| {
        additions.saturating_add(deletions).saturating_add(1)
    })
}

pub fn stable_key(entry: &CompareEntry) -> FileKey {
    let mut hasher = std::hash::DefaultHasher::new();
    entry.bucket.map(|bucket| bucket as u8).hash(&mut hasher);
    entry.paths.old_path().hash(&mut hasher);
    entry.paths.new_path().hash(&mut hasher);
    FileKey(hasher.finish() | 1 << 63)
}

pub fn manifest_keys(entries: &[CompareEntry]) -> Vec<FileKey> {
    let mut used = HashSet::with_capacity(entries.len());
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut key = match entry.bucket {
                Some(_) => stable_key(entry),
                None => key_at(index),
            };
            while !used.insert(key) {
                key = FileKey(key.0.wrapping_add(1) | 1 << 63);
            }
            key
        })
        .collect()
}

pub fn compare_result(
    generation: CompareGeneration,
    resolved: (String, String),
    output: CompareOutput,
    preparation: Option<Preparation>,
) -> Result<CompareResult, PipelineFailure> {
    let revision = file_revision(generation);
    let keys = manifest_keys(&output.entries);
    let key_at = |index: usize| keys[index];
    let files: Arc<[ManifestFile]> = output
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut file = ManifestFile::from_entry(key_at(index), revision, entry);
            file.buckets.extend(entry.bucket);
            if let Some(semantics) = output.document(index).and_then(|doc| doc.semantics(0)) {
                file.outline = semantics.outline().into();
            }
            file
        })
        .collect();
    let positions: HashMap<FileKey, usize> = keys
        .iter()
        .enumerate()
        .map(|(index, &key)| (key, index))
        .collect();
    let mut loaded = Vec::new();
    let mut placeholders = HashMap::with_capacity(keys.len());
    let mut installs = Vec::with_capacity(keys.len() + 1);
    let mut budget = (0u64, 0usize);
    for (index, entry) in output.entries.iter().enumerate() {
        let key = key_at(index);
        if let Some(doc) = output.document(index) {
            let file = prepare(key, generation, revision, doc.clone())?;
            let lines = u64::from(file.estimated_lines());
            if budget.0 + lines <= WORKING_SET_LINES && budget.1 < WORKING_SET_FILES {
                budget = (budget.0 + lines, budget.1 + 1);
                let file = file.with_segment(preparation);
                loaded.push(file.clone());
                installs.push(PipelineInstall::View(ViewportInstall::File {
                    prepared: file,
                    remap: None,
                }));
                continue;
            }
            let placeholder = file.evicted_placeholder();
            installs.push(PipelineInstall::Slot(SlotUpdate::Placeholder {
                key,
                placeholder: placeholder.clone(),
            }));
            placeholders.insert(key, placeholder);
            continue;
        }
        let placeholder = Placeholder::new(
            Revision(revision.0),
            SlotState::Unloaded,
            estimated_lines(entry.stats),
            entry.file_meta(),
        );
        installs.push(PipelineInstall::Slot(SlotUpdate::Placeholder {
            key,
            placeholder: placeholder.clone(),
        }));
        placeholders.insert(key, placeholder);
    }
    installs.push(PipelineInstall::View(ViewportInstall::Order {
        generation,
        files: keys.iter().copied().collect(),
    }));
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
            totals: output.totals(),
            working_tree: false,
        }),
        entries: output.entries.into(),
        positions,
        loaded,
        placeholders,
        installs,
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

pub const DIFF_SIZE_COMMIT_LIMIT: usize = 1_000;

fn is_git_worktree(kind: VcsKind, spec: &VcsCompareSpec) -> bool {
    kind == VcsKind::GIT && matches!(spec, VcsCompareSpec::WorkingCopy)
}

fn open_git(path: &std::path::Path) -> Result<GitService, PipelineFailure> {
    let mut git = GitService::new();
    git.open(&path.to_string_lossy()).map_err(failed)?;
    Ok(git)
}

fn worktree_document(
    path: &std::path::Path,
    entry: &CompareEntry,
    renderer: RendererKind,
) -> Result<Arc<DiffDocument>, PipelineFailure> {
    let git = open_git(path)?;
    compare_worktree_entry(&git, entry, renderer)
        .map_err(failed)?
        .map(Arc::new)
        .ok_or_else(|| PipelineFailure::Failed("No changes in this file.".to_owned()))
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
        let output = if is_git_worktree(repo.location().kind, &request.spec) {
            let git = open_git(&job.intent.repo_path)?;
            compare_worktree(&git, request.renderer, Some(progress)).map_err(failed)?
        } else {
            repo.compare(request, Some(progress)).map_err(failed)?
        };
        compare_result(job.generation, resolved, output, job.preparation)
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

    pub fn diff_size(&self, request: &DiffSizeRequest) -> Result<DiffSize, PipelineFailure> {
        let refs = [request.left.as_str(), request.right.as_str()];
        let mut repo = open_compare_repository(&request.repo_path, refs).map_err(failed)?;
        let spec = match request.mode {
            CompareMode::ThreeDot => VcsCompareSpec::MergeBaseRange {
                base: request.left.clone(),
                head: request.right.clone(),
            },
            CompareMode::TwoDot | CompareMode::SingleCommit => VcsCompareSpec::Range {
                from: request.left.clone(),
                to: request.right.clone(),
            },
        };
        let compare = VcsCompareRequest {
            spec,
            layout: LayoutMode::Unified,
            renderer: RendererKind::Builtin,
        };
        let (additions, deletions) = repo.compare_stats(&compare).map_err(failed)?;
        let history = repo
            .compare_history(&request.left, &request.right, DIFF_SIZE_COMMIT_LIMIT + 1)
            .map_err(failed)?;
        let files = if repo.location().kind == VcsKind::GIT {
            let (left, right) = repo.resolve_compare_request(&compare).map_err(failed)?;
            let git = open_git(&request.repo_path)?;
            let entries = git.diff_name_status(&left, &right, None).map_err(failed)?;
            Some(u32::try_from(entries.len()).unwrap_or(u32::MAX))
        } else {
            None
        };
        Ok(DiffSize {
            files,
            additions: u32::try_from(additions).unwrap_or(0),
            deletions: u32::try_from(deletions).unwrap_or(0),
            commits: history.len().min(DIFF_SIZE_COMMIT_LIMIT) as u32,
            commits_capped: history.len() > DIFF_SIZE_COMMIT_LIMIT,
        })
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
        if entry.bucket.is_some() {
            let doc = worktree_document(&request.repo_path, entry, request.request.renderer)?;
            return prepare(
                request.file.key,
                request.generation,
                request.file.revision,
                doc,
            )
            .map(|prepared| prepared.with_segment(request.preparation));
        }
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
        .map(|prepared| prepared.with_segment(request.preparation))
    }

    pub fn load_file_stats(
        &self,
        request: &FileStatsRequest,
    ) -> Result<Vec<FileStat>, PipelineFailure> {
        if request.files.iter().any(|file| file.entry.bucket.is_some()) {
            return request
                .files
                .iter()
                .map(|file| {
                    let doc = worktree_document(
                        &request.repo_path,
                        &file.entry,
                        request.request.renderer,
                    )?;
                    let (additions, deletions) = doc.stats(0).known().unwrap_or((0, 0));
                    Ok(FileStat {
                        key: file.key,
                        additions,
                        deletions,
                    })
                })
                .collect();
        }
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
        .map(|prepared| prepared.with_segment(request.preparation))
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
        .map(|hydrated| hydrated.with_segment(request.preparation))
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
            None,
        )
        .unwrap();
        assert_eq!(result.placeholders.len(), 1);
        assert!(result.placeholders.contains_key(&key_at(1)));
        assert_eq!(result.installs.len(), 3);
        assert_eq!(result.positions[&key_at(1)], 1);
        assert_eq!(result.manifest.totals, DiffStats::Pending);
        let files = &result.manifest.files;
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].key, key_at(1));
        assert_eq!(files[1].revision, FileRevision(4));
        assert_eq!(files[1].stats, DiffStats::Pending);
        assert_eq!(result.entries[1].summary.content, ContentChange::Unchanged);
        assert!(result.entries[1].summary.is_rename_only());
        assert_eq!(
            result.manifest.file(key_at(1)).unwrap().status,
            FileStatus::Renamed
        );
        assert_eq!(result.loaded.len(), 1);
        assert_eq!(result.loaded[0].key(), key_at(0));
        assert_eq!(result.loaded[0].stats().known(), Some((1, 1)));
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
            preparation: Some(crate::contracts::viewport::pipeline::Preparation {
                mode: diffy_diff::Mode::Unified,
                policy: diffy_diff::ContextPolicy::default(),
                comparison: diffy_diff::ComparisonOptions::default(),
            }),
        };
        let result = PipelineServices.run_compare(&job, &Phases).unwrap();
        assert_eq!(result.manifest.generation, CompareGeneration(7));
        assert_eq!(result.manifest.files.len(), 1);
        assert_eq!(result.manifest.files[0].paths.display_path(), "src/a.rs");
        assert_eq!(result.loaded[0].stats().known(), Some((1, 1)));
        assert!(result.loaded[0].segment().is_some());
        assert_eq!(result.manifest.totals.known(), Some((1, 1)));
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
                preparation: None,
            })
            .unwrap();
        assert!(loaded.segment().is_none());
        assert_eq!(loaded.key(), key_at(0));
        assert_eq!(loaded.revision(), FileRevision(7));
        assert_eq!(
            loaded.document().text(0, diffy_diff::Side::New).line(1),
            Some("three")
        );
    }

    #[test]
    fn git_working_copies_list_each_bucket_with_its_own_pair_and_stable_key() {
        use crate::contracts::viewport::pipeline::{
            CompareIntent, CompareJob, CompareResult, CompareWorkPriority, FileLoadRequest,
            FileTarget, ManifestFile,
        };
        use crate::core::vcs::model::{ChangeBucket, VcsCompareRequest, VcsCompareSpec};

        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        crate::core::compare::service::tests::commit_file(&repo, "a.rs", "one\n", "a");
        std::fs::write(dir.path().join("a.rs"), "two\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("a.rs")).unwrap();
        index.write().unwrap();
        std::fs::write(dir.path().join("a.rs"), "three\n").unwrap();
        std::fs::write(dir.path().join("new.txt"), "fresh\n").unwrap();
        let blob = |text: &str| repo.blob(text.as_bytes()).unwrap().to_string();

        for renderer in [RendererKind::Builtin, RendererKind::Semantic] {
            let request = VcsCompareRequest {
                spec: VcsCompareSpec::WorkingCopy,
                layout: LayoutMode::Unified,
                renderer,
            };
            let run = |generation| {
                PipelineServices
                    .run_compare(
                        &CompareJob {
                            generation: CompareGeneration(generation),
                            intent: CompareIntent {
                                repo_path: dir.path().to_path_buf(),
                                request: request.clone(),
                            },
                            preparation: None,
                        },
                        &Phases,
                    )
                    .unwrap()
            };
            let first = run(1);
            let files = &first.manifest.files;
            let rows: Vec<_> = files
                .iter()
                .map(|file| (file.paths.display_path().into_owned(), file.buckets.clone()))
                .collect();
            assert_eq!(
                rows,
                [
                    ("a.rs".to_owned(), vec![ChangeBucket::Staged]),
                    ("a.rs".to_owned(), vec![ChangeBucket::Unstaged]),
                    ("new.txt".to_owned(), vec![ChangeBucket::Untracked]),
                ],
                "{renderer:?}"
            );
            assert_ne!(files[0].key, files[1].key);
            let lines = |key| {
                let prepared = first.loaded.iter().find(|p| p.key() == key).unwrap();
                let doc = prepared.document();
                let line = |side| {
                    let store = doc.text(0, side);
                    (0..store.line_count())
                        .map(|i| store.line(i).unwrap().to_owned())
                        .collect::<Vec<_>>()
                };
                (line(diffy_diff::Side::Old), line(diffy_diff::Side::New))
            };
            assert_eq!(
                lines(files[0].key),
                (vec!["one".into()], vec!["two".into()])
            );
            assert_eq!(
                lines(files[1].key),
                (vec!["two".into()], vec!["three".into()])
            );
            assert_eq!(lines(files[2].key), (vec![], vec!["fresh".into()]));
            let id = |file: &ManifestFile| {
                (
                    file.old_id.as_ref().map(|id| id.to_string()),
                    file.new_id.as_ref().map(|id| id.to_string()),
                )
            };
            let (staged_old, staged_new) = id(&files[0]);
            assert!(blob("one\n").starts_with(&staged_old.unwrap()));
            assert!(blob("two\n").starts_with(&staged_new.unwrap()));
            let (unstaged_old, _) = id(&files[1]);
            assert!(blob("two\n").starts_with(&unstaged_old.unwrap()));

            let again = run(2);
            let keys = |result: &CompareResult| {
                result
                    .manifest
                    .files
                    .iter()
                    .map(|f| f.key)
                    .collect::<Vec<_>>()
            };
            assert_eq!(keys(&first), keys(&again));

            let unstaged = first.entries[1].clone();
            let loaded = PipelineServices
                .load_file(&FileLoadRequest {
                    generation: CompareGeneration(1),
                    repo_path: dir.path().to_path_buf(),
                    request,
                    file: FileTarget {
                        key: files[1].key,
                        revision: FileRevision(1),
                        entry: unstaged,
                    },
                    priority: CompareWorkPriority::InteractiveSelectedFile,
                    preparation: None,
                })
                .unwrap();
            assert_eq!(
                loaded.document().text(0, diffy_diff::Side::Old).line(0),
                Some("two")
            );
        }
    }

    #[test]
    fn diff_size_counts_files_lines_and_commits_between_refs() {
        use crate::contracts::viewport::pipeline::DiffSizeRequest;
        use crate::core::compare::CompareMode;

        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let commit = crate::core::compare::service::tests::commit_file;
        let base = commit(&repo, "a.rs", "one\n", "a");
        commit(&repo, "a.rs", "one\ntwo\n", "b");
        let head = commit(&repo, "b.rs", "x\ny\n", "c");
        let size = PipelineServices
            .diff_size(&DiffSizeRequest {
                token: 1,
                repo_path: dir.path().to_path_buf(),
                left: base,
                right: head,
                mode: CompareMode::TwoDot,
            })
            .unwrap();
        assert_eq!(size.files, Some(2));
        assert_eq!((size.additions, size.deletions), (3, 0));
        assert_eq!(size.commits, 2);
        assert!(!size.commits_capped);
    }

    #[test]
    fn a_clean_working_copy_compares_to_an_empty_manifest() {
        use crate::contracts::viewport::pipeline::{CompareIntent, CompareJob};
        use crate::core::vcs::model::{VcsCompareRequest, VcsCompareSpec};

        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        crate::core::compare::service::tests::commit_file(&repo, "a.rs", "one\n", "a");
        let result = PipelineServices
            .run_compare(
                &CompareJob {
                    generation: CompareGeneration(3),
                    intent: CompareIntent {
                        repo_path: dir.path().to_path_buf(),
                        request: VcsCompareRequest {
                            spec: VcsCompareSpec::WorkingCopy,
                            layout: LayoutMode::Unified,
                            renderer: RendererKind::Builtin,
                        },
                    },
                    preparation: None,
                },
                &Phases,
            )
            .unwrap();
        assert!(result.manifest.files.is_empty());
        assert_eq!(result.manifest.totals.known(), Some((0, 0)));
        assert_eq!(result.installs.len(), 1);
    }
}
