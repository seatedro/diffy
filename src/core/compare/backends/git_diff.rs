use git2::{Delta, DiffOptions};

use crate::core::compare::backends::DiffBackend;
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::CompareSpec;
use crate::core::diff::{DiffLine, FileDiff, Hunk, LineKind};
use crate::core::error::Result;
use crate::core::text::{DiffTokenSpan, SyntaxTokenKind, TextBuffer, TokenBuffer};
use crate::core::vcs::git::GitService;

#[derive(Debug, Default, Clone, Copy)]
pub struct GitDiffBackend;

impl DiffBackend for GitDiffBackend {
    fn compare(&self, spec: &CompareSpec, git: &GitService) -> Result<Option<CompareOutput>> {
        let repo = match git.repo() {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        let (left, right) = match spec.mode {
            crate::core::compare::spec::CompareMode::TwoDot => (
                git.resolve_ref(&spec.left_ref)?,
                git.resolve_ref(&spec.right_ref)?,
            ),
            crate::core::compare::spec::CompareMode::ThreeDot
            | crate::core::compare::spec::CompareMode::SingleCommit => {
                git.resolve_comparison(&spec.left_ref, &spec.right_ref, spec.mode)?
            }
        };

        let left_commit = repo.find_commit(git2::Oid::from_str(&left)?)?;
        let right_commit = repo.find_commit(git2::Oid::from_str(&right)?)?;
        let left_tree = left_commit.tree()?;
        let right_tree = right_commit.tree()?;

        let mut options = DiffOptions::new();
        options.context_lines(3);
        let mut diff =
            repo.diff_tree_to_tree(Some(&left_tree), Some(&right_tree), Some(&mut options))?;
        diff.find_similar(None)?;

        let mut output = CompareOutput::default();
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        let deltas: Vec<_> = diff.deltas().collect();
        for (delta_idx, delta) in deltas.iter().enumerate() {
            let mut file = FileDiff {
                path: delta
                    .new_file()
                    .path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                status: match delta.status() {
                    Delta::Added => "A".to_owned(),
                    Delta::Deleted => "D".to_owned(),
                    Delta::Renamed => "R".to_owned(),
                    _ => "M".to_owned(),
                },
                is_binary: delta.new_file().is_binary() || delta.old_file().is_binary(),
                ..FileDiff::default()
            };

            if file.is_binary {
                output.files.push(file);
                continue;
            }

            if let Ok(Some(patch)) = git2::Patch::from_diff(&diff, delta_idx) {
                for hunk_idx in 0..patch.num_hunks() {
                    let (hunk, _) = match patch.hunk(hunk_idx) {
                        Ok(h) => h,
                        Err(_) => continue,
                    };
                    let mut current_hunk = Hunk {
                        old_start: hunk.old_start() as i32,
                        new_start: hunk.new_start() as i32,
                        header: String::new(),
                        ..Hunk::default()
                    };

                    let mut old_line = hunk.old_start() as i32;
                    let mut new_line = hunk.new_start() as i32;

                    for line_idx in 0..patch.num_lines_in_hunk(hunk_idx)? {
                        let line = match patch.line_in_hunk(hunk_idx, line_idx) {
                            Ok(l) => l,
                            Err(_) => continue,
                        };
                        let content = std::str::from_utf8(line.content())
                            .unwrap_or_default()
                            .trim_end_matches('\n');
                        let text_range = text_buffer.append(content);

                        let origin = line.origin();
                        let (kind, old_num, new_num, tokens) = if origin == '-' {
                            let removed = vec![DiffTokenSpan {
                                offset: 0,
                                length: content.len() as u32,
                                kind: SyntaxTokenKind::Normal,
                            }];
                            let range = token_buffer.append(&removed);
                            let old = old_line;
                            old_line += 1;
                            (LineKind::Removed, Some(old), None, range)
                        } else if origin == '+' {
                            let added = vec![DiffTokenSpan {
                                offset: 0,
                                length: content.len() as u32,
                                kind: SyntaxTokenKind::Normal,
                            }];
                            let range = token_buffer.append(&added);
                            let new = new_line;
                            new_line += 1;
                            (LineKind::Added, None, Some(new), range)
                        } else if origin == ' ' || origin == '=' {
                            let old = old_line;
                            let new = new_line;
                            old_line += 1;
                            new_line += 1;
                            (LineKind::Context, Some(old), Some(new), Default::default())
                        } else {
                            continue;
                        };

                        current_hunk.lines.push(DiffLine {
                            kind,
                            old_line_number: old_num,
                            new_line_number: new_num,
                            text_range,
                            change_tokens: tokens,
                            ..DiffLine::default()
                        });

                        if kind == LineKind::Added {
                            file.additions += 1;
                        } else if kind == LineKind::Removed {
                            file.deletions += 1;
                        }
                    }

                    if !current_hunk.lines.is_empty() {
                        current_hunk.header = format!(
                            "@@ -{},{} +{},{} @@",
                            current_hunk.old_start,
                            current_hunk
                                .lines
                                .iter()
                                .filter(|l| l.kind != LineKind::Added)
                                .count(),
                            current_hunk.new_start,
                            current_hunk
                                .lines
                                .iter()
                                .filter(|l| l.kind != LineKind::Removed)
                                .count()
                        );
                        file.hunks.push(current_hunk);
                    }
                }
            }

            output.files.push(file);
        }

        output.text_buffer = text_buffer;
        output.token_buffer = token_buffer;
        Ok(Some(output))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Oid, Repository, Signature};
    use tempfile::TempDir;

    use super::GitDiffBackend;
    use crate::core::compare::backends::DiffBackend;
    use crate::core::compare::spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};
    use crate::core::diff::LineKind;
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

    fn checkout_branch(repo: &Repository, branch: &str) {
        let reference = format!("refs/heads/{branch}");
        repo.set_head(&reference).unwrap();
        let head = repo.revparse_single("HEAD").unwrap();
        repo.checkout_tree(&head, None).unwrap();
    }

    fn compare(repo_dir: &TempDir, spec: CompareSpec) -> CompareOutput {
        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();
        GitDiffBackend.compare(&spec, &git).unwrap().unwrap()
    }

    use crate::core::compare::service::CompareOutput;

    #[test]
    fn builtin_backend_uses_single_commit_resolution() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let _first = commit_file(&repo, "src/example.rs", "before\n", "initial");
        let second = commit_file(&repo, "src/example.rs", "after\n", "second");

        let output = compare(
            &repo_dir,
            CompareSpec {
                mode: CompareMode::SingleCommit,
                left_ref: second,
                right_ref: String::new(),
                renderer: RendererKind::Builtin,
                layout: LayoutMode::Unified,
            },
        );

        let file = output.files.first().expect("single file diff");
        assert_eq!(file.path, "src/example.rs");
        let removed = file
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .find(|line| line.kind == LineKind::Removed)
            .expect("removed line");
        assert_eq!(output.text_buffer.view(removed.text_range), "before");
    }

    #[test]
    fn builtin_backend_uses_three_dot_merge_base() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let base = commit_file(&repo, "src/example.rs", "start\n", "initial");
        let base_commit = repo.find_commit(Oid::from_str(&base).unwrap()).unwrap();
        repo.branch("feature", &base_commit, false).unwrap();

        checkout_branch(&repo, "feature");
        let feature = commit_file(&repo, "src/example.rs", "feature\n", "feature");

        checkout_branch(&repo, "master");
        let master = commit_file(&repo, "src/example.rs", "master\n", "master");

        let output = compare(
            &repo_dir,
            CompareSpec {
                mode: CompareMode::ThreeDot,
                left_ref: master,
                right_ref: feature,
                renderer: RendererKind::Builtin,
                layout: LayoutMode::Unified,
            },
        );

        let file = output.files.first().expect("single file diff");
        let removed = file
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .find(|line| line.kind == LineKind::Removed)
            .expect("removed line");
        let added = file
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .find(|line| line.kind == LineKind::Added)
            .expect("added line");

        assert_eq!(output.text_buffer.view(removed.text_range), "start");
        assert_eq!(output.text_buffer.view(added.text_range), "feature");
    }
}
