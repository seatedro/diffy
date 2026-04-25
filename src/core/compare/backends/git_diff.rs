use std::path::Path;

use git2::{Delta, DiffOptions, Oid, Repository};

use crate::core::compare::backends::{DiffBackend, find_similar_bounded};
use crate::core::compare::progress::{ComparePhase, ProgressSink};
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::CompareSpec;
use crate::core::diff::{DeferredHunkSource, DiffLine, FileDiff, Hunk, LineKind};
use crate::core::error::Result;
use crate::core::text::{ChangeIntensity, DiffTokenSpan, SyntaxTokenKind, TextBuffer, TokenBuffer};
use crate::core::vcs::git::{GitService, WORKDIR_REF};

/// Throttle file-progress emits so a 3,000-file diff doesn't post 3,000
/// mpsc sends + winit wakes. Emits on every Nth file plus the final one.
const LOADING_FILE_EMIT_STRIDE: usize = 16;
/// Above this many files, return the changed-file list immediately and defer
/// hunk construction until a file is selected. A Linux release range can touch
/// tens of thousands of files; parsing every patch before first paint makes the
/// app feel stuck even though the sidebar could be useful almost immediately.
const DEFER_HUNKS_FILE_LIMIT: usize = 2_000;

#[derive(Debug, Default, Clone, Copy)]
pub struct GitDiffBackend;

impl GitDiffBackend {
    pub fn compare_stats(
        &self,
        spec: &CompareSpec,
        git: &GitService,
    ) -> Result<Option<(i32, i32)>> {
        let repo = match git.repo() {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        let (left, right) = resolve_compare_refs(spec, git)?;

        let left_commit = repo.find_commit(git2::Oid::from_str(&left)?)?;
        let left_tree = left_commit.tree()?;

        let mut options = DiffOptions::new();
        options.context_lines(0);
        let diff = if right == WORKDIR_REF {
            repo.diff_tree_to_workdir_with_index(Some(&left_tree), Some(&mut options))?
        } else {
            let right_commit = repo.find_commit(git2::Oid::from_str(&right)?)?;
            let right_tree = right_commit.tree()?;
            repo.diff_tree_to_tree(Some(&left_tree), Some(&right_tree), Some(&mut options))?
        };
        let stats = diff.stats()?;
        Ok(Some((
            usize_to_i32_saturating(stats.insertions()),
            usize_to_i32_saturating(stats.deletions()),
        )))
    }

    pub fn compare_deferred_file(
        &self,
        file: &FileDiff,
        git: &GitService,
    ) -> Result<Option<CompareOutput>> {
        let repo = match git.repo() {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        let Some(source) = file.deferred_hunk_source.as_ref() else {
            return Ok(None);
        };
        if file.is_binary {
            let mut file = file.clone();
            file.hunks_deferred = false;
            file.deferred_hunk_source = None;
            return Ok(Some(CompareOutput {
                files: vec![file],
                ..CompareOutput::default()
            }));
        }
        if !can_diff_deferred_source(file, source) {
            return Ok(None);
        }

        let old_content = load_blob_content(repo, source.old_oid.as_deref())?;
        let new_content = load_blob_content(repo, source.new_oid.as_deref())?;
        let old_path = source.old_path.as_deref().map(Path::new);
        let new_path = source.new_path.as_deref().map(Path::new);
        let mut options = DiffOptions::new();
        options.context_lines(3);
        let patch = git2::Patch::from_buffers(
            &old_content,
            old_path,
            &new_content,
            new_path,
            Some(&mut options),
        )?;

        let mut output = CompareOutput::default();
        let mut loaded = file.clone();
        loaded.hunks_deferred = false;
        loaded.deferred_hunk_source = None;
        loaded.hunks.clear();
        loaded.additions = 0;
        loaded.deletions = 0;
        append_patch_hunks(
            &mut loaded,
            &patch,
            &mut output.text_buffer,
            &mut output.token_buffer,
        )?;
        output.files.push(loaded);
        Ok(Some(output))
    }

    pub fn compare_path(
        &self,
        spec: &CompareSpec,
        path: &str,
        git: &GitService,
    ) -> Result<Option<CompareOutput>> {
        let repo = match git.repo() {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        let (left, right) = resolve_compare_refs(spec, git)?;

        let left_commit = repo.find_commit(git2::Oid::from_str(&left)?)?;
        let left_tree = left_commit.tree()?;

        let mut options = DiffOptions::new();
        options.context_lines(3);
        options.pathspec(path);
        let mut diff = if right == WORKDIR_REF {
            repo.diff_tree_to_workdir_with_index(Some(&left_tree), Some(&mut options))?
        } else {
            let right_commit = repo.find_commit(git2::Oid::from_str(&right)?)?;
            let right_tree = right_commit.tree()?;
            repo.diff_tree_to_tree(Some(&left_tree), Some(&right_tree), Some(&mut options))?
        };
        // Per-file path reloads: no progress UI wired for these yet and
        // they're expected to be fast (one file), so pass None.
        Ok(Some(compare_output_from_diff(&mut diff, None)?))
    }

    pub fn deferred_file_line_stats(
        &self,
        file: &FileDiff,
        git: &GitService,
    ) -> Result<Option<(i32, i32)>> {
        if file.is_binary {
            return Ok(Some((0, 0)));
        }
        let repo = match git.repo() {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        let Some(source) = file.deferred_hunk_source.as_ref() else {
            return Ok(None);
        };
        if !can_diff_deferred_source(file, source) {
            return Ok(None);
        }

        let old_content = load_blob_content(repo, source.old_oid.as_deref())?;
        let new_content = load_blob_content(repo, source.new_oid.as_deref())?;
        let old_path = source.old_path.as_deref().map(Path::new);
        let new_path = source.new_path.as_deref().map(Path::new);
        let mut options = DiffOptions::new();
        options.context_lines(0);
        let patch = git2::Patch::from_buffers(
            &old_content,
            old_path,
            &new_content,
            new_path,
            Some(&mut options),
        )?;
        let (_, additions, deletions) = patch.line_stats()?;
        Ok(Some((
            usize_to_i32_saturating(additions),
            usize_to_i32_saturating(deletions),
        )))
    }
}

impl DiffBackend for GitDiffBackend {
    fn compare(
        &self,
        spec: &CompareSpec,
        git: &GitService,
        reporter: Option<&dyn ProgressSink>,
    ) -> Result<Option<CompareOutput>> {
        let repo = match git.repo() {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        let (left, right) = resolve_compare_refs(spec, git)?;

        // The expensive piece before file enumeration: walking trees and
        // running rename detection (`find_similar` inside
        // `compare_output_from_diff`). Announce the phase so the user
        // stops seeing the generic "Opening repository…" label.
        if let Some(r) = reporter {
            r.phase(ComparePhase::EnumeratingChanges);
        }

        let left_commit = repo.find_commit(git2::Oid::from_str(&left)?)?;
        let left_tree = left_commit.tree()?;

        let mut options = DiffOptions::new();
        options.context_lines(3);
        let mut diff = if right == WORKDIR_REF {
            repo.diff_tree_to_workdir_with_index(Some(&left_tree), Some(&mut options))?
        } else {
            let right_commit = repo.find_commit(git2::Oid::from_str(&right)?)?;
            let right_tree = right_commit.tree()?;
            repo.diff_tree_to_tree(Some(&left_tree), Some(&right_tree), Some(&mut options))?
        };
        Ok(Some(compare_output_from_diff(&mut diff, reporter)?))
    }
}

fn resolve_compare_refs(spec: &CompareSpec, git: &GitService) -> Result<(String, String)> {
    match spec.mode {
        crate::core::compare::spec::CompareMode::TwoDot => {
            if spec.right_ref == WORKDIR_REF {
                Ok((git.resolve_ref(&spec.left_ref)?, WORKDIR_REF.to_owned()))
            } else {
                Ok((
                    git.resolve_ref(&spec.left_ref)?,
                    git.resolve_ref(&spec.right_ref)?,
                ))
            }
        }
        crate::core::compare::spec::CompareMode::ThreeDot
        | crate::core::compare::spec::CompareMode::SingleCommit => {
            Ok(git.resolve_comparison(&spec.left_ref, &spec.right_ref, spec.mode)?)
        }
    }
}

pub(crate) fn compare_output_from_diff(
    diff: &mut git2::Diff<'_>,
    reporter: Option<&dyn ProgressSink>,
) -> Result<CompareOutput> {
    // `find_similar` does rename detection — the hottest path after
    // tree-walk on kernel-scale diffs. The user is still under the
    // "Enumerating changes…" label until we finish this.
    let mut output = CompareOutput::default();
    let mut text_buffer = TextBuffer::default();
    let mut token_buffer = TokenBuffer::default();

    let deltas: Vec<_> = diff.deltas().collect();
    if deltas.len() > DEFER_HUNKS_FILE_LIMIT {
        output.files = deltas.iter().map(file_summary_from_delta).collect();
        return Ok(output);
    }

    // `find_similar` does rename detection - the hottest path after tree walk.
    // Run it only after we know the diff is small enough to hydrate eagerly.
    find_similar_bounded(diff)?;
    let deltas: Vec<_> = diff.deltas().collect();
    let files_total = deltas.len() as u32;

    // Kick off the per-file phase with a zero count so the UI can flip to
    // the determinate bar even before the first delta is parsed.
    if let Some(r) = reporter {
        r.phase(ComparePhase::LoadingFiles {
            files_seen: 0,
            files_total,
        });
    }

    for (delta_idx, delta) in deltas.iter().enumerate() {
        // Heartbeat: throttled to 1-in-N deltas + always the final one,
        // to avoid flooding the event channel on big diffs.
        if let Some(r) = reporter {
            let is_last = delta_idx + 1 == deltas.len();
            if delta_idx % LOADING_FILE_EMIT_STRIDE == 0 || is_last {
                r.phase(ComparePhase::LoadingFiles {
                    files_seen: (delta_idx + 1) as u32,
                    files_total,
                });
            }
        }
        let mut file = file_summary_from_delta(delta);
        file.hunks_deferred = false;
        file.stats_deferred = false;
        file.deferred_hunk_source = None;

        if file.is_binary {
            output.files.push(file);
            continue;
        }

        if let Ok(Some(patch)) = git2::Patch::from_diff(diff, delta_idx) {
            append_patch_hunks(&mut file, &patch, &mut text_buffer, &mut token_buffer)?;
        }

        output.files.push(file);
    }

    output.text_buffer = text_buffer;
    output.token_buffer = token_buffer;
    Ok(output)
}

fn load_blob_content(repo: &Repository, oid: Option<&str>) -> Result<Vec<u8>> {
    let Some(oid) = oid else {
        return Ok(Vec::new());
    };
    let oid = Oid::from_str(oid)?;
    Ok(repo.find_blob(oid)?.content().to_vec())
}

fn append_patch_hunks(
    file: &mut FileDiff,
    patch: &git2::Patch<'_>,
    text_buffer: &mut TextBuffer,
    token_buffer: &mut TokenBuffer,
) -> Result<()> {
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
                    intensity: ChangeIntensity::NovelWord,
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
                    intensity: ChangeIntensity::NovelWord,
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
            current_hunk.old_count = current_hunk
                .lines
                .iter()
                .filter(|l| l.kind != LineKind::Added)
                .count() as i32;
            current_hunk.new_count = current_hunk
                .lines
                .iter()
                .filter(|l| l.kind != LineKind::Removed)
                .count() as i32;
            current_hunk.header = format!(
                "@@ -{},{} +{},{} @@",
                current_hunk.old_start,
                current_hunk.old_count,
                current_hunk.new_start,
                current_hunk.new_count,
            );
            file.hunks.push(current_hunk);
        }
    }
    Ok(())
}

fn file_summary_from_delta(delta: &git2::DiffDelta<'_>) -> FileDiff {
    let is_binary = delta.new_file().is_binary() || delta.old_file().is_binary();
    FileDiff {
        path: delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        status: match delta.status() {
            Delta::Added => "A".to_owned(),
            Delta::Deleted => "D".to_owned(),
            Delta::Renamed => "R".to_owned(),
            _ => "M".to_owned(),
        },
        is_binary,
        hunks_deferred: !is_binary,
        stats_deferred: !is_binary,
        deferred_hunk_source: (!is_binary).then(|| DeferredHunkSource {
            old_path: delta
                .old_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned()),
            new_path: delta
                .new_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned()),
            old_oid: oid_string(delta.old_file().id()),
            new_oid: oid_string(delta.new_file().id()),
        }),
        ..FileDiff::default()
    }
}

fn oid_string(oid: Oid) -> Option<String> {
    (oid != Oid::zero()).then(|| oid.to_string())
}

fn can_diff_deferred_source(file: &FileDiff, source: &DeferredHunkSource) -> bool {
    match file.status.as_str() {
        "A" => source.new_oid.is_some(),
        "D" => source.old_oid.is_some(),
        _ => source.old_oid.is_some() && source.new_oid.is_some(),
    }
}

fn usize_to_i32_saturating(value: usize) -> i32 {
    value.min(i32::MAX as usize) as i32
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
    use crate::core::diff::{DeferredHunkSource, FileDiff, LineKind};
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
        GitDiffBackend.compare(&spec, &git, None).unwrap().unwrap()
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

    #[test]
    fn builtin_backend_can_compare_single_path() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let first = commit_file(&repo, "src/a.rs", "before\n", "initial a");
        let _ = commit_file(&repo, "src/b.rs", "before\n", "initial b");
        let second = commit_file(&repo, "src/a.rs", "after\n", "update a");
        let _ = commit_file(&repo, "src/b.rs", "after\n", "update b");

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();

        let output = GitDiffBackend
            .compare_path(
                &CompareSpec {
                    mode: CompareMode::TwoDot,
                    left_ref: first,
                    right_ref: second,
                    renderer: RendererKind::Builtin,
                    layout: LayoutMode::Unified,
                },
                "src/a.rs",
                &git,
            )
            .unwrap()
            .unwrap();

        assert_eq!(output.files.len(), 1);
        assert_eq!(output.files[0].path, "src/a.rs");
    }

    #[test]
    fn builtin_backend_can_compare_deferred_blob_pair() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let first = commit_file(&repo, "src/a.rs", "before\n", "initial");
        let second = commit_file(&repo, "src/a.rs", "after\n", "update");

        let old_oid = repo
            .revparse_single(&format!("{first}:src/a.rs"))
            .unwrap()
            .id()
            .to_string();
        let new_oid = repo
            .revparse_single(&format!("{second}:src/a.rs"))
            .unwrap()
            .id()
            .to_string();
        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();

        let output = GitDiffBackend
            .compare_deferred_file(
                &FileDiff {
                    path: "src/a.rs".to_owned(),
                    status: "M".to_owned(),
                    hunks_deferred: true,
                    deferred_hunk_source: Some(DeferredHunkSource {
                        old_path: Some("src/a.rs".to_owned()),
                        new_path: Some("src/a.rs".to_owned()),
                        old_oid: Some(old_oid),
                        new_oid: Some(new_oid),
                    }),
                    ..FileDiff::default()
                },
                &git,
            )
            .unwrap()
            .unwrap();

        let file = output.files.first().expect("single file diff");
        assert!(!file.hunks_deferred);
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
        assert_eq!(output.text_buffer.view(removed.text_range), "before");
        assert_eq!(output.text_buffer.view(added.text_range), "after");
    }
}
