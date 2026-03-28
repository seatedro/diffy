use crate::core::compare::backends::{DiffBackend, DifftasticBackend, GitDiffBackend};
use crate::core::compare::spec::{CompareSpec, RendererKind};
use crate::core::diff::FileDiff;
use crate::core::error::{DiffyError, Result};
use crate::core::syntax::DiffSyntaxAnnotator;
use crate::core::text::{TextBuffer, TokenBuffer};
use crate::core::vcs::git::GitService;

#[derive(Debug, Clone, Default)]
pub struct CompareOutput {
    pub files: Vec<FileDiff>,
    pub raw_diff: String,
    pub text_buffer: TextBuffer,
    pub token_buffer: TokenBuffer,
    pub used_fallback: bool,
    pub fallback_message: String,
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
    pub fn compare(&self, spec: &CompareSpec, git: &GitService) -> Result<CompareOutput> {
        if spec.renderer == RendererKind::Builtin {
            let mut output = self.fallback.compare(spec, git)?.ok_or_else(|| {
                DiffyError::General("built-in backend returned no result".to_owned())
            })?;
            annotate_output(&mut output);
            return Ok(output);
        }

        match self.primary.compare(spec, git)? {
            Some(mut output) => {
                annotate_output(&mut output);
                Ok(output)
            }
            None => {
                let mut fallback = self.fallback.compare(spec, git)?.ok_or_else(|| {
                    DiffyError::General("fallback backend returned no result".to_owned())
                })?;
                fallback.used_fallback = true;
                fallback.fallback_message =
                    "difftastic unavailable, fell back to built-in backend".to_owned();
                annotate_output(&mut fallback);
                Ok(fallback)
            }
        }
    }
}

fn annotate_output(output: &mut CompareOutput) {
    if output.files.is_empty() {
        return;
    }
    DiffSyntaxAnnotator::new().annotate_files(
        &mut output.files,
        &mut output.text_buffer,
        &mut output.token_buffer,
    );
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Repository, Signature};
    use tempfile::TempDir;

    use super::CompareService;
    use crate::core::compare::spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};
    use crate::core::text::SyntaxTokenKind;
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
    fn compare_service_annotates_syntax_tokens_for_rendering() {
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
            )
            .unwrap();

        let has_non_normal_syntax = output
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .flat_map(|hunk| hunk.lines.iter())
            .flat_map(|line| output.token_buffer.view(line.syntax_tokens).iter())
            .any(|token| token.kind != SyntaxTokenKind::Normal);

        assert!(has_non_normal_syntax);
    }
}
