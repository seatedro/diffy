use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use diffy_compare::{FileDiff, FileInput, Observer, Options};
#[cfg(feature = "difftastic")]
use diffy_diff::{ChangeIntensity, FileMeta, SemanticSpan, TextStore};
use diffy_diff::{DiffDocument, FileStatus};
#[cfg(feature = "difftastic")]
use diffy_semantic::Row;
#[cfg(feature = "difftastic")]
use vendored_difftastic::{
    ChangeIntensity as DftIntensity, ChangeSpan, DiffRequest, DiffStatus, SemanticDiffResult,
};

use crate::core::compare::git::resolve_compare_refs;
use crate::core::compare::output::{CompareEntry, CompareOutput};
use crate::core::compare::progress::{ComparePhase, ProgressSink};
use crate::core::compare::spec::CompareSpec;
use crate::core::compare::stats::COMPARE_SUMMARY_FILE_LIMIT;
#[cfg(feature = "difftastic")]
use crate::core::error::DiffyError;
use crate::core::error::Result;
use crate::core::syntax::grammar::Grammars;
use crate::core::vcs::git::{
    GitService, INDEX_REF, StatusItem, StatusScope, WORKDIR_REF, mentions,
};

const LOADING_FILE_EMIT_STRIDE: u32 = 16;

pub fn status_from_label(label: &str) -> FileStatus {
    match label.as_bytes().first() {
        Some(b'A') => FileStatus::Added,
        Some(b'D') => FileStatus::Deleted,
        Some(b'R') => FileStatus::Renamed,
        Some(b'C') => FileStatus::Copied,
        _ => FileStatus::Modified,
    }
}

pub fn compare(
    spec: &CompareSpec,
    git: &GitService,
    reporter: Option<&dyn ProgressSink>,
) -> Result<CompareOutput> {
    let (left, right) = resolve_compare_refs(spec, git)?;
    if let Some(r) = reporter {
        r.phase(ComparePhase::EnumeratingChanges);
    }
    let entries = git.diff_name_status(&left, &right, None)?;
    if entries.len() > COMPARE_SUMMARY_FILE_LIMIT {
        return Ok(CompareOutput::deferred(
            entries
                .iter()
                .map(|(status, old, new)| {
                    CompareEntry::deferred(
                        old.as_deref(),
                        new.as_deref(),
                        status_from_label(status),
                    )
                })
                .collect(),
        ));
    }
    let changed = read_changed_paths(git, &left, &right, entries);
    let vet = rename_vet(git, &left, &right);
    compare_changed_paths(changed, reporter, Some(&vet))
}

pub fn compare_path(spec: &CompareSpec, path: &str, git: &GitService) -> Result<CompareOutput> {
    let (left, right) = resolve_compare_refs(spec, git)?;
    let entries = git.diff_name_status(&left, &right, Some(path))?;
    let changed = read_changed_paths(git, &left, &right, entries);
    let vet = rename_vet(git, &left, &right);
    compare_changed_paths(changed, None, Some(&vet))
}

pub fn compare_status_item(item: &StatusItem, git: &GitService) -> Result<CompareOutput> {
    let (old_ref, new_ref) = match item.scope {
        StatusScope::Staged => (Some("HEAD"), INDEX_REF),
        StatusScope::Unstaged => (Some(INDEX_REF), WORKDIR_REF),
        StatusScope::Untracked => (None, WORKDIR_REF),
    };
    let old_path = item.old_path.as_deref().unwrap_or(&item.path);
    let old_content =
        old_ref.and_then(|reference| git.read_file_bytes_at(reference, old_path).ok());
    let deleted = item.status == "D" && item.scope != StatusScope::Untracked;
    let new_content = (!deleted)
        .then(|| git.read_file_bytes_at(new_ref, &item.path).ok())
        .flatten();
    let status = match (&old_content, &new_content) {
        (None, _) => FileStatus::Added,
        (_, None) => FileStatus::Deleted,
        _ => status_from_label(&item.status),
    };
    let changed = FileInput {
        status,
        old_path: old_content.as_ref().map(|_| old_path.to_owned()),
        new_path: new_content.as_ref().map(|_| item.path.clone()),
        old: old_content.unwrap_or_default(),
        new: new_content.unwrap_or_default(),
    };
    let vet = rename_vet(git, old_ref.unwrap_or(new_ref), new_ref);
    compare_changed_paths(vec![changed], None, Some(&vet))
}

pub fn compare_text(left: &str, right: &str, display_path: &str) -> Result<CompareOutput> {
    if left == right {
        return Ok(CompareOutput::default());
    }
    let status = if left.is_empty() {
        FileStatus::Added
    } else if right.is_empty() {
        FileStatus::Deleted
    } else {
        FileStatus::Modified
    };
    let changed = FileInput {
        status,
        old_path: (status != FileStatus::Added).then(|| display_path.to_owned()),
        new_path: (status != FileStatus::Deleted).then(|| display_path.to_owned()),
        old: left.as_bytes().to_vec(),
        new: right.as_bytes().to_vec(),
    };
    compare_changed_paths(vec![changed], None, None)
}

fn rename_vet<'g>(
    git: &GitService,
    left: &'g str,
    right: &'g str,
) -> impl Fn(&[(&str, &str)]) -> Vec<bool> + Sync + 'g {
    static SLOW_REPOS: Mutex<Vec<std::path::PathBuf>> = Mutex::new(Vec::new());
    let repo = std::path::PathBuf::from(git.repo_path());
    let known: Mutex<HashMap<(bool, String), bool>> = Mutex::default();
    move |pairs| {
        let absent = |in_right: bool, word: &str| {
            let key = (in_right, word.to_owned());
            if let Some(&absent) = known
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&key)
            {
                return absent;
            }
            let slow = SLOW_REPOS.lock().unwrap_or_else(PoisonError::into_inner);
            if slow.contains(&repo) {
                return false;
            }
            drop(slow);
            let rev = if in_right { right } else { left };
            match mentions(&repo, rev, word) {
                Some(found) => {
                    known
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(key, !found);
                    !found
                }
                None => {
                    let mut slow = SLOW_REPOS.lock().unwrap_or_else(PoisonError::into_inner);
                    if !slow.contains(&repo) {
                        tracing::info!(target: "diffy::energy", repo = %repo.display(), "rename vetting disabled after a failed lookup");
                        slow.push(repo.clone());
                    }
                    false
                }
            }
        };
        let absent = &absent;
        std::thread::scope(|scope| {
            let lookups: Vec<_> = pairs
                .iter()
                .map(|&(old, new)| {
                    let gone = scope.spawn(move || absent(true, old));
                    let fresh = scope.spawn(move || absent(false, new));
                    (gone, fresh)
                })
                .collect();
            lookups
                .into_iter()
                .map(|(gone, fresh)| gone.join().unwrap_or(false) && fresh.join().unwrap_or(false))
                .collect()
        })
    }
}

/// Per-file energy attribution and throttled `LoadingFiles` progress.
struct LoadingFiles<'a> {
    reporter: Option<&'a dyn ProgressSink>,
}

impl Observer for LoadingFiles<'_> {
    fn file(&self, path: &str, work: &mut dyn FnMut()) {
        let _work = crate::platform::energy::work(crate::platform::energy::Kind::Semantic)
            .detail(path.to_owned());
        work();
    }

    fn progress(&self, seen: u32, total: u32) {
        if let Some(r) = self.reporter
            && (seen.is_multiple_of(LOADING_FILE_EMIT_STRIDE) || seen == total)
        {
            r.phase(ComparePhase::LoadingFiles {
                files_seen: seen,
                files_total: total,
            });
        }
    }
}

pub fn compare_changed_paths(
    changed: Vec<FileInput>,
    reporter: Option<&dyn ProgressSink>,
    vet: Option<&diffy_semantic::Vet<'_>>,
) -> Result<CompareOutput> {
    if let Some(r) = reporter {
        r.phase(ComparePhase::LoadingFiles {
            files_seen: 0,
            files_total: changed.len() as u32,
        });
    }
    let store = Grammars::shared().store();
    let observer = LoadingFiles { reporter };
    let mut options = Options::new(&store);
    options.vet = vet;
    options.observer = Some(&observer);
    let diffs = diffy_compare::diff_files(&changed, &options)?;
    let mut output = CompareOutput::default();
    for (input, diff) in changed.iter().zip(diffs) {
        log_fallback(input, &diff);
        output.push_document(Arc::new(diff.document));
    }
    Ok(output)
}

/// One file's semantic diff, on the calling thread.
pub fn document_from_changed(changed: &FileInput) -> Result<DiffDocument> {
    let store = Grammars::shared().store();
    let diff = diffy_compare::diff_file(changed, &Options::new(&store))?;
    log_fallback(changed, &diff);
    Ok(diff.document)
}

fn log_fallback(changed: &FileInput, diff: &FileDiff) {
    if let Some(reason) = &diff.fallback {
        tracing::info!(
            target: "diffy::semantic",
            path = %changed.display_path(),
            reason = %reason,
            old_bytes = changed.old.len(),
            new_bytes = changed.new.len(),
            "semantic diff fell back to line diff"
        );
    }
}

#[cfg(feature = "difftastic")]
pub fn document_from_difftastic(changed: &FileInput) -> Result<DiffDocument> {
    let (meta, old, new) = match diffy_compare::prepare(changed)? {
        diffy_compare::Prepared::Done(doc) => return Ok(*doc),
        diffy_compare::Prepared::Text { meta, old, new } => (meta, old, new),
    };
    let result = vendored_difftastic::diff_bytes_semantic(DiffRequest {
        display_path: changed.display_path(),
        lhs_path: changed.old_path.as_deref().map(std::path::Path::new),
        rhs_path: changed.new_path.as_deref().map(std::path::Path::new),
        lhs_bytes: old.as_str().as_bytes(),
        rhs_bytes: new.as_str().as_bytes(),
    })
    .map_err(|error| DiffyError::General(format!("difftastic failed: {error}")))?;
    log_line_fallback(changed, &result);
    let doc = match result.status {
        DiffStatus::Binary => diffy_compare::binary_document(meta)?,
        DiffStatus::Changed if !result.chunks.is_empty() => {
            document_from_semantic(meta, old, new, &result)?
        }
        _ => diffy_compare::line_diff(meta, &changed.old, &changed.new)?,
    };
    Ok(doc)
}

#[cfg(feature = "difftastic")]
fn document_from_semantic(
    meta: FileMeta,
    old: TextStore,
    new: TextStore,
    result: &SemanticDiffResult,
) -> Result<DiffDocument> {
    let chunks = sanitize(result, &old, &new);
    let structural =
        result.line_fallback_reason.is_none() && line_fallback_reason(&result.language).is_none();
    let doc = diffy_compare::document_from_rows(
        meta,
        old,
        new,
        chunks,
        structural,
        diffy_compare::Semantics::default(),
    )?;
    Ok(doc)
}

#[cfg(feature = "difftastic")]
fn sanitize(result: &SemanticDiffResult, old: &TextStore, new: &TextStore) -> Vec<Vec<Row>> {
    let side_spans = |store: &TextStore, line: Option<u32>, spans: &[ChangeSpan]| {
        line.and_then(|line| store.display_line(line))
            .map(|text| sanitize_spans(spans, text))
            .unwrap_or_default()
    };
    result
        .chunks
        .iter()
        .map(|chunk| {
            chunk
                .lines
                .iter()
                .map(|line| {
                    let old_line = line.lhs_line.filter(|&l| l < old.line_count());
                    let new_line = line.rhs_line.filter(|&l| l < new.line_count());
                    Row {
                        old: old_line,
                        new: new_line,
                        spans: [
                            side_spans(old, old_line, &line.lhs_changes),
                            side_spans(new, new_line, &line.rhs_changes),
                        ],
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(feature = "difftastic")]
fn sanitize_spans(spans: &[ChangeSpan], text: &str) -> Vec<SemanticSpan> {
    let spans = spans
        .iter()
        .map(|span| SemanticSpan {
            start: span.start_col,
            end: span.end_col,
            intensity: intensity(span.intensity),
        })
        .collect();
    diffy_compare::clean_spans(spans, text)
}

#[cfg(feature = "difftastic")]
fn intensity(intensity: DftIntensity) -> ChangeIntensity {
    match intensity {
        DftIntensity::Novel => ChangeIntensity::Novel,
        DftIntensity::NovelWord => ChangeIntensity::NovelWord,
        DftIntensity::UnchangedContext => ChangeIntensity::UnchangedContext,
    }
}

#[cfg(feature = "difftastic")]
fn line_fallback_reason(language: &str) -> Option<&str> {
    language
        .strip_prefix("Text (")
        .and_then(|reason| reason.strip_suffix(')'))
}

#[cfg(feature = "difftastic")]
fn log_line_fallback(changed: &FileInput, result: &SemanticDiffResult) {
    if let Some(reason) = result
        .line_fallback_reason
        .as_deref()
        .or_else(|| line_fallback_reason(&result.language))
    {
        tracing::info!(
            target: "diffy::difftastic",
            path = %changed.display_path(),
            reason,
            chunks = result.chunks.len(),
            old_bytes = changed.old.len(),
            new_bytes = changed.new.len(),
            "difftastic semantic diff fell back to line diff"
        );
    }
}

fn read_changed_paths(
    git: &GitService,
    left: &str,
    right: &str,
    entries: Vec<(String, Option<String>, Option<String>)>,
) -> Vec<FileInput> {
    let old_paths = entries.iter().filter_map(|(_, old, _)| old.as_deref());
    let mut old_contents = git.read_file_bytes_batch_at(left, old_paths).into_iter();
    let new_paths = entries.iter().filter_map(|(_, _, new)| new.as_deref());
    let mut new_contents = git.read_file_bytes_batch_at(right, new_paths).into_iter();
    entries
        .into_iter()
        .map(|(status, old_path, new_path)| {
            let old_content = old_path
                .as_ref()
                .and_then(|_| old_contents.next().flatten());
            let new_content = new_path
                .as_ref()
                .and_then(|_| new_contents.next().flatten());
            FileInput {
                status: status_from_label(&status),
                old_path,
                new_path,
                old: old_content.unwrap_or_default(),
                new: new_content.unwrap_or_default(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use diffy_diff::{FileStatus, Side};

    use super::{compare, compare_path};
    use crate::core::compare::service::tests::commit_file;
    use crate::core::compare::spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};
    use crate::core::vcs::git::{GitService, WORKDIR_REF};

    fn repo_spec(left: &str, right: &str) -> CompareSpec {
        CompareSpec {
            mode: CompareMode::TwoDot,
            left_ref: left.to_owned(),
            right_ref: right.to_owned(),
            renderer: RendererKind::Semantic,
            layout: LayoutMode::Unified,
        }
    }

    #[test]
    fn workdir_compares_read_current_files_and_filter_paths() {
        if diffy_syntax::testing::store_with("rust").is_none() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        commit_file(&repo, "src/other.rs", "stay\n", "other");
        let head = commit_file(
            &repo,
            "src/lib.rs",
            "fn answer() {\n    old();\n}\n",
            "initial",
        );
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "fn answer() {\n    new();\n}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/other.rs"), "changed\n").unwrap();
        let mut service = GitService::new();
        service.open(dir.path().to_str().unwrap()).unwrap();
        let output = compare(&repo_spec(&head, WORKDIR_REF), &service, None).unwrap();
        assert_eq!(output.file_count(), 2);
        let lib = output
            .entries
            .iter()
            .position(|entry| entry.path() == "src/lib.rs")
            .unwrap();
        let doc = output.document(lib).unwrap();
        assert_eq!(doc.text(0, Side::Old).line(1), Some("    old();"));
        assert_eq!(doc.text(0, Side::New).line(1), Some("    new();"));
        assert_eq!(doc.stats(0).known(), Some((1, 1)));
        assert!(!doc.semantic_spans(0, Side::New, 1).is_empty());
        let one = compare_path(&repo_spec(&head, WORKDIR_REF), "src/lib.rs", &service).unwrap();
        assert_eq!(one.file_count(), 1);
        assert_eq!(one.entry(0).unwrap().path(), "src/lib.rs");
    }

    #[test]
    fn added_files_in_a_range_are_whole_file_changes() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let base = commit_file(&repo, "README.md", "base\n", "initial");
        let head = commit_file(&repo, "src/new.rs", "fn new() {}\nlet x = 1;\n", "add");
        let mut service = GitService::new();
        service.open(dir.path().to_str().unwrap()).unwrap();
        let output = compare(&repo_spec(&base, &head), &service, None).unwrap();
        let entry = output.entry(0).unwrap();
        assert_eq!(entry.status(), FileStatus::Added);
        assert_eq!(entry.stats.known(), Some((2, 0)));
    }
}
