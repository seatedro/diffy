use std::sync::Arc;

use diffy_diff::DiffDocument;

use crate::core::compare::git;
use crate::core::compare::output::{CompareEntry, CompareOutput};
use crate::core::compare::progress::ProgressSink;
use crate::core::compare::spec::{CompareSpec, RendererKind};
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::git::GitService;

#[derive(Debug, Default, Clone, Copy)]
pub struct CompareService;

impl CompareService {
    pub fn compare(
        &self,
        spec: &CompareSpec,
        repo: &GitService,
        reporter: Option<&dyn ProgressSink>,
    ) -> Result<CompareOutput> {
        if spec.renderer == RendererKind::Semantic {
            return crate::core::compare::semantic::compare(spec, repo, reporter);
        }
        git::compare(spec, repo, reporter)
    }

    pub fn compare_file(
        &self,
        spec: &CompareSpec,
        entry: &CompareEntry,
        repo: &GitService,
    ) -> Result<Arc<DiffDocument>> {
        let path = entry.path().into_owned();
        if spec.renderer == RendererKind::Semantic && !entry.is_binary() {
            let output = crate::core::compare::semantic::compare_path(spec, &path, repo)?;
            return matching_document(output, entry);
        }
        if let Some(doc) = git::compare_entry(entry, repo)? {
            return Ok(Arc::new(doc));
        }
        let renames = !entry.stats.is_pending();
        matching_document(git::compare_path(spec, &path, repo, renames)?, entry)
    }
}

pub fn matching_document(output: CompareOutput, entry: &CompareEntry) -> Result<Arc<DiffDocument>> {
    let (old, new) = (entry.paths.old_path(), entry.paths.new_path());
    let index = output
        .entries
        .iter()
        .position(|candidate| {
            candidate.paths.new_path() == new && candidate.paths.old_path() == old
        })
        .or_else(|| {
            output.entries.iter().position(|candidate| {
                (new.is_some() && candidate.paths.new_path() == new)
                    || (old.is_some() && candidate.paths.old_path() == old)
            })
        })
        .or_else(|| (output.entries.len() == 1).then_some(0));
    index
        .and_then(|index| output.document(index).cloned())
        .ok_or_else(|| DiffyError::General(format!("no diff for {}", entry.path())))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::path::Path;

    use diffy_diff::{BlockKind, DiffStats, FileStatus, Side};
    use git2::{Oid, Repository, Signature};
    use tempfile::TempDir;

    use super::CompareService;
    use crate::core::compare::git;
    use crate::core::compare::output::{CompareEntry, CompareOutput};
    use crate::core::compare::spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};
    use crate::core::vcs::git::{GitService, WORKDIR_REF};

    pub(crate) fn commit_file(
        repo: &Repository,
        path: &str,
        content: &str,
        message: &str,
    ) -> String {
        let workdir = repo.workdir().expect("repo workdir");
        let full_path = workdir.join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full_path, content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(path)).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = Signature::now("Diffy", "diffy@example.com").unwrap();
        let parents = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| repo.find_commit(oid).unwrap())
            .into_iter()
            .collect::<Vec<_>>();
        let parents = parents.iter().collect::<Vec<_>>();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .unwrap()
        .to_string()
    }

    fn checkout_branch(repo: &Repository, branch: &str) {
        repo.set_head(&format!("refs/heads/{branch}")).unwrap();
        let head = repo.revparse_single("HEAD").unwrap();
        repo.checkout_tree(&head, None).unwrap();
    }

    fn open(dir: &TempDir) -> GitService {
        let mut service = GitService::new();
        service.open(dir.path().to_str().unwrap()).unwrap();
        service
    }

    fn spec(mode: CompareMode, left: &str, right: &str, renderer: RendererKind) -> CompareSpec {
        CompareSpec {
            mode,
            left_ref: left.to_owned(),
            right_ref: right.to_owned(),
            renderer,
            layout: LayoutMode::Unified,
        }
    }

    fn changed_lines(output: &CompareOutput, index: usize) -> (String, String) {
        let doc = output.document(index).expect("document");
        let b = doc.blocks();
        let block = (0..b.kind.len())
            .find(|&i| b.kind[i] == BlockKind::Change)
            .expect("change block");
        let line = |side: Side, store: u32| doc.text(0, side).line(store).unwrap_or("").to_owned();
        (
            line(Side::Old, b.old_store[block]),
            line(Side::New, b.new_store[block]),
        )
    }

    #[test]
    fn builtin_compare_resolves_single_commits() {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_file(&repo, "src/example.rs", "before\n", "initial");
        let second = commit_file(&repo, "src/example.rs", "after\n", "second");
        let output = CompareService
            .compare(
                &spec(
                    CompareMode::SingleCommit,
                    &second,
                    "",
                    RendererKind::Builtin,
                ),
                &open(&dir),
                None,
            )
            .unwrap();
        assert_eq!(output.file_count(), 1);
        assert_eq!(output.entry(0).unwrap().path(), "src/example.rs");
        assert!(output.entry(0).unwrap().old_id.is_some());
        assert_eq!(changed_lines(&output, 0), ("before".into(), "after".into()));
        assert!(!output.used_fallback);
    }

    #[test]
    fn builtin_compare_uses_the_merge_base_for_three_dot_ranges() {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let base = commit_file(&repo, "src/example.rs", "start\n", "initial");
        let base_commit = repo.find_commit(Oid::from_str(&base).unwrap()).unwrap();
        repo.branch("feature", &base_commit, false).unwrap();
        checkout_branch(&repo, "feature");
        let feature = commit_file(&repo, "src/example.rs", "feature\n", "feature");
        checkout_branch(&repo, "master");
        let master = commit_file(&repo, "src/example.rs", "master\n", "master");
        let service = open(&dir);
        let output = CompareService
            .compare(
                &spec(
                    CompareMode::ThreeDot,
                    &master,
                    &feature,
                    RendererKind::Builtin,
                ),
                &service,
                None,
            )
            .unwrap();
        assert_eq!(
            changed_lines(&output, 0),
            ("start".into(), "feature".into())
        );

        checkout_branch(&repo, "feature");
        fs::write(repo.workdir().unwrap().join("src/example.rs"), "dirty\n").unwrap();
        let output = CompareService
            .compare(
                &spec(
                    CompareMode::ThreeDot,
                    &master,
                    WORKDIR_REF,
                    RendererKind::Builtin,
                ),
                &service,
                None,
            )
            .unwrap();
        assert_eq!(changed_lines(&output, 0), ("start".into(), "dirty".into()));
    }

    #[test]
    fn deferred_entries_load_stats_and_documents_on_demand() {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let first = commit_file(&repo, "src/a.rs", "same\nold\n", "initial");
        let second = commit_file(&repo, "src/a.rs", "same\nnew\nextra\n", "update");
        let service = open(&dir);
        let spec = spec(CompareMode::TwoDot, &first, &second, RendererKind::Builtin);
        let id = |rev: &str| {
            diffy_diff::ObjectId::parse(
                &repo
                    .revparse_single(&format!("{rev}:src/a.rs"))
                    .unwrap()
                    .id()
                    .to_string(),
            )
        };
        let entry =
            CompareEntry::deferred(Some("src/a.rs"), Some("src/a.rs"), FileStatus::Modified)
                .with_ids(id(&first), id(&second));
        assert_eq!(
            git::entry_stats(&spec, &service, std::slice::from_ref(&entry)),
            [Some(DiffStats::Known {
                additions: 2,
                deletions: 1
            })]
        );
        let doc = CompareService
            .compare_file(&spec, &entry, &service)
            .unwrap();
        assert_eq!(doc.stats(0).known(), Some((2, 1)));
        assert_eq!(doc.text(0, Side::New).line(2), Some("extra"));
        let by_path =
            CompareEntry::deferred(Some("src/a.rs"), Some("src/a.rs"), FileStatus::Modified);
        let doc = CompareService
            .compare_file(&spec, &by_path, &service)
            .unwrap();
        assert_eq!(doc.stats(0).known(), Some((2, 1)));
    }

    #[test]
    fn single_path_compares_return_that_file() {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let first = commit_file(&repo, "src/a.rs", "before\n", "initial a");
        commit_file(&repo, "src/b.rs", "before\n", "initial b");
        commit_file(&repo, "src/a.rs", "after\n", "update a");
        let last = commit_file(&repo, "src/b.rs", "after\n", "update b");
        let service = open(&dir);
        let spec = spec(CompareMode::TwoDot, &first, &last, RendererKind::Builtin);
        for renames in [true, false] {
            let output = git::compare_path(&spec, "src/a.rs", &service, renames).unwrap();
            assert_eq!(output.file_count(), 1);
            assert_eq!(output.entry(0).unwrap().path(), "src/a.rs");
        }
    }

    #[test]
    fn renames_and_mode_changes_keep_their_facts() {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let body = "one\ntwo\nthree\nfour\nfive\nsix\n";
        commit_file(&repo, "src/old_name.rs", body, "initial");
        let first = commit_file(&repo, "run.sh", "echo\n", "script");
        let workdir = repo.workdir().unwrap().to_path_buf();
        fs::rename(
            workdir.join("src/old_name.rs"),
            workdir.join("src/new_name.rs"),
        )
        .unwrap();
        let mut index = repo.index().unwrap();
        index.remove_path(Path::new("src/old_name.rs")).unwrap();
        index.add_path(Path::new("src/new_name.rs")).unwrap();
        let mut entry = index.get_path(Path::new("run.sh"), 0).unwrap();
        entry.mode = 0o100755;
        index.add(&entry).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = Signature::now("Diffy", "diffy@example.com").unwrap();
        let parent = repo.find_commit(Oid::from_str(&first).unwrap()).unwrap();
        let second = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "move",
                &tree,
                &[&parent],
            )
            .unwrap()
            .to_string();
        let output = CompareService
            .compare(
                &spec(CompareMode::TwoDot, &first, &second, RendererKind::Builtin),
                &open(&dir),
                None,
            )
            .unwrap();
        let find = |path: &str| {
            output
                .entries
                .iter()
                .position(|entry| entry.path() == path)
                .expect(path)
        };
        let renamed = output.entry(find("src/new_name.rs")).unwrap();
        assert_eq!(renamed.status(), FileStatus::Renamed);
        assert_eq!(renamed.paths.old_path().as_deref(), Some("src/old_name.rs"));
        assert!(renamed.summary.is_rename_only());
        assert_eq!(renamed.stats.known(), Some((0, 0)));
        let mode = output.entry(find("run.sh")).unwrap();
        assert!(mode.summary.is_mode_only());
        assert_eq!(mode.old_mode.as_deref(), Some("100644"));
        assert_eq!(mode.new_mode.as_deref(), Some("100755"));
    }

    #[test]
    fn eof_and_crlf_changes_are_line_changes() {
        let output = git::compare_text("a\r\nb", "a\nb\n", "w.txt").unwrap();
        let doc = output.document(0).unwrap();
        assert_eq!(doc.stats(0).known(), Some((2, 2)));
        assert!(doc.facts(0).old_missing_newline);
        assert!(!doc.facts(0).new_missing_newline);
        assert!(
            git::compare_text("same\n", "same\n", "w.txt")
                .unwrap()
                .entries
                .is_empty()
        );
        let binary = git::compare_text("a\0b", "c", "bin").unwrap();
        assert!(binary.entry(0).unwrap().is_binary());
    }
}
