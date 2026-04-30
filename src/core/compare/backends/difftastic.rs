use std::collections::HashMap;
use std::path::Path;

use vendored_difftastic::{
    ChangeIntensity as DftIntensity, DiffRequest, DiffStatus, SemanticDiffResult,
};

use crate::core::compare::backends::DiffBackend;
use crate::core::compare::progress::{ComparePhase, ProgressSink};
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::{CompareMode, CompareSpec};
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::git::{GitService, StatusItem, StatusScope, WORKDIR_REF};

/// Match git_diff.rs — throttle per-file emits so a 3k-file diff doesn't
/// flood the event channel.
const LOADING_FILE_EMIT_STRIDE: usize = 16;

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

        // Git enumeration phase — covers `collect_changed_paths` which
        // runs `diff_tree_to_tree` + rename detection + per-path blob
        // loads. Distinguished from the per-file semantic diff that
        // follows, which is the dominant cost for difftastic on large
        // repos.
        if let Some(r) = reporter {
            r.phase(ComparePhase::EnumeratingChanges);
        }

        let changed_paths = collect_changed_paths(git, &left, &right, None)?;

        Ok(Some(compare_changed_paths(changed_paths, reporter)?))
    }
}

fn compare_changed_paths(
    changed_paths: Vec<ChangedPath>,
    reporter: Option<&dyn ProgressSink>,
) -> Result<CompareOutput> {
    let mut output = CompareOutput::default();
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

    for (idx, changed) in changed_paths.into_iter().enumerate() {
        if let Some(r) = reporter {
            let is_last = (idx as u32) + 1 == files_total;
            if idx % LOADING_FILE_EMIT_STRIDE == 0 || is_last {
                r.phase(ComparePhase::LoadingFiles {
                    files_seen: (idx + 1) as u32,
                    files_total,
                });
            }
        }
        let display_path = changed
            .new_path
            .as_deref()
            .or(changed.old_path.as_deref())
            .unwrap_or_default();
        if changed.is_binary {
            output
                .carbon
                .files
                .push(carbon_binary_file(display_path, &changed.status, idx));
            continue;
        }

        let semantic = vendored_difftastic::diff_bytes_semantic(DiffRequest {
            display_path,
            lhs_path: changed.old_path.as_deref().map(Path::new),
            rhs_path: changed.new_path.as_deref().map(Path::new),
            lhs_bytes: &changed.old_content,
            rhs_bytes: &changed.new_content,
        })
        .map_err(|error| DiffyError::General(format!("difftastic failed: {error}")))?;
        let old_src = String::from_utf8_lossy(&changed.old_content);
        let new_src = String::from_utf8_lossy(&changed.new_content);
        output
            .carbon
            .files
            .push(carbon_file_from_semantic_result_with_id(
                &semantic,
                display_path,
                &changed.status,
                &old_src,
                &new_src,
                idx,
            ));
    }

    Ok(output)
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

    let aligned_order: HashMap<(Option<u32>, Option<u32>), usize> = result
        .aligned_lines
        .iter()
        .enumerate()
        .map(|(i, &(l, r))| ((l, r), i))
        .collect();

    let mut file = carbon::FileDiff {
        id: carbon::FileId(usize_to_u32_saturating(file_id)),
        old_path: (status != carbon::FileStatus::Added).then(|| fallback_path.to_owned()),
        new_path: (status != carbon::FileStatus::Deleted).then(|| fallback_path.to_owned()),
        status,
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

        let mut sorted_lines: Vec<_> = chunk.lines.iter().collect();
        sorted_lines.sort_by_key(|line| {
            aligned_order
                .get(&(line.lhs_line, line.rhs_line))
                .copied()
                .unwrap_or(usize::MAX)
        });

        for line in sorted_lines {
            let lhs_line_no = line.lhs_line.map(|n| n.saturating_add(1));
            let rhs_line_no = line.rhs_line.map(|n| n.saturating_add(1));

            let lhs_text = line
                .lhs_line
                .and_then(|n| old_lines.get(n as usize).copied());
            let rhs_text = line
                .rhs_line
                .and_then(|n| new_lines.get(n as usize).copied());

            let is_context = line.lhs_changes.is_empty()
                && line.rhs_changes.is_empty()
                && lhs_text.is_some()
                && rhs_text.is_some()
                && lhs_text == rhs_text;

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

#[derive(Debug)]
struct ChangedPath {
    status: String,
    old_path: Option<String>,
    new_path: Option<String>,
    old_content: Vec<u8>,
    new_content: Vec<u8>,
    is_binary: bool,
}

fn collect_changed_paths(
    git: &GitService,
    left: &str,
    right: &str,
    only_path: Option<&str>,
) -> Result<Vec<ChangedPath>> {
    let mut changed = Vec::new();
    for (status, old_path, new_path) in git.diff_name_status(left, right, only_path)? {
        let old_content = if let Some(path) = old_path.as_deref() {
            git.read_file_bytes_at(left, path).ok()
        } else {
            None
        };
        let new_reference = if right == WORKDIR_REF {
            WORKDIR_REF
        } else {
            right
        };
        let new_content = if let Some(path) = new_path.as_deref() {
            git.read_file_bytes_at(new_reference, path).ok()
        } else {
            None
        };
        let old_binary = old_content
            .as_ref()
            .is_some_and(|bytes| bytes.iter().take(1024).any(|b| *b == 0));
        let new_binary = new_content
            .as_ref()
            .is_some_and(|bytes| bytes.iter().take(1024).any(|b| *b == 0));
        changed.push(ChangedPath {
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

fn changed_path_for_status_item(git: &GitService, item: &StatusItem) -> Result<ChangedPath> {
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

    Ok(ChangedPath {
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
        map_intensity,
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
    fn carbon_semantic_builds_hunk_headers_for_modified_lines() {
        let result = SemanticDiffResult {
            status: DiffStatus::Changed,
            language: "Rust".to_owned(),
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
    fn carbon_semantic_uses_text_store_content() {
        let result = SemanticDiffResult {
            status: DiffStatus::Changed,
            language: "Rust".to_owned(),
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

        let changed = collect_changed_paths(&repo, &head, WORKDIR_REF, None).unwrap();
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

        let changed = collect_changed_paths(&repo, &head, WORKDIR_REF, Some("src/lib.rs")).unwrap();

        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].new_path.as_deref(), Some("src/lib.rs"));
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
}
