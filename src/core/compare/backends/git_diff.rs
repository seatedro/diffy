use std::path::Path;

use git2::{Delta, DiffOptions, Oid, Repository};

use crate::core::compare::backends::{DiffBackend, find_similar_bounded};
use crate::core::compare::progress::{ComparePhase, ProgressSink};
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::CompareSpec;
use crate::core::diff::unified_parser::lower_carbon_file;
use crate::core::diff::{DeferredHunkSource, FileDiff};
use crate::core::error::{DiffyError, Result};
use crate::core::text::{TextBuffer, TokenBuffer};
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
                carbon: carbon::DiffDocument {
                    files: vec![carbon_summary_from_legacy(&file, 0)],
                },
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
        let mut patch = git2::Patch::from_buffers(
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
        let (raw_diff, carbon_file) =
            carbon_file_from_patch(&mut patch, output.carbon.files.len(), Some(&loaded))?;
        output.raw_diff.push_str(&raw_diff);
        loaded = lower_carbon_file(
            &carbon_file,
            &mut output.text_buffer,
            Some(&mut output.token_buffer),
        );
        loaded.hunks_deferred = false;
        loaded.deferred_hunk_source = None;
        output.carbon.files.push(carbon_file);
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
        output.carbon.files = output
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| carbon_summary_from_legacy(file, index))
            .collect();
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
            output
                .carbon
                .files
                .push(carbon_summary_from_legacy(&file, output.carbon.files.len()));
            output.files.push(file);
            continue;
        }

        if let Ok(Some(mut patch)) = git2::Patch::from_diff(diff, delta_idx) {
            let (raw_diff, carbon_file) =
                carbon_file_from_patch(&mut patch, output.carbon.files.len(), Some(&file))?;
            output.raw_diff.push_str(&raw_diff);
            file = lower_carbon_file(&carbon_file, &mut text_buffer, Some(&mut token_buffer));
            file.hunks_deferred = false;
            file.stats_deferred = false;
            file.deferred_hunk_source = None;
            output.carbon.files.push(carbon_file);
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

fn carbon_file_from_patch(
    patch: &mut git2::Patch<'_>,
    file_id: usize,
    summary: Option<&FileDiff>,
) -> Result<(String, carbon::FileDiff)> {
    let raw = patch.to_buf()?;
    let raw_diff = String::from_utf8_lossy(raw.as_ref()).into_owned();
    let mut document = carbon::parse_unified_patch(&raw_diff)
        .map_err(|error| DiffyError::Parse(error.to_string()))?;
    let Some(mut file) = document.files.pop() else {
        return Err(DiffyError::Parse("patch contained no file diff".to_owned()));
    };
    file.id = carbon::FileId(usize_to_u32_saturating(file_id));
    if let Some(summary) = summary {
        if file.old_path.is_none() && !summary.path.is_empty() {
            file.old_path = Some(summary.path.clone());
        }
        if file.new_path.is_none() && !summary.path.is_empty() {
            file.new_path = Some(summary.path.clone());
        }
        if summary.is_binary {
            file.is_binary = true;
            file.status = carbon::FileStatus::Binary;
        }
    }
    Ok((raw_diff, file))
}

fn carbon_summary_from_legacy(file: &FileDiff, file_id: usize) -> carbon::FileDiff {
    let source = file.deferred_hunk_source.as_ref();
    let path = (!file.path.is_empty()).then(|| file.path.clone());
    let (old_path, new_path) = match file.status.as_str() {
        "A" => (
            None,
            source.and_then(|source| source.new_path.clone()).or(path),
        ),
        "D" => (
            source.and_then(|source| source.old_path.clone()).or(path),
            None,
        ),
        _ => (
            source
                .and_then(|source| source.old_path.clone())
                .or_else(|| path.clone()),
            source.and_then(|source| source.new_path.clone()).or(path),
        ),
    };
    carbon::FileDiff {
        id: carbon::FileId(usize_to_u32_saturating(file_id)),
        old_path,
        new_path,
        status: carbon_status_from_legacy(file),
        is_binary: file.is_binary,
        is_partial: true,
        ..carbon::FileDiff::default()
    }
}

fn carbon_status_from_legacy(file: &FileDiff) -> carbon::FileStatus {
    if file.is_binary {
        carbon::FileStatus::Binary
    } else {
        match file.status.as_str() {
            "A" => carbon::FileStatus::Added,
            "D" => carbon::FileStatus::Deleted,
            "R" => carbon::FileStatus::Renamed,
            _ => carbon::FileStatus::Modified,
        }
    }
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

fn usize_to_u32_saturating(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
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
        assert_eq!(output.carbon.files.len(), 1);
        assert_eq!(output.carbon.files[0].path(), "src/example.rs");
        assert!(output.raw_diff.contains("diff --git"));
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
        assert_eq!(output.carbon.files.len(), 1);
        assert_eq!(output.carbon.files[0].path(), "src/a.rs");
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
