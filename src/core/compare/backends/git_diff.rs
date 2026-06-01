use gix::bstr::{BStr, ByteSlice};
use gix::diff::blob::{
    Algorithm, Diff, InternedInput, UnifiedDiff, diff_with_slider_heuristics,
    platform::resource::ByteLinesWithoutTerminator,
    unified_diff::{ConsumeBinaryHunk, ContextSize},
};

use crate::core::compare::backends::{DiffBackend, RENAME_DETECTION_LIMIT};
use crate::core::compare::progress::{ComparePhase, ProgressSink};
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::CompareSpec;
use crate::core::compare::stats::{
    COMPARE_SUMMARY_FILE_LIMIT, CompareFilePaths, CompareFileStatsTarget, CompareFileSummary,
};
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::git::{GitService, WORKDIR_REF};

/// Throttle file-progress emits so a 3,000-file diff doesn't post 3,000
/// mpsc sends + winit wakes. Emits on every Nth file plus the final one.
const LOADING_FILE_EMIT_STRIDE: usize = 16;
/// Line-stat work parallelism. Hunk deferral uses the VCS-neutral
/// `COMPARE_SUMMARY_FILE_LIMIT`.
const LINE_STATS_MAX_WORKERS: usize = 8;
const DEFERRED_STATS_MIN_FILES_PER_WORKER: usize = 16;

#[derive(Debug, Default, Clone, Copy)]
pub struct GitDiffBackend;

impl GitDiffBackend {
    pub fn compare_stats(
        &self,
        spec: &CompareSpec,
        git: &GitService,
    ) -> Result<Option<(i32, i32)>> {
        let (left, right) = resolve_compare_refs(spec, git)?;
        Ok(Some(git.diff_shortstat_find_renames(&left, &right)?))
    }

    pub fn compare_deferred_file(
        &self,
        file: &carbon::FileDiff,
        git: &GitService,
    ) -> Result<Option<CompareOutput>> {
        if file.is_binary {
            let mut file = file.clone();
            file.is_partial = false;
            return Ok(Some(CompareOutput {
                carbon: carbon::DiffDocument { files: vec![file] },
                ..CompareOutput::default()
            }));
        }
        if !can_diff_deferred_file(file) {
            return Ok(None);
        }

        let gix_repo = open_gix_repo(git)?;
        let old_content = load_gix_blob_content(&gix_repo, file.old_oid.as_ref())?;
        let new_content = load_gix_blob_content(&gix_repo, file.new_oid.as_ref())?;
        if is_binary_bytes(old_content.as_bytes()) || is_binary_bytes(new_content.as_bytes()) {
            let mut file = file.clone();
            file.is_binary = true;
            file.status = carbon::FileStatus::Binary;
            file.is_partial = false;
            return Ok(Some(CompareOutput {
                carbon: carbon::DiffDocument { files: vec![file] },
                ..CompareOutput::default()
            }));
        }
        let raw_diff =
            raw_patch_from_blob_pair(file, old_content.as_bytes(), new_content.as_bytes(), 3)?;
        let mut output = CompareOutput::default();
        let carbon_file =
            carbon_file_from_raw_diff(&raw_diff, output.carbon.files.len(), Some(file))?;
        output.raw_diff_len = output.raw_diff_len.saturating_add(raw_diff.len());
        output.carbon.files.push(carbon_file);
        Ok(Some(output))
    }

    pub fn compare_path(
        &self,
        spec: &CompareSpec,
        path: &str,
        git: &GitService,
    ) -> Result<Option<CompareOutput>> {
        let (left, right) = resolve_compare_refs(spec, git)?;
        let raw = git.diff_two_refs_path(&left, &right, path)?;
        Ok(Some(compare_output_from_raw_patch(&raw)?))
    }

    pub fn compare_path_no_renames(
        &self,
        spec: &CompareSpec,
        path: &str,
        git: &GitService,
    ) -> Result<Option<CompareOutput>> {
        let (left, right) = resolve_compare_refs(spec, git)?;
        let raw = git.diff_two_refs_path_no_renames(&left, &right, path)?;
        Ok(Some(compare_output_from_raw_patch(&raw)?))
    }

    pub fn deferred_file_line_stats_batch_for_request(
        &self,
        spec: &CompareSpec,
        git: &GitService,
        files: &[CompareFileStatsTarget],
    ) -> Vec<Option<(i32, i32)>> {
        if files.is_empty() {
            return Vec::new();
        }
        let Ok((left, right)) = resolve_compare_refs(spec, git) else {
            return vec![None; files.len()];
        };
        if right == WORKDIR_REF {
            return vec![None; files.len()];
        }
        let worker_count = deferred_stats_worker_count(files.len());
        if worker_count == 1 {
            return deferred_file_stats_target_chunk(git.repo_path(), &left, &right, 0, files)
                .into_iter()
                .map(|(_, stat)| stat)
                .collect();
        }

        let repo_path = git.repo_path();
        let left_ref = left.as_str();
        let right_ref = right.as_str();
        let chunk_size = files.len().div_ceil(worker_count);
        let mut results = vec![None; files.len()];
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(worker_count);
            for (chunk_index, chunk) in files.chunks(chunk_size).enumerate() {
                let start = chunk_index * chunk_size;
                handles.push(scope.spawn(move || {
                    deferred_file_stats_target_chunk(repo_path, left_ref, right_ref, start, chunk)
                }));
            }

            for handle in handles {
                for (index, stat) in handle.join().unwrap_or_default() {
                    if let Some(slot) = results.get_mut(index) {
                        *slot = stat;
                    }
                }
            }
        });
        results
    }
}

impl DiffBackend for GitDiffBackend {
    fn compare(
        &self,
        spec: &CompareSpec,
        git: &GitService,
        reporter: Option<&dyn ProgressSink>,
    ) -> Result<Option<CompareOutput>> {
        let (left, right) = resolve_compare_refs(spec, git)?;
        if right != WORKDIR_REF {
            if let Some(r) = reporter {
                r.phase(ComparePhase::EnumeratingChanges);
            }
            let mut repo = open_gix_repo(git)?;
            let mut changes = collect_gix_changes(&mut repo, &left, &right, None, false)?;
            if changes.len() <= COMPARE_SUMMARY_FILE_LIMIT
                && gix_changes_may_include_rewrites(&changes)
            {
                changes = collect_gix_changes(&mut repo, &left, &right, None, true)?;
            }
            return Ok(Some(compare_output_from_gix_changes(
                &repo, changes, reporter,
            )?));
        }

        if let Some(r) = reporter {
            r.phase(ComparePhase::EnumeratingChanges);
        }

        let raw = git.diff_two_refs(&left, &right)?;
        Ok(Some(compare_output_from_raw_patch(&raw)?))
    }
}

fn deferred_file_stats_target_with_trees(
    file: &CompareFileStatsTarget,
    gix_repo: &gix::Repository,
    left_tree: &gix::Tree<'_>,
    right_tree: &gix::Tree<'_>,
) -> Result<Option<(i32, i32)>> {
    if file.is_binary {
        return Ok(Some((0, 0)));
    }
    let old_content = match file.status {
        carbon::FileStatus::Added => GixBlobContent::Empty,
        _ => {
            let old_path = file.paths.old_path();
            let Some(content) =
                load_gix_tree_blob_content(gix_repo, left_tree, old_path.as_deref())?
            else {
                return Ok(None);
            };
            content
        }
    };
    let new_content = match file.status {
        carbon::FileStatus::Deleted => GixBlobContent::Empty,
        _ => {
            let new_path = file.paths.new_path();
            let Some(content) =
                load_gix_tree_blob_content(gix_repo, right_tree, new_path.as_deref())?
            else {
                return Ok(None);
            };
            content
        }
    };
    if is_binary_bytes(old_content.as_bytes()) || is_binary_bytes(new_content.as_bytes()) {
        return Ok(Some((0, 0)));
    }
    let (additions, deletions) = gix_line_stats(old_content.as_bytes(), new_content.as_bytes());
    Ok(Some((
        u32_to_i32_saturating(additions),
        u32_to_i32_saturating(deletions),
    )))
}

fn deferred_file_stats_target_chunk(
    repo_path: &str,
    left: &str,
    right: &str,
    start: usize,
    files: &[CompareFileStatsTarget],
) -> Vec<(usize, Option<(i32, i32)>)> {
    let empty_stats = || {
        (0..files.len())
            .map(|offset| (start + offset, None))
            .collect()
    };
    let Ok(gix_repo) = open_gix_repo_path(repo_path) else {
        return empty_stats();
    };
    let Ok(left_tree) = gix_tree_for_oid(&gix_repo, left) else {
        return empty_stats();
    };
    let Ok(right_tree) = gix_tree_for_oid(&gix_repo, right) else {
        return empty_stats();
    };
    files
        .iter()
        .enumerate()
        .map(|(offset, file)| {
            (
                start + offset,
                deferred_file_stats_target_with_trees(file, &gix_repo, &left_tree, &right_tree)
                    .ok()
                    .flatten(),
            )
        })
        .collect()
}

fn deferred_stats_worker_count(file_count: usize) -> usize {
    let available = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let useful = file_count.div_ceil(DEFERRED_STATS_MIN_FILES_PER_WORKER);
    available.min(LINE_STATS_MAX_WORKERS).min(useful).max(1)
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

pub(crate) fn compare_output_from_raw_patch(raw_diff: &str) -> Result<CompareOutput> {
    let mut output = CompareOutput::default();
    let mut document = carbon::parse_unified_patch(raw_diff)
        .map_err(|error| DiffyError::Parse(error.to_string()))?;
    for (index, file) in document.files.iter_mut().enumerate() {
        file.id = carbon::FileId(usize_to_u32_saturating(index));
        file.is_partial = false;
    }
    output.raw_diff_len = raw_diff.len();
    output.carbon = document;
    Ok(output)
}

pub(crate) fn compare_text_builtin(
    left_text: &str,
    right_text: &str,
    display_path: &str,
) -> Result<CompareOutput> {
    if left_text == right_text {
        return Ok(CompareOutput::default());
    }

    let status = if left_text.is_empty() {
        carbon::FileStatus::Added
    } else if right_text.is_empty() {
        carbon::FileStatus::Deleted
    } else {
        carbon::FileStatus::Modified
    };
    let old_path = (status != carbon::FileStatus::Added).then_some(display_path);
    let new_path = (status != carbon::FileStatus::Deleted).then_some(display_path);
    let mut raw = raw_patch_header(
        old_path,
        new_path,
        None,
        None,
        Some("100644"),
        Some("100644"),
        status,
    );
    raw.push_str(&render_gix_unified_hunks(
        left_text.as_bytes(),
        right_text.as_bytes(),
        3,
    )?);
    let mut output = compare_output_from_raw_patch(&raw)?;
    hydrate_text_compare_sources(&mut output, left_text, right_text);
    Ok(output)
}

fn hydrate_text_compare_sources(output: &mut CompareOutput, left_text: &str, right_text: &str) {
    let Some(file) = output.carbon.files.first_mut() else {
        return;
    };
    if !left_text.is_empty() {
        file.old_text = Some(carbon::TextStore::from_text(left_text.to_owned()));
    }
    if !right_text.is_empty() {
        file.new_text = Some(carbon::TextStore::from_text(right_text.to_owned()));
    }
    if !left_text.is_empty() && !left_text.ends_with('\n') {
        if let Some(block) = file.blocks.iter_mut().rev().find(|block| block.old.len > 0) {
            block.old_no_newline_at_end = true;
        }
    }
    if !right_text.is_empty() && !right_text.ends_with('\n') {
        if let Some(block) = file.blocks.iter_mut().rev().find(|block| block.new.len > 0) {
            block.new_no_newline_at_end = true;
        }
    }
}

type GixChange = gix::object::tree::diff::ChangeDetached;

fn open_gix_repo(git: &GitService) -> Result<gix::Repository> {
    open_gix_repo_path(git.repo_path())
}

fn open_gix_repo_path(repo_path: &str) -> Result<gix::Repository> {
    let mut repo = gix::open(repo_path).map_err(gix_error)?;
    repo.object_cache_size_if_unset(64 * 1024 * 1024);
    Ok(repo)
}

fn gix_error(error: impl std::fmt::Display) -> DiffyError {
    DiffyError::General(format!("Gitoxide error: {error}"))
}

fn collect_gix_changes(
    repo: &mut gix::Repository,
    left: &str,
    right: &str,
    path_filter: Option<&str>,
    track_rewrites: bool,
) -> Result<Vec<GixChange>> {
    let left_tree = gix_tree_for_oid(repo, left)?;
    let right_tree = gix_tree_for_oid(repo, right)?;
    let mut options = gix::diff::Options::default();
    options.track_path();
    if track_rewrites {
        options.track_rewrites(Some(gix::diff::Rewrites {
            limit: RENAME_DETECTION_LIMIT,
            ..Default::default()
        }));
    } else {
        options.track_rewrites(None);
    }
    let mut changes = repo
        .diff_tree_to_tree(Some(&left_tree), Some(&right_tree), Some(options))
        .map_err(gix_error)?;
    changes.retain(gix_change_is_file);
    if let Some(path) = path_filter {
        changes.retain(|change| gix_change_matches_path(change, path));
    }
    Ok(changes)
}

fn gix_tree_for_oid<'repo>(repo: &'repo gix::Repository, oid: &str) -> Result<gix::Tree<'repo>> {
    let oid = gix_object_id(oid)?;
    let object = repo.find_object(oid).map_err(gix_error)?;
    object.peel_to_tree().map_err(gix_error)
}

fn gix_object_id(oid: &str) -> Result<gix::ObjectId> {
    gix::ObjectId::from_hex(oid.as_bytes()).map_err(gix_error)
}

fn gix_change_matches_path(change: &GixChange, path: &str) -> bool {
    let path = path.as_bytes();
    change.location().as_bytes() == path || change.source_location().as_bytes() == path
}

fn gix_change_is_file(change: &GixChange) -> bool {
    match change {
        GixChange::Addition { entry_mode, .. } | GixChange::Deletion { entry_mode, .. } => {
            entry_mode.is_no_tree()
        }
        GixChange::Modification {
            previous_entry_mode,
            entry_mode,
            ..
        } => previous_entry_mode.is_no_tree() && entry_mode.is_no_tree(),
        GixChange::Rewrite {
            source_entry_mode,
            entry_mode,
            ..
        } => source_entry_mode.is_no_tree() && entry_mode.is_no_tree(),
    }
}

fn gix_changes_may_include_rewrites(changes: &[GixChange]) -> bool {
    let mut has_addition = false;
    let mut has_deletion = false;
    for change in changes {
        match change {
            GixChange::Addition { .. } => has_addition = true,
            GixChange::Deletion { .. } => has_deletion = true,
            GixChange::Modification { .. } | GixChange::Rewrite { .. } => {}
        }
        if has_addition && has_deletion {
            return true;
        }
    }
    false
}

fn compare_output_from_gix_changes(
    repo: &gix::Repository,
    changes: Vec<GixChange>,
    reporter: Option<&dyn ProgressSink>,
) -> Result<CompareOutput> {
    let mut output = CompareOutput::default();
    if changes.len() > COMPARE_SUMMARY_FILE_LIMIT {
        output.file_summaries = changes
            .iter()
            .enumerate()
            .map(|(index, change)| compare_summary_from_gix_change(change, index, true, false))
            .collect();
        output.compact_file_summaries();
        return Ok(output);
    }

    let files_total = changes.len() as u32;
    if let Some(r) = reporter {
        r.phase(ComparePhase::LoadingFiles {
            files_seen: 0,
            files_total,
        });
    }

    for (change_idx, change) in changes.iter().enumerate() {
        if let Some(r) = reporter {
            let is_last = change_idx + 1 == changes.len();
            if change_idx % LOADING_FILE_EMIT_STRIDE == 0 || is_last {
                r.phase(ComparePhase::LoadingFiles {
                    files_seen: (change_idx + 1) as u32,
                    files_total,
                });
            }
        }

        let Some((old_content, new_content)) = gix_change_contents(repo, change)? else {
            continue;
        };
        if is_binary_bytes(old_content.as_bytes()) || is_binary_bytes(new_content.as_bytes()) {
            output.carbon.files.push(carbon_summary_from_gix_change(
                change,
                output.carbon.files.len(),
                false,
                true,
            ));
            continue;
        }

        let raw_diff =
            raw_patch_from_gix_change(change, old_content.as_bytes(), new_content.as_bytes(), 3)?;
        let carbon_file = carbon_file_from_raw_diff(&raw_diff, output.carbon.files.len(), None)?;
        output.raw_diff_len = output.raw_diff_len.saturating_add(raw_diff.len());
        output.carbon.files.push(carbon_file);
    }

    Ok(output)
}

enum GixBlobContent<'repo> {
    Empty,
    Blob(gix::Blob<'repo>),
}

impl GixBlobContent<'_> {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Empty => &[],
            Self::Blob(blob) => &blob.data,
        }
    }
}

fn gix_change_contents<'repo>(
    repo: &'repo gix::Repository,
    change: &GixChange,
) -> Result<Option<(GixBlobContent<'repo>, GixBlobContent<'repo>)>> {
    match change {
        GixChange::Addition { entry_mode, id, .. } => {
            if !entry_mode.is_no_tree() {
                return Ok(None);
            }
            Ok(Some((GixBlobContent::Empty, load_gix_blob_id(repo, *id)?)))
        }
        GixChange::Deletion { entry_mode, id, .. } => {
            if !entry_mode.is_no_tree() {
                return Ok(None);
            }
            Ok(Some((load_gix_blob_id(repo, *id)?, GixBlobContent::Empty)))
        }
        GixChange::Modification {
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
            ..
        } => {
            if !previous_entry_mode.is_no_tree() || !entry_mode.is_no_tree() {
                return Ok(None);
            }
            Ok(Some((
                load_gix_blob_id(repo, *previous_id)?,
                load_gix_blob_id(repo, *id)?,
            )))
        }
        GixChange::Rewrite {
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            ..
        } => {
            if !source_entry_mode.is_no_tree() || !entry_mode.is_no_tree() {
                return Ok(None);
            }
            Ok(Some((
                load_gix_blob_id(repo, *source_id)?,
                load_gix_blob_id(repo, *id)?,
            )))
        }
    }
}

fn load_gix_blob_content<'repo>(
    repo: &'repo gix::Repository,
    oid: Option<&carbon::ObjectId>,
) -> Result<GixBlobContent<'repo>> {
    let Some(oid) = oid else {
        return Ok(GixBlobContent::Empty);
    };
    load_gix_blob_id(repo, gix_object_id(&oid.0)?)
}

fn load_gix_tree_blob_content<'repo>(
    repo: &'repo gix::Repository,
    tree: &gix::Tree<'_>,
    path: Option<&str>,
) -> Result<Option<GixBlobContent<'repo>>> {
    let Some(path) = path else {
        return Ok(Some(GixBlobContent::Empty));
    };
    let Some(entry) = tree.lookup_entry_by_path(path).map_err(gix_error)? else {
        return Ok(None);
    };
    if !entry.mode().is_blob_or_symlink() {
        return Ok(None);
    }
    Ok(Some(load_gix_blob_id(repo, entry.object_id())?))
}

fn load_gix_blob_id(repo: &gix::Repository, oid: gix::ObjectId) -> Result<GixBlobContent<'_>> {
    if oid.is_null() {
        return Ok(GixBlobContent::Empty);
    }
    Ok(GixBlobContent::Blob(
        repo.find_blob(oid).map_err(gix_error)?,
    ))
}

fn is_binary_bytes(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

fn gix_line_stats(old_content: &[u8], new_content: &[u8]) -> (u32, u32) {
    let input = InternedInput::new(
        ByteLinesWithoutTerminator::new(old_content),
        ByteLinesWithoutTerminator::new(new_content),
    );
    let diff = Diff::compute(Algorithm::Histogram, &input);
    (diff.count_additions(), diff.count_removals())
}

fn raw_patch_from_blob_pair(
    file: &carbon::FileDiff,
    old_content: &[u8],
    new_content: &[u8],
    context_lines: u32,
) -> Result<String> {
    let old_path = file.old_path.as_deref();
    let new_path = file.new_path.as_deref();
    let old_oid = file.old_oid.as_ref().map(|oid| oid.0.as_str());
    let new_oid = file.new_oid.as_ref().map(|oid| oid.0.as_str());
    let old_mode = Some("100644");
    let new_mode = Some("100644");
    let mut raw = raw_patch_header(
        old_path,
        new_path,
        old_oid,
        new_oid,
        old_mode,
        new_mode,
        file.status,
    );
    raw.push_str(&render_gix_unified_hunks(
        old_content,
        new_content,
        context_lines,
    )?);
    Ok(raw)
}

fn raw_patch_from_gix_change(
    change: &GixChange,
    old_content: &[u8],
    new_content: &[u8],
    context_lines: u32,
) -> Result<String> {
    let meta = gix_change_meta(change);
    let mut raw = raw_patch_header(
        meta.old_path,
        meta.new_path,
        meta.old_oid.as_deref(),
        meta.new_oid.as_deref(),
        meta.old_mode.as_deref(),
        meta.new_mode.as_deref(),
        meta.status,
    );
    raw.push_str(&render_gix_unified_hunks(
        old_content,
        new_content,
        context_lines,
    )?);
    Ok(raw)
}

fn render_gix_unified_hunks(
    old_content: &[u8],
    new_content: &[u8],
    context_lines: u32,
) -> Result<String> {
    let input = InternedInput::new(
        ByteLinesWithoutTerminator::new(old_content),
        ByteLinesWithoutTerminator::new(new_content),
    );
    let diff = diff_with_slider_heuristics(Algorithm::Histogram, &input);
    UnifiedDiff::new(
        &diff,
        &input,
        ConsumeBinaryHunk::new(String::new(), "\n"),
        ContextSize::symmetrical(context_lines),
    )
    .consume()
    .map_err(|error| DiffyError::General(format!("Gitoxide diff render failed: {error}")))
}

fn raw_patch_header(
    old_path: Option<&str>,
    new_path: Option<&str>,
    old_oid: Option<&str>,
    new_oid: Option<&str>,
    old_mode: Option<&str>,
    new_mode: Option<&str>,
    status: carbon::FileStatus,
) -> String {
    let display_old = old_path.or(new_path).unwrap_or("unknown");
    let display_new = new_path.or(old_path).unwrap_or("unknown");
    let old_header = old_path
        .map(|path| format!("a/{path}"))
        .unwrap_or_else(|| "/dev/null".to_owned());
    let new_header = new_path
        .map(|path| format!("b/{path}"))
        .unwrap_or_else(|| "/dev/null".to_owned());
    let old_oid = old_oid.unwrap_or("0000000000000000000000000000000000000000");
    let new_oid = new_oid.unwrap_or("0000000000000000000000000000000000000000");
    let mode = new_mode.or(old_mode).unwrap_or("100644");

    let mut raw = format!("diff --git a/{display_old} b/{display_new}\n");
    match status {
        carbon::FileStatus::Added => {
            raw.push_str(&format!("new file mode {}\n", new_mode.unwrap_or(mode)));
        }
        carbon::FileStatus::Deleted => {
            raw.push_str(&format!("deleted file mode {}\n", old_mode.unwrap_or(mode)));
        }
        carbon::FileStatus::Renamed | carbon::FileStatus::RenamedModified => {
            if let Some(path) = old_path {
                raw.push_str(&format!("rename from {path}\n"));
            }
            if let Some(path) = new_path {
                raw.push_str(&format!("rename to {path}\n"));
            }
        }
        _ => {}
    }
    if let (Some(old_mode), Some(new_mode)) = (old_mode, new_mode)
        && old_mode != new_mode
    {
        raw.push_str(&format!("old mode {old_mode}\n"));
        raw.push_str(&format!("new mode {new_mode}\n"));
    }
    raw.push_str(&format!("index {old_oid}..{new_oid} {mode}\n"));
    raw.push_str(&format!("--- {old_header}\n"));
    raw.push_str(&format!("+++ {new_header}\n"));
    raw
}

struct GixChangeMeta<'a> {
    old_path: Option<&'a str>,
    new_path: Option<&'a str>,
    old_oid: Option<String>,
    new_oid: Option<String>,
    old_mode: Option<String>,
    new_mode: Option<String>,
    status: carbon::FileStatus,
}

fn gix_change_meta(change: &GixChange) -> GixChangeMeta<'_> {
    match change {
        GixChange::Addition {
            location,
            entry_mode,
            id,
            ..
        } => GixChangeMeta {
            old_path: None,
            new_path: Some(bstr_to_str(location.as_bstr())),
            old_oid: None,
            new_oid: Some(id.to_string()),
            old_mode: None,
            new_mode: Some(gix_mode(entry_mode)),
            status: carbon::FileStatus::Added,
        },
        GixChange::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => GixChangeMeta {
            old_path: Some(bstr_to_str(location.as_bstr())),
            new_path: None,
            old_oid: Some(id.to_string()),
            new_oid: None,
            old_mode: Some(gix_mode(entry_mode)),
            new_mode: None,
            status: carbon::FileStatus::Deleted,
        },
        GixChange::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => GixChangeMeta {
            old_path: Some(bstr_to_str(location.as_bstr())),
            new_path: Some(bstr_to_str(location.as_bstr())),
            old_oid: Some(previous_id.to_string()),
            new_oid: Some(id.to_string()),
            old_mode: Some(gix_mode(previous_entry_mode)),
            new_mode: Some(gix_mode(entry_mode)),
            status: carbon::FileStatus::Modified,
        },
        GixChange::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            location,
            copy: _,
            ..
        } => GixChangeMeta {
            old_path: Some(bstr_to_str(source_location.as_bstr())),
            new_path: Some(bstr_to_str(location.as_bstr())),
            old_oid: Some(source_id.to_string()),
            new_oid: Some(id.to_string()),
            old_mode: Some(gix_mode(source_entry_mode)),
            new_mode: Some(gix_mode(entry_mode)),
            status: carbon::FileStatus::Renamed,
        },
    }
}

fn bstr_to_str(path: &BStr) -> &str {
    path.to_str().unwrap_or("")
}

fn gix_mode(mode: &gix::objs::tree::EntryMode) -> String {
    format!("{:06o}", mode.value())
}

fn carbon_summary_from_gix_change(
    change: &GixChange,
    file_id: usize,
    stats_deferred: bool,
    is_binary: bool,
) -> carbon::FileDiff {
    let meta = gix_change_meta(change);
    carbon::FileDiff {
        id: carbon::FileId(usize_to_u32_saturating(file_id)),
        old_path: meta.old_path.map(ToOwned::to_owned),
        new_path: meta.new_path.map(ToOwned::to_owned),
        old_oid: meta.old_oid.map(carbon::ObjectId),
        new_oid: meta.new_oid.map(carbon::ObjectId),
        status: if is_binary {
            carbon::FileStatus::Binary
        } else {
            meta.status
        },
        is_binary,
        is_partial: stats_deferred,
        stats_deferred,
        ..carbon::FileDiff::default()
    }
}

fn compare_summary_from_gix_change(
    change: &GixChange,
    _file_id: usize,
    stats_deferred: bool,
    is_binary: bool,
) -> CompareFileSummary {
    let meta = gix_change_meta(change);
    CompareFileSummary {
        paths: CompareFilePaths::from_paths(meta.old_path.as_deref(), meta.new_path.as_deref()),
        old_oid: meta.old_oid.map(std::sync::Arc::from),
        new_oid: meta.new_oid.map(std::sync::Arc::from),
        status: if is_binary {
            carbon::FileStatus::Binary
        } else {
            meta.status
        },
        is_binary,
        is_partial: stats_deferred,
        additions: 0,
        deletions: 0,
        stats_deferred,
    }
}

fn carbon_file_from_raw_diff(
    raw_diff: &str,
    file_id: usize,
    summary: Option<&carbon::FileDiff>,
) -> Result<carbon::FileDiff> {
    let mut document = carbon::parse_unified_patch(raw_diff)
        .map_err(|error| DiffyError::Parse(error.to_string()))?;
    let Some(mut file) = document.files.pop() else {
        return Err(DiffyError::Parse("patch contained no file diff".to_owned()));
    };
    file.id = carbon::FileId(usize_to_u32_saturating(file_id));
    file.is_partial = false;
    if let Some(summary) = summary {
        if file.old_path.is_none() {
            file.old_path.clone_from(&summary.old_path);
        }
        if file.new_path.is_none() {
            file.new_path.clone_from(&summary.new_path);
        }
        if summary.is_binary {
            file.is_binary = true;
            file.status = carbon::FileStatus::Binary;
        }
    }
    Ok(file)
}

fn can_diff_deferred_file(file: &carbon::FileDiff) -> bool {
    match file.status {
        carbon::FileStatus::Added => file.new_oid.is_some(),
        carbon::FileStatus::Deleted => file.old_oid.is_some(),
        _ => file.old_oid.is_some() && file.new_oid.is_some(),
    }
}

fn u32_to_i32_saturating(value: u32) -> i32 {
    value.min(i32::MAX as u32) as i32
}

fn usize_to_u32_saturating(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use git2::{Oid, Repository, Signature};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    use super::GitDiffBackend;
    use crate::core::compare::backends::DiffBackend;
    use crate::core::compare::spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};
    use crate::core::vcs::git::{GitService, WORKDIR_REF};

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
    use crate::core::compare::stats::{CompareFilePaths, CompareFileStatsTarget, ComparePath};

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

        assert_eq!(output.carbon.files.len(), 1);
        assert_eq!(output.carbon.files[0].path(), "src/example.rs");
        assert!(output.raw_diff_len > 0);
        let removed = output.carbon.files[0]
            .blocks
            .iter()
            .find(|block| block.kind == carbon::BlockKind::Change)
            .and_then(|block| {
                output.carbon.files[0]
                    .old_text
                    .as_ref()
                    .and_then(|text| text.line_str(carbon::LineId(block.old.start)))
            })
            .expect("removed line");
        assert_eq!(removed, "before");
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

        assert_eq!(output.carbon.files.len(), 1);
        let change = output.carbon.files[0]
            .blocks
            .iter()
            .find(|block| block.kind == carbon::BlockKind::Change)
            .expect("change block");
        let removed = output.carbon.files[0]
            .old_text
            .as_ref()
            .and_then(|text| text.line_str(carbon::LineId(change.old.start)))
            .expect("removed line");
        let added = output.carbon.files[0]
            .new_text
            .as_ref()
            .and_then(|text| text.line_str(carbon::LineId(change.new.start)))
            .expect("added line");

        assert_eq!(removed, "start");
        assert_eq!(added, "feature");
    }

    #[test]
    fn builtin_backend_uses_head_merge_base_for_three_dot_workdir() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let base = commit_file(&repo, "src/example.rs", "start\n", "initial");
        let base_commit = repo.find_commit(Oid::from_str(&base).unwrap()).unwrap();
        repo.branch("feature", &base_commit, false).unwrap();

        checkout_branch(&repo, "feature");
        let _feature = commit_file(&repo, "src/example.rs", "feature\n", "feature");

        checkout_branch(&repo, "master");
        let master = commit_file(&repo, "src/example.rs", "master\n", "master");

        checkout_branch(&repo, "feature");
        fs::write(repo.workdir().unwrap().join("src/example.rs"), "dirty\n").unwrap();

        let output = compare(
            &repo_dir,
            CompareSpec {
                mode: CompareMode::ThreeDot,
                left_ref: master,
                right_ref: WORKDIR_REF.to_owned(),
                renderer: RendererKind::Builtin,
                layout: LayoutMode::Unified,
            },
        );

        assert_eq!(output.carbon.files.len(), 1);
        let change = output.carbon.files[0]
            .blocks
            .iter()
            .find(|block| block.kind == carbon::BlockKind::Change)
            .expect("change block");
        let removed = output.carbon.files[0]
            .old_text
            .as_ref()
            .and_then(|text| text.line_str(carbon::LineId(change.old.start)))
            .expect("removed line");
        let added = output.carbon.files[0]
            .new_text
            .as_ref()
            .and_then(|text| text.line_str(carbon::LineId(change.new.start)))
            .expect("added line");

        assert_eq!(removed, "start");
        assert_eq!(added, "dirty");
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

        assert_eq!(output.carbon.files.len(), 1);
        assert_eq!(output.carbon.files[0].path(), "src/a.rs");
    }

    #[test]
    fn builtin_backend_can_compare_summary_path_without_renames() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let first = commit_file(&repo, "src/a.rs", "before\n", "initial a");
        let _ = commit_file(&repo, "src/b.rs", "before\n", "initial b");
        let second = commit_file(&repo, "src/a.rs", "after\n", "update a");
        let _ = commit_file(&repo, "src/b.rs", "after\n", "update b");

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();

        let output = GitDiffBackend
            .compare_path_no_renames(
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

        assert_eq!(output.carbon.files.len(), 1);
        assert_eq!(output.carbon.files[0].path(), "src/a.rs");
    }

    #[test]
    fn builtin_backend_can_compute_deferred_stats_from_paths() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let first = commit_file(&repo, "src/a.rs", "same\nold\n", "initial");
        let second = commit_file(&repo, "src/a.rs", "same\nnew\nextra\n", "update");

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();

        let stats = GitDiffBackend.deferred_file_line_stats_batch_for_request(
            &CompareSpec {
                mode: CompareMode::TwoDot,
                left_ref: first,
                right_ref: second,
                renderer: RendererKind::Builtin,
                layout: LayoutMode::Unified,
            },
            &git,
            &[CompareFileStatsTarget {
                paths: CompareFilePaths::Same(ComparePath::from("src/a.rs")),
                status: carbon::FileStatus::Modified,
                is_binary: false,
                additions: 0,
                deletions: 0,
            }],
        );

        assert_eq!(stats, vec![Some((2, 1))]);
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
                &carbon::FileDiff {
                    old_path: Some("src/a.rs".to_owned()),
                    new_path: Some("src/a.rs".to_owned()),
                    old_oid: Some(carbon::ObjectId(old_oid)),
                    new_oid: Some(carbon::ObjectId(new_oid)),
                    is_partial: true,
                    ..carbon::FileDiff::default()
                },
                &git,
            )
            .unwrap()
            .unwrap();

        assert_eq!(output.carbon.files.len(), 1);
        assert_eq!(output.carbon.files[0].path(), "src/a.rs");
        assert!(!output.carbon.files[0].is_partial);
        let change = output.carbon.files[0]
            .blocks
            .iter()
            .find(|block| block.kind == carbon::BlockKind::Change)
            .expect("change block");
        let removed = output.carbon.files[0]
            .old_text
            .as_ref()
            .and_then(|text| text.line_str(carbon::LineId(change.old.start)))
            .expect("removed line");
        let added = output.carbon.files[0]
            .new_text
            .as_ref()
            .and_then(|text| text.line_str(carbon::LineId(change.new.start)))
            .expect("added line");
        assert_eq!(removed, "before");
        assert_eq!(added, "after");
    }
}
