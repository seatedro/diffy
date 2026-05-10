use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use rayon::prelude::*;

use vendored_difftastic::{
    ChangeIntensity as DftIntensity, DiffRequest, DiffStatus, SemanticDiffResult,
};

use crate::core::compare::backends::DiffBackend;
use crate::core::compare::progress::{ComparePhase, ProgressSink};
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::{CompareMode, CompareSpec};
use crate::core::compare::stats::{COMPARE_SUMMARY_FILE_LIMIT, CompareFileSummary};
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::git::{GitService, StatusItem, StatusScope, WORKDIR_REF};

/// Match git_diff.rs — throttle per-file emits so a 3k-file diff doesn't
/// flood the event channel.
const LOADING_FILE_EMIT_STRIDE: usize = 16;
const DIFFTASTIC_MAX_WORKERS: usize = 4;
const DIFFTASTIC_MIN_FILES_FOR_PARALLEL: usize = 4;

#[derive(Debug, Default, Clone, Copy)]
pub struct DifftasticBackend;

impl DifftasticBackend {
    pub const fn is_available() -> bool {
        true
    }

    pub fn compare_status_item(
        &self,
        item: &StatusItem,
        git: &GitService,
    ) -> Result<CompareOutput> {
        let changed = changed_path_for_status_item(git, item)?;
        compare_changed_paths(vec![changed], None)
    }

    pub fn compare_path(
        &self,
        spec: &CompareSpec,
        path: &str,
        git: &GitService,
    ) -> Result<Option<CompareOutput>> {
        let (left, right) = match spec.mode {
            CompareMode::TwoDot => {
                if spec.right_ref == WORKDIR_REF {
                    (git.resolve_ref(&spec.left_ref)?, WORKDIR_REF.to_owned())
                } else {
                    (
                        git.resolve_ref(&spec.left_ref)?,
                        git.resolve_ref(&spec.right_ref)?,
                    )
                }
            }
            CompareMode::ThreeDot | CompareMode::SingleCommit => {
                git.resolve_comparison(&spec.left_ref, &spec.right_ref, spec.mode)?
            }
        };

        let changed_paths = collect_changed_paths(git, &left, &right, Some(path))?;

        Ok(Some(compare_changed_paths(changed_paths, None)?))
    }
}

impl DiffBackend for DifftasticBackend {
    fn compare(
        &self,
        spec: &CompareSpec,
        git: &GitService,
        reporter: Option<&dyn ProgressSink>,
    ) -> Result<Option<CompareOutput>> {
        let (left, right) = match spec.mode {
            CompareMode::TwoDot => {
                if spec.right_ref == WORKDIR_REF {
                    (git.resolve_ref(&spec.left_ref)?, WORKDIR_REF.to_owned())
                } else {
                    (
                        git.resolve_ref(&spec.left_ref)?,
                        git.resolve_ref(&spec.right_ref)?,
                    )
                }
            }
            CompareMode::ThreeDot | CompareMode::SingleCommit => {
                git.resolve_comparison(&spec.left_ref, &spec.right_ref, spec.mode)?
            }
        };

        // Git enumeration phase — covers `diff_tree_to_tree` + rename detection.
        // Per-path blob loads and semantic diffs happen below only when the file
        // list is small enough to materialize eagerly.
        if let Some(r) = reporter {
            r.phase(ComparePhase::EnumeratingChanges);
        }

        let entries = collect_changed_path_entries(git, &left, &right, None)?;
        if should_defer_difftastic_files(entries.len(), &right) {
            return Ok(Some(compare_summaries_from_entries(entries)));
        }

        let changed_paths = collect_changed_paths_from_entries(git, &left, &right, entries)?;

        Ok(Some(compare_changed_paths(changed_paths, reporter)?))
    }
}

pub(crate) fn compare_changed_paths(
    changed_paths: Vec<DifftasticChangedPath>,
    reporter: Option<&dyn ProgressSink>,
) -> Result<CompareOutput> {
    let files_total = changed_paths.len() as u32;

    // Seed the determinate bar with a zero count so the UI swaps off the
    // shimmer immediately, before the first semantic diff blocks the
    // loop.
    if let Some(r) = reporter {
        r.phase(ComparePhase::LoadingFiles {
            files_seen: 0,
            files_total,
        });
    }

    let progress = AtomicU32::new(0);
    let worker_count = difftastic_worker_count(changed_paths.len());
    let files = if worker_count == 1 {
        changed_paths
            .into_iter()
            .enumerate()
            .map(|(idx, changed)| {
                let file = carbon_file_from_changed_path(changed, idx)?;
                report_file_loaded(reporter, &progress, files_total);
                Ok(file)
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_count)
            .thread_name(|idx| format!("diffy-difftastic-{idx}"))
            .build()
            .map_err(|error| {
                DiffyError::General(format!("difftastic worker setup failed: {error}"))
            })?;
        pool.install(|| {
            changed_paths
                .into_par_iter()
                .enumerate()
                .map(|(idx, changed)| {
                    let file = carbon_file_from_changed_path(changed, idx)?;
                    report_file_loaded(reporter, &progress, files_total);
                    Ok(file)
                })
                .collect::<Result<Vec<_>>>()
        })?
    };

    let mut output = CompareOutput::default();
    output.carbon.files = files;
    Ok(output)
}

fn difftastic_worker_count(file_count: usize) -> usize {
    if file_count < DIFFTASTIC_MIN_FILES_FOR_PARALLEL {
        return 1;
    }
    let available = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1);
    file_count.min(available).min(DIFFTASTIC_MAX_WORKERS).max(1)
}

fn report_file_loaded(reporter: Option<&dyn ProgressSink>, progress: &AtomicU32, files_total: u32) {
    let files_seen = progress.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    if let Some(r) = reporter {
        if files_seen % LOADING_FILE_EMIT_STRIDE as u32 == 0 || files_seen == files_total {
            r.phase(ComparePhase::LoadingFiles {
                files_seen,
                files_total,
            });
        }
    }
}

fn carbon_file_from_changed_path(
    changed: DifftasticChangedPath,
    file_id: usize,
) -> Result<carbon::FileDiff> {
    let display_path = changed
        .new_path
        .as_deref()
        .or(changed.old_path.as_deref())
        .unwrap_or_default();
    if changed.is_binary {
        return Ok(carbon_binary_file(display_path, &changed.status, file_id));
    }

    let old_src = String::from_utf8_lossy(&changed.old_content);
    let new_src = String::from_utf8_lossy(&changed.new_content);
    if let Some(file) = carbon_file_for_whole_file_change(
        display_path,
        &changed.status,
        changed.old_path.is_none(),
        changed.new_path.is_none(),
        &old_src,
        &new_src,
        file_id,
    ) {
        return Ok(file);
    }

    let semantic = vendored_difftastic::diff_bytes_semantic(DiffRequest {
        display_path,
        lhs_path: changed.old_path.as_deref().map(Path::new),
        rhs_path: changed.new_path.as_deref().map(Path::new),
        lhs_bytes: &changed.old_content,
        rhs_bytes: &changed.new_content,
    })
    .map_err(|error| DiffyError::General(format!("difftastic failed: {error}")))?;
    log_difftastic_semantic_result(
        display_path,
        &semantic,
        changed.old_content.len(),
        changed.new_content.len(),
    );

    Ok(carbon_file_from_semantic_result_with_id(
        &semantic,
        display_path,
        &changed.status,
        &old_src,
        &new_src,
        file_id,
    ))
}

fn log_difftastic_semantic_result(
    display_path: &str,
    result: &SemanticDiffResult,
    old_bytes: usize,
    new_bytes: usize,
) {
    if let Some(reason) = result
        .line_fallback_reason
        .as_deref()
        .or_else(|| difftastic_line_fallback_reason(&result.language))
    {
        tracing::info!(
            target: "diffy::difftastic",
            path = %display_path,
            reason,
            chunks = result.chunks.len(),
            aligned_lines = result.aligned_lines.len(),
            old_bytes,
            new_bytes,
            "difftastic semantic diff fell back to line diff"
        );
    }
}

fn difftastic_line_fallback_reason(language: &str) -> Option<&str> {
    language
        .strip_prefix("Text (")
        .and_then(|reason| reason.strip_suffix(')'))
}

fn carbon_file_for_whole_file_change(
    fallback_path: &str,
    fallback_status: &str,
    old_path_missing: bool,
    new_path_missing: bool,
    old_src: &str,
    new_src: &str,
    file_id: usize,
) -> Option<carbon::FileDiff> {
    let status = if old_path_missing || fallback_status == "A" {
        carbon::FileStatus::Added
    } else if new_path_missing || fallback_status == "D" {
        carbon::FileStatus::Deleted
    } else {
        return None;
    };

    let mut file = carbon::FileDiff {
        id: carbon::FileId(usize_to_u32_saturating(file_id)),
        old_path: (status != carbon::FileStatus::Added).then(|| fallback_path.to_owned()),
        new_path: (status != carbon::FileStatus::Deleted).then(|| fallback_path.to_owned()),
        status,
        ..carbon::FileDiff::default()
    };

    match status {
        carbon::FileStatus::Added => {
            let new_text = carbon::TextStore::from_text(new_src.to_owned());
            let new_count = new_text.line_count();
            file.additions = new_count;
            if new_count > 0 {
                let mut block = carbon::Block::change(
                    carbon::BlockId(0),
                    carbon::SourceRange::new(0, 0),
                    carbon::SourceRange::new(0, new_count),
                )
                .with_source_lines(1, 1);
                block.new_no_newline_at_end = new_text.no_newline_at_eof();
                file.new_text = Some(new_text);
                file.add_hunk(
                    carbon::Hunk::new(
                        carbon::HunkId(0),
                        1,
                        0,
                        1,
                        new_count,
                        carbon::BlockRange::default(),
                    ),
                    [block],
                );
            }
        }
        carbon::FileStatus::Deleted => {
            let old_text = carbon::TextStore::from_text(old_src.to_owned());
            let old_count = old_text.line_count();
            file.deletions = old_count;
            if old_count > 0 {
                let mut block = carbon::Block::change(
                    carbon::BlockId(0),
                    carbon::SourceRange::new(0, old_count),
                    carbon::SourceRange::new(0, 0),
                )
                .with_source_lines(1, 1);
                block.old_no_newline_at_end = old_text.no_newline_at_eof();
                file.old_text = Some(old_text);
                file.add_hunk(
                    carbon::Hunk::new(
                        carbon::HunkId(0),
                        1,
                        old_count,
                        1,
                        0,
                        carbon::BlockRange::default(),
                    ),
                    [block],
                );
            }
        }
        _ => return None,
    }

    Some(file)
}

fn carbon_file_from_semantic_result_with_id(
    result: &SemanticDiffResult,
    fallback_path: &str,
    fallback_status: &str,
    old_src: &str,
    new_src: &str,
    file_id: usize,
) -> carbon::FileDiff {
    let status = match result.status {
        DiffStatus::Created => carbon::FileStatus::Added,
        DiffStatus::Deleted => carbon::FileStatus::Deleted,
        DiffStatus::Unchanged => carbon::FileStatus::Modified,
        DiffStatus::Binary => {
            return carbon_binary_file(fallback_path, fallback_status, file_id);
        }
        DiffStatus::Changed => carbon_status_from_label(fallback_status, false),
    };

    let old_lines: Vec<&str> = old_src.split('\n').collect();
    let new_lines: Vec<&str> = new_src.split('\n').collect();

    let mut file = carbon::FileDiff {
        id: carbon::FileId(usize_to_u32_saturating(file_id)),
        old_path: (status != carbon::FileStatus::Added).then(|| fallback_path.to_owned()),
        new_path: (status != carbon::FileStatus::Deleted).then(|| fallback_path.to_owned()),
        status,
        prefer_structural_projection: result.line_fallback_reason.is_none()
            && difftastic_line_fallback_reason(&result.language).is_none(),
        ..carbon::FileDiff::default()
    };
    let mut old_text = String::new();
    let mut new_text = String::new();
    let mut old_store_count = 0u32;
    let mut new_store_count = 0u32;

    for chunk in &result.chunks {
        let mut blocks = Vec::new();
        let mut old_start: Option<u32> = None;
        let mut new_start: Option<u32> = None;
        let mut old_count = 0_u32;
        let mut new_count = 0_u32;

        // vendored_difftastic emits chunk lines in display order, so avoid
        // rebuilding and sorting an aligned-line index per file.
        for line in &chunk.lines {
            let lhs_line_no = line.lhs_line.map(|n| n.saturating_add(1));
            let rhs_line_no = line.rhs_line.map(|n| n.saturating_add(1));

            let lhs_text = line
                .lhs_line
                .and_then(|n| old_lines.get(n as usize).copied());
            let rhs_text = line
                .rhs_line
                .and_then(|n| new_lines.get(n as usize).copied());

            let is_context = lhs_text.is_some()
                && rhs_text.is_some()
                && lhs_text == rhs_text
                && semantic_spans_are_context(&line.lhs_changes)
                && semantic_spans_are_context(&line.rhs_changes);

            if is_context {
                if let Some(text) = lhs_text {
                    let old_block_start = old_store_count;
                    let new_block_start = new_store_count;
                    let old_line_start = lhs_line_no.unwrap_or_else(|| old_count.saturating_add(1));
                    let new_line_start = rhs_line_no.unwrap_or_else(|| new_count.saturating_add(1));
                    if let Some(n) = lhs_line_no {
                        old_start.get_or_insert(n);
                        old_count += 1;
                    }
                    if let Some(n) = rhs_line_no {
                        new_start.get_or_insert(n);
                        new_count += 1;
                    }
                    push_carbon_text_line(&mut old_text, text);
                    push_carbon_text_line(&mut new_text, text);
                    old_store_count = old_store_count.saturating_add(1);
                    new_store_count = new_store_count.saturating_add(1);
                    blocks.push(
                        carbon::Block::context(
                            carbon::BlockId(usize_to_u32_saturating(
                                file.blocks.len().saturating_add(blocks.len()),
                            )),
                            carbon::SourceRange::new(old_block_start, 1),
                            carbon::SourceRange::new(new_block_start, 1),
                        )
                        .with_source_lines(old_line_start, new_line_start),
                    );
                }
                continue;
            }

            let old_block_start = old_store_count;
            let new_block_start = new_store_count;
            let mut old_block_count = 0u32;
            let mut new_block_count = 0u32;
            let old_line_start = lhs_line_no.unwrap_or_else(|| old_count.saturating_add(1));
            let new_line_start = rhs_line_no.unwrap_or_else(|| new_count.saturating_add(1));
            let mut block = carbon::Block::change(
                carbon::BlockId(usize_to_u32_saturating(
                    file.blocks.len().saturating_add(blocks.len()),
                )),
                carbon::SourceRange::new(old_block_start, 0),
                carbon::SourceRange::new(new_block_start, 0),
            )
            .with_source_lines(old_line_start, new_line_start);

            if let Some(text) = lhs_text {
                if let Some(n) = lhs_line_no {
                    old_start.get_or_insert(n);
                    old_count += 1;
                }
                push_carbon_text_line(&mut old_text, text);
                old_store_count = old_store_count.saturating_add(1);
                old_block_count = old_block_count.saturating_add(1);
                block.old_inline = convert_change_spans(&line.lhs_changes);
                file.deletions = file.deletions.saturating_add(1);
            }

            if let Some(text) = rhs_text {
                if let Some(n) = rhs_line_no {
                    new_start.get_or_insert(n);
                    new_count += 1;
                }
                push_carbon_text_line(&mut new_text, text);
                new_store_count = new_store_count.saturating_add(1);
                new_block_count = new_block_count.saturating_add(1);
                block.new_inline = convert_change_spans(&line.rhs_changes);
                file.additions = file.additions.saturating_add(1);
            }
            block.old.len = old_block_count;
            block.new.len = new_block_count;
            blocks.push(block);
        }

        if !blocks.is_empty() {
            let computed_old_start = old_start.unwrap_or_else(|| new_start.unwrap_or(1));
            let computed_new_start = new_start.unwrap_or_else(|| old_start.unwrap_or(1));
            let hunk_id = carbon::HunkId(usize_to_u32_saturating(file.hunks.len()));
            file.add_hunk(
                carbon::Hunk::new(
                    hunk_id,
                    computed_old_start,
                    old_count,
                    computed_new_start,
                    new_count,
                    carbon::BlockRange::default(),
                ),
                blocks,
            );
        }
    }

    file.old_text = (old_store_count > 0).then(|| carbon::TextStore::from_text(old_text));
    file.new_text = (new_store_count > 0).then(|| carbon::TextStore::from_text(new_text));
    file
}

fn semantic_spans_are_context(spans: &[vendored_difftastic::ChangeSpan]) -> bool {
    spans
        .iter()
        .all(|span| span.intensity == DftIntensity::UnchangedContext)
}

fn push_carbon_text_line(text: &mut String, line: &str) {
    text.push_str(line);
    text.push('\n');
}

fn convert_change_spans(spans: &[vendored_difftastic::ChangeSpan]) -> Vec<carbon::InlineSpan> {
    spans
        .iter()
        .filter(|s| s.end_col > s.start_col)
        .map(|s| carbon::InlineSpan {
            offset: s.start_col,
            len: s.end_col - s.start_col,
            intensity: map_intensity(s.intensity),
        })
        .collect()
}

fn map_intensity(intensity: DftIntensity) -> carbon::ChangeIntensity {
    match intensity {
        DftIntensity::Novel => carbon::ChangeIntensity::Novel,
        DftIntensity::NovelWord => carbon::ChangeIntensity::NovelWord,
        DftIntensity::UnchangedContext => carbon::ChangeIntensity::UnchangedContext,
    }
}

fn carbon_binary_file(path: &str, status: &str, file_id: usize) -> carbon::FileDiff {
    carbon::FileDiff {
        id: carbon::FileId(usize_to_u32_saturating(file_id)),
        old_path: (status != "A").then(|| path.to_owned()),
        new_path: (status != "D").then(|| path.to_owned()),
        status: carbon::FileStatus::Binary,
        is_binary: true,
        is_partial: true,
        ..carbon::FileDiff::default()
    }
}

fn carbon_status_from_label(status: &str, is_binary: bool) -> carbon::FileStatus {
    if is_binary {
        carbon::FileStatus::Binary
    } else {
        match status {
            "A" => carbon::FileStatus::Added,
            "D" => carbon::FileStatus::Deleted,
            "R" => carbon::FileStatus::Renamed,
            _ => carbon::FileStatus::Modified,
        }
    }
}

type ChangedPathEntry = (String, Option<String>, Option<String>);

#[derive(Debug)]
pub(crate) struct DifftasticChangedPath {
    pub(crate) status: String,
    pub(crate) old_path: Option<String>,
    pub(crate) new_path: Option<String>,
    pub(crate) old_content: Vec<u8>,
    pub(crate) new_content: Vec<u8>,
    pub(crate) is_binary: bool,
}

fn should_defer_difftastic_files(file_count: usize, right: &str) -> bool {
    right != WORKDIR_REF && file_count > COMPARE_SUMMARY_FILE_LIMIT
}

fn collect_changed_path_entries(
    git: &GitService,
    left: &str,
    right: &str,
    only_path: Option<&str>,
) -> Result<Vec<ChangedPathEntry>> {
    git.diff_name_status(left, right, only_path)
}

fn compare_summaries_from_entries(entries: Vec<ChangedPathEntry>) -> CompareOutput {
    let mut output = CompareOutput {
        file_summaries: entries
            .into_iter()
            .map(|(status, old_path, new_path)| {
                CompareFileSummary::from_paths_status(
                    old_path.as_deref(),
                    new_path.as_deref(),
                    carbon_status_from_label(&status, false),
                    true,
                )
            })
            .collect(),
        ..CompareOutput::default()
    };
    output.compact_file_summaries();
    output
}

fn collect_changed_paths(
    git: &GitService,
    left: &str,
    right: &str,
    only_path: Option<&str>,
) -> Result<Vec<DifftasticChangedPath>> {
    let entries = collect_changed_path_entries(git, left, right, only_path)?;
    collect_changed_paths_from_entries(git, left, right, entries)
}

fn collect_changed_paths_from_entries(
    git: &GitService,
    left: &str,
    right: &str,
    entries: Vec<ChangedPathEntry>,
) -> Result<Vec<DifftasticChangedPath>> {
    let old_paths = entries
        .iter()
        .filter_map(|(_, old_path, _)| old_path.as_deref())
        .collect::<Vec<_>>();
    let new_reference = if right == WORKDIR_REF {
        WORKDIR_REF
    } else {
        right
    };
    let new_paths = entries
        .iter()
        .filter_map(|(_, _, new_path)| new_path.as_deref())
        .collect::<Vec<_>>();
    let mut old_contents = git.read_file_bytes_batch_at(left, old_paths).into_iter();
    let mut new_contents = git
        .read_file_bytes_batch_at(new_reference, new_paths)
        .into_iter();

    let mut changed = Vec::with_capacity(entries.len());
    for (status, old_path, new_path) in entries {
        let old_content = old_path
            .as_ref()
            .and_then(|_| old_contents.next().flatten());
        let new_content = new_path
            .as_ref()
            .and_then(|_| new_contents.next().flatten());
        let old_binary = old_content
            .as_ref()
            .is_some_and(|bytes| bytes.iter().take(1024).any(|b| *b == 0));
        let new_binary = new_content
            .as_ref()
            .is_some_and(|bytes| bytes.iter().take(1024).any(|b| *b == 0));
        changed.push(DifftasticChangedPath {
            status,
            old_path,
            new_path,
            old_content: old_content.unwrap_or_default(),
            new_content: new_content.unwrap_or_default(),
            is_binary: old_binary || new_binary,
        });
    }
    Ok(changed)
}

fn changed_path_for_status_item(
    git: &GitService,
    item: &StatusItem,
) -> Result<DifftasticChangedPath> {
    let old_content = match item.scope {
        StatusScope::Staged => git.read_file_bytes_at("HEAD", &item.path).ok(),
        StatusScope::Unstaged => git
            .read_file_bytes_at(crate::core::vcs::git::INDEX_REF, &item.path)
            .ok(),
        StatusScope::Untracked => None,
    };

    let new_content = match item.scope {
        StatusScope::Staged => git
            .read_file_bytes_at(crate::core::vcs::git::INDEX_REF, &item.path)
            .ok(),
        StatusScope::Unstaged | StatusScope::Untracked => {
            git.read_file_bytes_at(WORKDIR_REF, &item.path).ok()
        }
    };

    let old_binary = old_content
        .as_ref()
        .is_some_and(|bytes| bytes.iter().take(1024).any(|b| *b == 0));
    let new_binary = new_content
        .as_ref()
        .is_some_and(|bytes| bytes.iter().take(1024).any(|b| *b == 0));

    Ok(DifftasticChangedPath {
        status: item.status.clone(),
        old_path: match item.scope {
            StatusScope::Untracked => None,
            _ => Some(item.path.clone()),
        },
        new_path: if item.status == "D" && item.scope != StatusScope::Untracked {
            None
        } else {
            Some(item.path.clone())
        },
        old_content: old_content.unwrap_or_default(),
        new_content: new_content.unwrap_or_default(),
        is_binary: old_binary || new_binary,
    })
}

fn usize_to_u32_saturating(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Repository, Signature};
    use vendored_difftastic::{
        ChangeIntensity as DftIntensity, ChangeSpan, DiffStatus, HighlightKind, SemanticChunk,
        SemanticDiffResult, SemanticLine,
    };

    use super::{
        DifftasticBackend, carbon_file_from_semantic_result_with_id, collect_changed_paths,
        compare_summaries_from_entries, difftastic_line_fallback_reason, map_intensity,
        should_defer_difftastic_files,
    };
    use crate::core::compare::backends::DiffBackend;
    use crate::core::compare::spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};
    use crate::core::vcs::git::{GitService, WORKDIR_REF};
    use tempfile::TempDir;

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
    fn difftastic_fallback_reason_extracts_text_fallback() {
        assert_eq!(
            difftastic_line_fallback_reason(
                "Text (38 C parse errors, exceeded DFT_PARSE_ERROR_LIMIT)"
            ),
            Some("38 C parse errors, exceeded DFT_PARSE_ERROR_LIMIT")
        );
        assert_eq!(difftastic_line_fallback_reason("C"), None);
        assert_eq!(difftastic_line_fallback_reason("Text"), None);
    }

    #[test]
    fn carbon_semantic_builds_hunk_headers_for_modified_lines() {
        let result = SemanticDiffResult {
            status: DiffStatus::Changed,
            language: "Rust".to_owned(),
            line_fallback_reason: None,
            aligned_lines: vec![(Some(0), Some(0))],
            chunks: vec![SemanticChunk {
                lines: vec![SemanticLine {
                    lhs_line: Some(0),
                    rhs_line: Some(0),
                    lhs_changes: vec![ChangeSpan {
                        start_col: 4,
                        end_col: 7,
                        highlight: HighlightKind::Normal,
                        intensity: DftIntensity::Novel,
                    }],
                    rhs_changes: vec![ChangeSpan {
                        start_col: 4,
                        end_col: 7,
                        highlight: HighlightKind::Normal,
                        intensity: DftIntensity::Novel,
                    }],
                }],
            }],
        };

        let file = carbon_file_from_semantic_result_with_id(
            &result,
            "src/lib.rs",
            "M",
            "    old();\n",
            "    new();\n",
            0,
        );

        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].old_start, 1);
        assert_eq!(file.hunks[0].old_count, 1);
        assert_eq!(file.hunks[0].new_start, 1);
        assert!(file.prefer_structural_projection);
        assert_eq!(file.hunks[0].new_count, 1);
        assert_eq!(file.hunks[0].header, "@@ -1,1 +1,1 @@");
        assert_eq!(
            file.old_text
                .as_ref()
                .and_then(|text| text.line_str(carbon::LineId(0))),
            Some("    old();")
        );
        assert_eq!(
            file.new_text
                .as_ref()
                .and_then(|text| text.line_str(carbon::LineId(0))),
            Some("    new();")
        );
    }

    #[test]
    fn carbon_semantic_treats_unchanged_semantic_lines_as_context() {
        let result = SemanticDiffResult {
            status: DiffStatus::Changed,
            language: "Rust".to_owned(),
            line_fallback_reason: None,
            aligned_lines: vec![(Some(0), Some(0)), (Some(1), Some(1))],
            chunks: vec![SemanticChunk {
                lines: vec![
                    SemanticLine {
                        lhs_line: Some(0),
                        rhs_line: Some(0),
                        lhs_changes: vec![ChangeSpan {
                            start_col: 0,
                            end_col: 8,
                            highlight: HighlightKind::Normal,
                            intensity: DftIntensity::UnchangedContext,
                        }],
                        rhs_changes: vec![ChangeSpan {
                            start_col: 0,
                            end_col: 8,
                            highlight: HighlightKind::Normal,
                            intensity: DftIntensity::UnchangedContext,
                        }],
                    },
                    SemanticLine {
                        lhs_line: Some(1),
                        rhs_line: Some(1),
                        lhs_changes: vec![ChangeSpan {
                            start_col: 4,
                            end_col: 7,
                            highlight: HighlightKind::Normal,
                            intensity: DftIntensity::Novel,
                        }],
                        rhs_changes: vec![ChangeSpan {
                            start_col: 4,
                            end_col: 7,
                            highlight: HighlightKind::Normal,
                            intensity: DftIntensity::Novel,
                        }],
                    },
                ],
            }],
        };

        let file = carbon_file_from_semantic_result_with_id(
            &result,
            "src/lib.rs",
            "M",
            "same();\nold();\n",
            "same();\nnew();\n",
            0,
        );

        assert_eq!(file.blocks[0].kind, carbon::BlockKind::Context);
        assert_eq!(file.blocks[1].kind, carbon::BlockKind::Change);
        assert_eq!(file.additions, 1);
        assert_eq!(file.deletions, 1);
    }

    #[test]
    fn carbon_semantic_uses_text_store_content() {
        let result = SemanticDiffResult {
            status: DiffStatus::Changed,
            language: "Rust".to_owned(),
            line_fallback_reason: None,
            aligned_lines: vec![(Some(0), Some(0))],
            chunks: vec![SemanticChunk {
                lines: vec![SemanticLine {
                    lhs_line: Some(0),
                    rhs_line: Some(0),
                    lhs_changes: vec![ChangeSpan {
                        start_col: 4,
                        end_col: 7,
                        highlight: HighlightKind::Normal,
                        intensity: DftIntensity::Novel,
                    }],
                    rhs_changes: vec![ChangeSpan {
                        start_col: 4,
                        end_col: 7,
                        highlight: HighlightKind::Normal,
                        intensity: DftIntensity::Novel,
                    }],
                }],
            }],
        };

        let carbon_file = carbon_file_from_semantic_result_with_id(
            &result,
            "src/lib.rs",
            "M",
            "let old = 1;\n",
            "let new = 1;\n",
            9,
        );

        assert_eq!(carbon_file.id, carbon::FileId(9));
        assert_eq!(
            carbon_file
                .old_text
                .as_ref()
                .and_then(|text| text.line_str(carbon::LineId(0))),
            Some("let old = 1;")
        );
        assert_eq!(
            carbon_file
                .new_text
                .as_ref()
                .and_then(|text| text.line_str(carbon::LineId(0))),
            Some("let new = 1;")
        );
    }

    #[test]
    fn carbon_semantic_handles_pure_insert() {
        let result = SemanticDiffResult {
            status: DiffStatus::Changed,
            language: "Rust".to_owned(),
            line_fallback_reason: None,
            aligned_lines: vec![(None, Some(0))],
            chunks: vec![SemanticChunk {
                lines: vec![SemanticLine {
                    lhs_line: None,
                    rhs_line: Some(0),
                    lhs_changes: vec![],
                    rhs_changes: vec![ChangeSpan {
                        start_col: 0,
                        end_col: 8,
                        highlight: HighlightKind::Normal,
                        intensity: DftIntensity::Novel,
                    }],
                }],
            }],
        };

        let file = carbon_file_from_semantic_result_with_id(
            &result,
            "src/lib.rs",
            "M",
            "",
            "inserted\n",
            0,
        );

        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].old_count, 0);
        assert_eq!(file.hunks[0].new_start, 1);
        assert_eq!(file.hunks[0].new_count, 1);
    }

    #[test]
    fn carbon_semantic_preserves_change_intensity() {
        let result = SemanticDiffResult {
            status: DiffStatus::Changed,
            language: "Rust".to_owned(),
            line_fallback_reason: None,
            aligned_lines: vec![(Some(0), Some(0))],
            chunks: vec![SemanticChunk {
                lines: vec![SemanticLine {
                    lhs_line: Some(0),
                    rhs_line: Some(0),
                    lhs_changes: vec![
                        ChangeSpan {
                            start_col: 0,
                            end_col: 1,
                            highlight: HighlightKind::String,
                            intensity: DftIntensity::UnchangedContext,
                        },
                        ChangeSpan {
                            start_col: 1,
                            end_col: 4,
                            highlight: HighlightKind::String,
                            intensity: DftIntensity::NovelWord,
                        },
                        ChangeSpan {
                            start_col: 4,
                            end_col: 5,
                            highlight: HighlightKind::String,
                            intensity: DftIntensity::UnchangedContext,
                        },
                    ],
                    rhs_changes: vec![ChangeSpan {
                        start_col: 1,
                        end_col: 4,
                        highlight: HighlightKind::String,
                        intensity: DftIntensity::NovelWord,
                    }],
                }],
            }],
        };

        let file = carbon_file_from_semantic_result_with_id(
            &result,
            "src/lib.rs",
            "M",
            "\"foo\"\n",
            "\"bar\"\n",
            0,
        );

        let block = file.block(carbon::BlockId(0)).expect("change block");
        assert_eq!(block.old_inline.len(), 3);
        assert_eq!(
            block.old_inline[0].intensity,
            carbon::ChangeIntensity::UnchangedContext
        );
        assert_eq!(
            block.old_inline[1].intensity,
            carbon::ChangeIntensity::NovelWord
        );
        assert_eq!(
            block.old_inline[2].intensity,
            carbon::ChangeIntensity::UnchangedContext
        );

        assert_eq!(block.new_inline.len(), 1);
        assert_eq!(
            block.new_inline[0].intensity,
            carbon::ChangeIntensity::NovelWord
        );
    }

    #[test]
    fn intensity_mapping_covers_all_variants() {
        assert_eq!(
            map_intensity(DftIntensity::Novel),
            carbon::ChangeIntensity::Novel
        );
        assert_eq!(
            map_intensity(DftIntensity::NovelWord),
            carbon::ChangeIntensity::NovelWord
        );
        assert_eq!(
            map_intensity(DftIntensity::UnchangedContext),
            carbon::ChangeIntensity::UnchangedContext
        );
    }

    #[test]
    fn difftastic_large_compare_summaries_preserve_paths_and_status() {
        assert!(should_defer_difftastic_files(
            crate::core::compare::stats::COMPARE_SUMMARY_FILE_LIMIT + 1,
            "abc123"
        ));
        assert!(!should_defer_difftastic_files(
            crate::core::compare::stats::COMPARE_SUMMARY_FILE_LIMIT + 1,
            WORKDIR_REF
        ));

        let output = compare_summaries_from_entries(vec![
            ("A".to_owned(), None, Some("src/new.rs".to_owned())),
            ("D".to_owned(), Some("src/old.rs".to_owned()), None),
            (
                "R".to_owned(),
                Some("src/from.rs".to_owned()),
                Some("src/to.rs".to_owned()),
            ),
        ]);

        assert_eq!(output.carbon.files.len(), 0);
        assert_eq!(output.file_summaries.len(), 3);
        assert_eq!(output.file_summaries[0].path(), "src/new.rs");
        assert_eq!(output.file_summaries[0].status, carbon::FileStatus::Added);
        assert!(output.file_summaries[0].is_partial);
        assert!(output.file_summaries[0].stats_deferred);
        assert_eq!(output.file_summaries[1].path(), "src/old.rs");
        assert_eq!(output.file_summaries[1].status, carbon::FileStatus::Deleted);
        assert_eq!(output.file_summaries[2].path(), "src/to.rs");
        assert_eq!(output.file_summaries[2].status, carbon::FileStatus::Renamed);
    }

    #[test]
    fn collect_changed_paths_reads_current_workdir_content() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let head = commit_file(
            &repo,
            "src/lib.rs",
            "fn answer() {\n    old();\n}\n",
            "initial",
        );
        fs::write(
            repo_dir.path().join("src/lib.rs"),
            "fn answer() {\n    new();\n}\n",
        )
        .unwrap();

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();
        let changed = collect_changed_paths(&git, &head, WORKDIR_REF, None).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].new_path.as_deref(), Some("src/lib.rs"));
        assert_eq!(
            String::from_utf8_lossy(&changed[0].new_content),
            "fn answer() {\n    new();\n}\n"
        );
    }

    #[test]
    fn collect_changed_paths_can_filter_to_one_path() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let head = commit_file(&repo, "src/lib.rs", "before\n", "initial");
        commit_file(&repo, "src/other.rs", "stay\n", "add other");
        fs::write(repo_dir.path().join("src/lib.rs"), "after\n").unwrap();
        fs::write(repo_dir.path().join("src/other.rs"), "changed\n").unwrap();

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();
        let changed = collect_changed_paths(&git, &head, WORKDIR_REF, Some("src/lib.rs")).unwrap();

        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].new_path.as_deref(), Some("src/lib.rs"));
    }

    #[test]
    fn difftastic_backend_renders_added_files_without_semantic_diff() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let base = commit_file(&repo, "README.md", "base\n", "initial");
        let head = commit_file(&repo, "src/new.rs", "fn new() {}\nlet x = 1;\n", "add file");

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();

        let output = DifftasticBackend
            .compare(
                &CompareSpec {
                    mode: CompareMode::TwoDot,
                    left_ref: base,
                    right_ref: head,
                    renderer: RendererKind::Difftastic,
                    layout: LayoutMode::Unified,
                },
                &git,
                None,
            )
            .unwrap()
            .expect("difftastic result");

        let file = output
            .carbon
            .files
            .iter()
            .find(|file| file.path() == "src/new.rs")
            .expect("added file");
        assert_eq!(file.status, carbon::FileStatus::Added);
        assert_eq!(file.additions, 2);
        assert_eq!(file.deletions, 0);
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].old_count, 0);
        assert_eq!(file.hunks[0].new_count, 2);
        assert_eq!(
            file.new_text
                .as_ref()
                .and_then(|text| text.line_str(carbon::LineId(1))),
            Some("let x = 1;")
        );
    }

    #[test]
    fn difftastic_backend_supports_workdir_compare() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let head = commit_file(
            &repo,
            "src/lib.rs",
            "fn answer() {\n    old();\n}\n",
            "initial",
        );
        fs::write(
            repo_dir.path().join("src/lib.rs"),
            "fn answer() {\n    new();\n}\n",
        )
        .unwrap();

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();

        let output = DifftasticBackend
            .compare(
                &CompareSpec {
                    mode: CompareMode::TwoDot,
                    left_ref: head,
                    right_ref: WORKDIR_REF.to_owned(),
                    renderer: RendererKind::Difftastic,
                    layout: LayoutMode::Unified,
                },
                &git,
                None,
            )
            .unwrap()
            .expect("difftastic result");

        assert_eq!(output.carbon.files.len(), 1);
        let file = &output.carbon.files[0];
        assert_eq!(file.path(), "src/lib.rs");
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(
            file.old_text
                .as_ref()
                .and_then(|text| text.line_str(carbon::LineId(1))),
            Some("    old();")
        );
        assert_eq!(
            file.new_text
                .as_ref()
                .and_then(|text| text.line_str(carbon::LineId(1))),
            Some("    new();")
        );
    }
}
