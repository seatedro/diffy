use crate::core::compare::backends::{DiffBackend, DifftasticBackend, GitDiffBackend};
use crate::core::compare::progress::ProgressSink;
use crate::core::compare::spec::{CompareSpec, RendererKind};
use crate::core::compare::stats::{
    CompareFileStatsTarget, CompareFileSummary, compact_summary_paths,
};
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::git::GitService;

#[derive(Debug, Clone, Default)]
pub struct CompareOutput {
    pub carbon: carbon::DiffDocument,
    pub file_summaries: Vec<CompareFileSummary>,
    pub raw_diff: String,
    pub used_fallback: bool,
    pub fallback_message: String,
}

impl CompareOutput {
    pub fn file_count(&self) -> usize {
        if self.file_summaries.is_empty() {
            self.carbon.files.len()
        } else {
            self.file_summaries.len()
        }
    }

    pub fn summary_at(&self, index: usize) -> Option<CompareFileSummary> {
        self.file_summaries.get(index).cloned().or_else(|| {
            self.carbon
                .files
                .get(index)
                .map(CompareFileSummary::from_file)
        })
    }

    pub fn deferred_stats_target_at(&self, index: usize) -> Option<CompareFileStatsTarget> {
        let summary = self.summary_at(index)?;
        summary.stats_deferred.then(|| summary.stats_target())
    }

    pub fn for_each_deferred_stats_target(
        &self,
        limit: usize,
        mut visit: impl FnMut(usize, CompareFileStatsTarget),
    ) {
        if limit == 0 {
            return;
        }
        let mut visited = 0;
        if self.file_summaries.is_empty() {
            for (index, file) in self.carbon.files.iter().enumerate() {
                let summary = CompareFileSummary::from_file(file);
                if !summary.stats_deferred {
                    continue;
                }
                visit(index, summary.stats_target());
                visited += 1;
                if visited >= limit {
                    break;
                }
            }
        } else {
            for (index, summary) in self.file_summaries.iter().enumerate() {
                if !summary.stats_deferred {
                    continue;
                }
                visit(index, summary.stats_target());
                visited += 1;
                if visited >= limit {
                    break;
                }
            }
        }
    }

    pub fn for_each_summary(&self, mut visit: impl FnMut(usize, &CompareFileSummary)) {
        if self.file_summaries.is_empty() {
            for (index, file) in self.carbon.files.iter().enumerate() {
                let summary = CompareFileSummary::from_file(file);
                visit(index, &summary);
            }
        } else {
            for (index, summary) in self.file_summaries.iter().enumerate() {
                visit(index, summary);
            }
        }
    }

    pub fn for_each_path(&self, mut visit: impl FnMut(usize, &str)) {
        if self.file_summaries.is_empty() {
            for (index, file) in self.carbon.files.iter().enumerate() {
                visit(index, file.path());
            }
        } else {
            let mut scratch = String::new();
            for (index, summary) in self.file_summaries.iter().enumerate() {
                scratch.clear();
                summary.push_path_to(&mut scratch);
                visit(index, &scratch);
            }
        }
    }

    pub fn max_path_chars(&self) -> usize {
        if self.file_summaries.is_empty() {
            self.carbon
                .files
                .iter()
                .map(|file| file.path().chars().count())
                .max()
                .unwrap_or(0)
        } else {
            self.file_summaries
                .iter()
                .map(CompareFileSummary::path_chars)
                .max()
                .unwrap_or(0)
        }
    }

    pub fn compact_file_summaries(&mut self) {
        compact_summary_paths(&mut self.file_summaries);
        self.file_summaries.shrink_to_fit();
    }
}

pub struct CompareService {
    primary: Box<dyn DiffBackend>,
    fallback: Box<dyn DiffBackend>,
}

impl Default for CompareService {
    fn default() -> Self {
        Self {
            primary: Box::new(DifftasticBackend),
            fallback: Box::new(GitDiffBackend),
        }
    }
}

impl CompareService {
    pub fn compare(
        &self,
        spec: &CompareSpec,
        git: &GitService,
        reporter: Option<&dyn ProgressSink>,
    ) -> Result<CompareOutput> {
        if spec.renderer == RendererKind::Builtin {
            return self.fallback.compare(spec, git, reporter)?.ok_or_else(|| {
                DiffyError::General("built-in backend returned no result".to_owned())
            });
        }

        match self.primary.compare(spec, git, reporter)? {
            Some(output) => Ok(output),
            None => {
                let mut fallback =
                    self.fallback.compare(spec, git, reporter)?.ok_or_else(|| {
                        DiffyError::General("fallback backend returned no result".to_owned())
                    })?;
                fallback.used_fallback = true;
                fallback.fallback_message =
                    "difftastic unavailable, fell back to built-in backend".to_owned();
                Ok(fallback)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Repository, Signature};
    use tempfile::TempDir;

    use super::CompareService;
    use crate::core::compare::spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};
    use crate::core::vcs::git::GitService;

    fn commit_file(repo: &Repository, relative_path: &str, content: &str, message: &str) -> String {
        let workdir = repo.workdir().expect("repo workdir");
        let full_path = workdir.join(relative_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full_path, content).unwrap();

        let mut index = repo.index().unwrap();
        index.add_path(Path::new(relative_path)).unwrap();
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = Signature::now("Diffy", "diffy@example.com").unwrap();
        let parents = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| repo.find_commit(oid).unwrap())
            .into_iter()
            .collect::<Vec<_>>();
        let parent_refs = parents.iter().collect::<Vec<_>>();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parent_refs,
        )
        .unwrap()
        .to_string()
    }

    #[test]
    fn compare_service_defers_syntax_annotation_until_file_selection() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let _first = commit_file(
            &repo,
            "src/lib.rs",
            "fn answer() -> i32 {\n    1\n}\n",
            "initial",
        );
        let second = commit_file(
            &repo,
            "src/lib.rs",
            "fn answer() -> i32 {\n    2\n}\n",
            "second",
        );

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();

        let output = CompareService::default()
            .compare(
                &CompareSpec {
                    mode: CompareMode::SingleCommit,
                    left_ref: second,
                    right_ref: String::new(),
                    renderer: RendererKind::Builtin,
                    layout: LayoutMode::Unified,
                },
                &git,
                None,
            )
            .unwrap();

        assert!(
            output.carbon.files[0]
                .blocks
                .iter()
                .all(|block| { block.old_inline.is_empty() && block.new_inline.is_empty() })
        );
    }
}
