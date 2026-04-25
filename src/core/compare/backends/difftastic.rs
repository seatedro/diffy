use std::collections::HashMap;
use std::fs;
use std::path::Path;

use git2::{Delta, DiffOptions, ObjectType, Repository};
use vendored_difftastic::{
    ChangeIntensity as DftIntensity, DiffRequest, DiffStatus, HighlightKind, SemanticDiffResult,
};

use crate::core::compare::backends::DiffBackend;
use crate::core::compare::progress::{ComparePhase, ProgressSink};
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::{CompareMode, CompareSpec};
use crate::core::diff::{DiffLine, FileDiff, Hunk, LineKind};
use crate::core::error::{DiffyError, Result};
use crate::core::text::{ChangeIntensity, DiffTokenSpan, SyntaxTokenKind, TextBuffer, TokenBuffer};
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
        let repo = git.repo()?;
        let changed = changed_path_for_status_item(repo, item)?;
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

        let repo = git.repo()?;
        let changed_paths = collect_changed_paths(repo, &left, &right, Some(path))?;

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

        let repo = git.repo()?;
        let changed_paths = collect_changed_paths(repo, &left, &right, None)?;

        Ok(Some(compare_changed_paths(changed_paths, reporter)?))
    }
}

fn compare_changed_paths(
    changed_paths: Vec<ChangedPath>,
    reporter: Option<&dyn ProgressSink>,
) -> Result<CompareOutput> {
    let mut output = CompareOutput::default();
    let mut text_buffer = TextBuffer::default();
    let mut token_buffer = TokenBuffer::default();
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
            let file = FileDiff {
                path: display_path.to_owned(),
                status: changed.status,
                is_binary: true,
                ..FileDiff::default()
            };
            output
                .carbon
                .files
                .push(carbon_file_from_semantic_file(&file, &text_buffer, idx));
            output.files.push(file);
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
        let file = convert_semantic_result(
            &semantic,
            display_path,
            &changed.status,
            &old_src,
            &new_src,
            &mut text_buffer,
            &mut token_buffer,
        );
        output
            .carbon
            .files
            .push(carbon_file_from_semantic_file(&file, &text_buffer, idx));
        output.files.push(file);
    }

    output.text_buffer = text_buffer;
    output.token_buffer = token_buffer;
    Ok(output)
}

fn convert_semantic_result(
    result: &SemanticDiffResult,
    fallback_path: &str,
    fallback_status: &str,
    old_src: &str,
    new_src: &str,
    text_buffer: &mut TextBuffer,
    token_buffer: &mut TokenBuffer,
) -> FileDiff {
    let status = match result.status {
        DiffStatus::Created => "A".to_owned(),
        DiffStatus::Deleted => "D".to_owned(),
        DiffStatus::Unchanged => "U".to_owned(),
        DiffStatus::Binary => {
            return FileDiff {
                path: fallback_path.to_owned(),
                status: fallback_status.to_owned(),
                is_binary: true,
                ..FileDiff::default()
            };
        }
        DiffStatus::Changed => fallback_status.to_owned(),
    };

    let old_lines: Vec<&str> = old_src.split('\n').collect();
    let new_lines: Vec<&str> = new_src.split('\n').collect();

    let aligned_order: HashMap<(Option<u32>, Option<u32>), usize> = result
        .aligned_lines
        .iter()
        .enumerate()
        .map(|(i, &(l, r))| ((l, r), i))
        .collect();

    let mut file = FileDiff {
        path: fallback_path.to_owned(),
        status,
        ..FileDiff::default()
    };

    for chunk in &result.chunks {
        let mut hunk = Hunk::default();
        let mut old_start: Option<i32> = None;
        let mut new_start: Option<i32> = None;
        let mut old_count = 0_i32;
        let mut new_count = 0_i32;
        let mut next_pair_id = 0_u32;

        let mut sorted_lines: Vec<_> = chunk.lines.iter().collect();
        sorted_lines.sort_by_key(|line| {
            aligned_order
                .get(&(line.lhs_line, line.rhs_line))
                .copied()
                .unwrap_or(usize::MAX)
        });

        for line in sorted_lines {
            let lhs_line_no = line.lhs_line.map(|n| (n + 1) as i32);
            let rhs_line_no = line.rhs_line.map(|n| (n + 1) as i32);

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
                    if let Some(n) = lhs_line_no {
                        old_start.get_or_insert(n);
                        old_count += 1;
                    }
                    if let Some(n) = rhs_line_no {
                        new_start.get_or_insert(n);
                        new_count += 1;
                    }
                    let range = text_buffer.append(text);
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Context,
                        old_line_number: lhs_line_no,
                        new_line_number: rhs_line_no,
                        text_range: range,
                        ..DiffLine::default()
                    });
                }
                continue;
            }

            let pair_id = Some(next_pair_id);
            next_pair_id = next_pair_id.saturating_add(1);

            if let Some(text) = lhs_text {
                if let Some(n) = lhs_line_no {
                    old_start.get_or_insert(n);
                    old_count += 1;
                }
                let text_range = text_buffer.append(text);
                let tokens = convert_change_spans(&line.lhs_changes);
                let change_tokens = token_buffer.append(&tokens);
                hunk.lines.push(DiffLine {
                    kind: LineKind::Removed,
                    old_line_number: lhs_line_no,
                    new_line_number: None,
                    text_range,
                    change_tokens,
                    pair_id,
                    ..DiffLine::default()
                });
                file.deletions += 1;
            }

            if let Some(text) = rhs_text {
                if let Some(n) = rhs_line_no {
                    new_start.get_or_insert(n);
                    new_count += 1;
                }
                let text_range = text_buffer.append(text);
                let tokens = convert_change_spans(&line.rhs_changes);
                let change_tokens = token_buffer.append(&tokens);
                hunk.lines.push(DiffLine {
                    kind: LineKind::Added,
                    old_line_number: None,
                    new_line_number: rhs_line_no,
                    text_range,
                    change_tokens,
                    pair_id,
                    ..DiffLine::default()
                });
                file.additions += 1;
            }
        }

        if !hunk.lines.is_empty() {
            let computed_old_start =
                old_start.unwrap_or_else(|| new_start.unwrap_or(0).saturating_sub(1));
            let computed_new_start =
                new_start.unwrap_or_else(|| old_start.unwrap_or(0).saturating_sub(1));
            hunk.old_start = computed_old_start;
            hunk.old_count = old_count;
            hunk.new_start = computed_new_start;
            hunk.new_count = new_count;
            hunk.header = format!(
                "@@ -{},{} +{},{} @@",
                hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
            );
            file.hunks.push(hunk);
        }
    }

    file
}

fn carbon_file_from_semantic_file(
    file: &FileDiff,
    text_buffer: &TextBuffer,
    file_id: usize,
) -> carbon::FileDiff {
    let mut carbon_file = carbon::FileDiff {
        id: carbon::FileId(usize_to_u32_saturating(file_id)),
        old_path: (file.status != "A").then(|| file.path.clone()),
        new_path: (file.status != "D").then(|| file.path.clone()),
        status: carbon_status(file),
        is_binary: file.is_binary,
        is_partial: true,
        ..carbon::FileDiff::default()
    };
    if file.is_binary {
        return carbon_file;
    }

    let mut old_text = String::new();
    let mut new_text = String::new();
    let mut old_count = 0u32;
    let mut new_count = 0u32;

    for (hunk_index, hunk) in file.hunks.iter().enumerate() {
        let mut blocks = Vec::new();
        let mut line_index = 0usize;

        while line_index < hunk.lines.len() {
            match hunk.lines[line_index].kind {
                LineKind::Context => {
                    let old_start = old_count;
                    let new_start = new_count;
                    let old_line_start = hunk.lines[line_index]
                        .old_line_number
                        .map(i32_to_u32_nonnegative)
                        .unwrap_or_else(|| old_start.saturating_add(1));
                    let new_line_start = hunk.lines[line_index]
                        .new_line_number
                        .map(i32_to_u32_nonnegative)
                        .unwrap_or_else(|| new_start.saturating_add(1));
                    let mut count = 0u32;
                    while line_index < hunk.lines.len()
                        && hunk.lines[line_index].kind == LineKind::Context
                    {
                        let text = text_buffer.view(hunk.lines[line_index].text_range);
                        push_carbon_text_line(&mut old_text, text);
                        push_carbon_text_line(&mut new_text, text);
                        old_count = old_count.saturating_add(1);
                        new_count = new_count.saturating_add(1);
                        count = count.saturating_add(1);
                        line_index += 1;
                    }
                    blocks.push(
                        carbon::Block::context(
                            carbon::BlockId(usize_to_u32_saturating(
                                carbon_file.blocks.len().saturating_add(blocks.len()),
                            )),
                            carbon::SourceRange::new(old_start, count),
                            carbon::SourceRange::new(new_start, count),
                        )
                        .with_source_lines(old_line_start, new_line_start),
                    );
                }
                LineKind::Added | LineKind::Removed => {
                    let old_start = old_count;
                    let new_start = new_count;
                    let mut old_line_start = old_start.saturating_add(1);
                    let mut new_line_start = new_start.saturating_add(1);
                    let mut old_block_count = 0u32;
                    let mut new_block_count = 0u32;
                    let mut saw_old = false;
                    let mut saw_new = false;
                    while line_index < hunk.lines.len()
                        && hunk.lines[line_index].kind != LineKind::Context
                    {
                        let line = &hunk.lines[line_index];
                        let text = text_buffer.view(line.text_range);
                        match line.kind {
                            LineKind::Removed => {
                                if !saw_old {
                                    old_line_start = line
                                        .old_line_number
                                        .map(i32_to_u32_nonnegative)
                                        .unwrap_or_else(|| old_count.saturating_add(1));
                                    saw_old = true;
                                }
                                push_carbon_text_line(&mut old_text, text);
                                old_count = old_count.saturating_add(1);
                                old_block_count = old_block_count.saturating_add(1);
                            }
                            LineKind::Added => {
                                if !saw_new {
                                    new_line_start = line
                                        .new_line_number
                                        .map(i32_to_u32_nonnegative)
                                        .unwrap_or_else(|| new_count.saturating_add(1));
                                    saw_new = true;
                                }
                                push_carbon_text_line(&mut new_text, text);
                                new_count = new_count.saturating_add(1);
                                new_block_count = new_block_count.saturating_add(1);
                            }
                            LineKind::Context => {}
                        }
                        line_index += 1;
                    }
                    blocks.push(
                        carbon::Block::change(
                            carbon::BlockId(usize_to_u32_saturating(
                                carbon_file.blocks.len().saturating_add(blocks.len()),
                            )),
                            carbon::SourceRange::new(old_start, old_block_count),
                            carbon::SourceRange::new(new_start, new_block_count),
                        )
                        .with_source_lines(old_line_start, new_line_start),
                    );
                }
            }
        }

        let mut carbon_hunk = carbon::Hunk::new(
            carbon::HunkId(usize_to_u32_saturating(hunk_index)),
            i32_to_u32_nonnegative(hunk.old_start),
            i32_to_u32_nonnegative(hunk.old_count),
            i32_to_u32_nonnegative(hunk.new_start),
            i32_to_u32_nonnegative(hunk.new_count),
            carbon::BlockRange::default(),
        );
        carbon_hunk.header.clone_from(&hunk.header);
        carbon_file.add_hunk(carbon_hunk, blocks);
    }

    carbon_file.old_text = (old_count > 0).then(|| carbon::TextStore::from_text(old_text));
    carbon_file.new_text = (new_count > 0).then(|| carbon::TextStore::from_text(new_text));
    carbon_file
}

fn carbon_status(file: &FileDiff) -> carbon::FileStatus {
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

fn push_carbon_text_line(text: &mut String, line: &str) {
    text.push_str(line);
    text.push('\n');
}

fn convert_change_spans(spans: &[vendored_difftastic::ChangeSpan]) -> Vec<DiffTokenSpan> {
    spans
        .iter()
        .filter(|s| s.end_col > s.start_col)
        .map(|s| DiffTokenSpan {
            offset: s.start_col,
            length: s.end_col - s.start_col,
            kind: map_highlight(s.highlight),
            intensity: map_intensity(s.intensity),
        })
        .collect()
}

fn map_highlight(kind: HighlightKind) -> SyntaxTokenKind {
    match kind {
        HighlightKind::Normal => SyntaxTokenKind::Normal,
        HighlightKind::Keyword => SyntaxTokenKind::Keyword,
        HighlightKind::String => SyntaxTokenKind::String,
        HighlightKind::Comment => SyntaxTokenKind::Comment,
        HighlightKind::Number => SyntaxTokenKind::Number,
        HighlightKind::Type => SyntaxTokenKind::Type,
        HighlightKind::Function => SyntaxTokenKind::Function,
        HighlightKind::Operator => SyntaxTokenKind::Operator,
        HighlightKind::Punctuation => SyntaxTokenKind::Punctuation,
        HighlightKind::Variable => SyntaxTokenKind::Variable,
        HighlightKind::Constant => SyntaxTokenKind::Constant,
        HighlightKind::Builtin => SyntaxTokenKind::Builtin,
        HighlightKind::Attribute => SyntaxTokenKind::Attribute,
        HighlightKind::Tag => SyntaxTokenKind::Tag,
        HighlightKind::Property => SyntaxTokenKind::Property,
        HighlightKind::Namespace => SyntaxTokenKind::Namespace,
        HighlightKind::Label => SyntaxTokenKind::Label,
        HighlightKind::Preprocessor => SyntaxTokenKind::Preprocessor,
    }
}

fn map_intensity(intensity: DftIntensity) -> ChangeIntensity {
    match intensity {
        DftIntensity::Novel => ChangeIntensity::Novel,
        DftIntensity::NovelWord => ChangeIntensity::NovelWord,
        DftIntensity::UnchangedContext => ChangeIntensity::UnchangedContext,
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
    repo: &Repository,
    left: &str,
    right: &str,
    only_path: Option<&str>,
) -> Result<Vec<ChangedPath>> {
    let left_tree = repo
        .revparse_single(left)?
        .peel(ObjectType::Commit)?
        .peel_to_tree()?;
    let right_tree = repo
        .revparse_single(right)
        .ok()
        .and_then(|object| object.peel(ObjectType::Commit).ok())
        .and_then(|object| object.peel_to_tree().ok());
    let mut options = DiffOptions::new();
    options.context_lines(3);
    if let Some(path) = only_path {
        options.pathspec(path);
    }
    let is_workdir = right == WORKDIR_REF;
    let workdir = if is_workdir {
        Some(
            repo.workdir()
                .ok_or_else(|| DiffyError::General("repository has no workdir".to_owned()))?,
        )
    } else {
        None
    };
    let mut diff = if is_workdir {
        repo.diff_tree_to_workdir_with_index(Some(&left_tree), Some(&mut options))?
    } else {
        repo.diff_tree_to_tree(Some(&left_tree), right_tree.as_ref(), Some(&mut options))?
    };
    diff.find_similar(None)?;

    let mut changed = Vec::new();
    for delta in diff.deltas() {
        let old_content = load_blob_content(repo, delta.old_file().id())?;
        let new_content = if let Some(workdir) = workdir {
            load_workdir_content(
                workdir,
                delta.status(),
                delta.old_file().path(),
                delta.new_file().path(),
            )?
        } else {
            load_blob_content(repo, delta.new_file().id())?
        };
        let old_binary = old_content
            .as_ref()
            .is_some_and(|bytes| bytes.iter().take(1024).any(|b| *b == 0));
        let new_binary = new_content
            .as_ref()
            .is_some_and(|bytes| bytes.iter().take(1024).any(|b| *b == 0));
        changed.push(ChangedPath {
            status: match delta.status() {
                Delta::Added => "A".to_owned(),
                Delta::Deleted => "D".to_owned(),
                Delta::Renamed => "R".to_owned(),
                _ => "M".to_owned(),
            },
            old_path: delta
                .old_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned()),
            new_path: delta
                .new_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned()),
            old_content: old_content.unwrap_or_default(),
            new_content: new_content.unwrap_or_default(),
            is_binary: old_binary || new_binary,
        });
    }
    Ok(changed)
}

fn changed_path_for_status_item(repo: &Repository, item: &StatusItem) -> Result<ChangedPath> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| DiffyError::General("repository has no workdir".to_owned()))?;
    let path = Path::new(&item.path);
    let index = repo.index()?;

    let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
    let head_entry = head_tree.as_ref().and_then(|tree| tree.get_path(path).ok());
    let index_entry = index.get_path(path, 0);

    let old_content = match item.scope {
        StatusScope::Staged => head_entry
            .as_ref()
            .map(|entry| {
                repo.find_blob(entry.id())
                    .map(|blob| blob.content().to_vec())
            })
            .transpose()?,
        StatusScope::Unstaged => index_entry
            .as_ref()
            .map(|entry| repo.find_blob(entry.id).map(|blob| blob.content().to_vec()))
            .transpose()?,
        StatusScope::Untracked => None,
    };

    let new_content = match item.scope {
        StatusScope::Staged => index_entry
            .as_ref()
            .map(|entry| repo.find_blob(entry.id).map(|blob| blob.content().to_vec()))
            .transpose()?,
        StatusScope::Unstaged | StatusScope::Untracked => {
            let absolute_path = workdir.join(path);
            match fs::read(&absolute_path) {
                Ok(content) => Some(content),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            }
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

fn load_blob_content(repo: &Repository, oid: git2::Oid) -> Result<Option<Vec<u8>>> {
    if oid.is_zero() {
        return Ok(None);
    }
    Ok(Some(repo.find_blob(oid)?.content().to_vec()))
}

fn load_workdir_content(
    workdir: &Path,
    status: Delta,
    old_path: Option<&Path>,
    new_path: Option<&Path>,
) -> Result<Option<Vec<u8>>> {
    let relative_path = match status {
        Delta::Deleted => return Ok(None),
        _ => new_path.or(old_path),
    };
    let Some(relative_path) = relative_path else {
        return Ok(None);
    };
    let absolute_path = workdir.join(relative_path);
    match fs::read(&absolute_path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn i32_to_u32_nonnegative(value: i32) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
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
        DifftasticBackend, carbon_file_from_semantic_file, collect_changed_paths,
        convert_semantic_result, map_highlight, map_intensity,
    };
    use crate::core::compare::backends::DiffBackend;
    use crate::core::compare::spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};
    use crate::core::text::{ChangeIntensity, SyntaxTokenKind, TextBuffer, TokenBuffer};
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
    fn convert_semantic_builds_hunk_headers_for_modified_lines() {
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
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        let file = convert_semantic_result(
            &result,
            "src/lib.rs",
            "M",
            "    old();\n",
            "    new();\n",
            &mut text_buffer,
            &mut token_buffer,
        );

        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].old_start, 1);
        assert_eq!(file.hunks[0].old_count, 1);
        assert_eq!(file.hunks[0].new_start, 1);
        assert_eq!(file.hunks[0].new_count, 1);
        assert_eq!(file.hunks[0].header, "@@ -1,1 +1,1 @@");
        assert_eq!(
            text_buffer.view(file.hunks[0].lines[0].text_range),
            "    old();"
        );
        assert_eq!(
            text_buffer.view(file.hunks[0].lines[1].text_range),
            "    new();"
        );
    }

    #[test]
    fn carbon_file_from_semantic_file_uses_text_store_content() {
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
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();
        let file = convert_semantic_result(
            &result,
            "src/lib.rs",
            "M",
            "let old = 1;\n",
            "let new = 1;\n",
            &mut text_buffer,
            &mut token_buffer,
        );

        let carbon_file = carbon_file_from_semantic_file(&file, &text_buffer, 9);

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
    fn convert_semantic_handles_pure_insert() {
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
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        let file = convert_semantic_result(
            &result,
            "src/lib.rs",
            "M",
            "",
            "inserted\n",
            &mut text_buffer,
            &mut token_buffer,
        );

        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].old_count, 0);
        assert_eq!(file.hunks[0].new_start, 1);
        assert_eq!(file.hunks[0].new_count, 1);
    }

    #[test]
    fn convert_semantic_preserves_change_intensity() {
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
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        let file = convert_semantic_result(
            &result,
            "src/lib.rs",
            "M",
            "\"foo\"\n",
            "\"bar\"\n",
            &mut text_buffer,
            &mut token_buffer,
        );

        let removed_tokens = token_buffer.view(file.hunks[0].lines[0].change_tokens);
        assert_eq!(removed_tokens.len(), 3);
        assert_eq!(
            removed_tokens[0].intensity,
            ChangeIntensity::UnchangedContext
        );
        assert_eq!(removed_tokens[1].intensity, ChangeIntensity::NovelWord);
        assert_eq!(
            removed_tokens[2].intensity,
            ChangeIntensity::UnchangedContext
        );

        let added_tokens = token_buffer.view(file.hunks[0].lines[1].change_tokens);
        assert_eq!(added_tokens.len(), 1);
        assert_eq!(added_tokens[0].intensity, ChangeIntensity::NovelWord);
    }

    #[test]
    fn convert_semantic_assigns_pair_ids() {
        let result = SemanticDiffResult {
            status: DiffStatus::Changed,
            language: "Rust".to_owned(),
            aligned_lines: vec![(Some(0), None), (None, Some(0))],
            chunks: vec![SemanticChunk {
                lines: vec![
                    SemanticLine {
                        lhs_line: Some(0),
                        rhs_line: None,
                        lhs_changes: vec![ChangeSpan {
                            start_col: 0,
                            end_col: 7,
                            highlight: HighlightKind::Normal,
                            intensity: DftIntensity::Novel,
                        }],
                        rhs_changes: vec![],
                    },
                    SemanticLine {
                        lhs_line: None,
                        rhs_line: Some(0),
                        lhs_changes: vec![],
                        rhs_changes: vec![ChangeSpan {
                            start_col: 0,
                            end_col: 5,
                            highlight: HighlightKind::Normal,
                            intensity: DftIntensity::Novel,
                        }],
                    },
                ],
            }],
        };
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        let file = convert_semantic_result(
            &result,
            "src/lib.rs",
            "M",
            "removed\n",
            "added\n",
            &mut text_buffer,
            &mut token_buffer,
        );

        assert_eq!(file.hunks.len(), 1);
        assert!(file.hunks[0].lines[0].pair_id.is_some());
        assert!(file.hunks[0].lines[1].pair_id.is_some());
        assert_ne!(
            file.hunks[0].lines[0].pair_id,
            file.hunks[0].lines[1].pair_id
        );
    }

    #[test]
    fn highlight_mapping_covers_all_variants() {
        assert_eq!(
            map_highlight(HighlightKind::Normal),
            SyntaxTokenKind::Normal
        );
        assert_eq!(
            map_highlight(HighlightKind::Keyword),
            SyntaxTokenKind::Keyword
        );
        assert_eq!(
            map_highlight(HighlightKind::String),
            SyntaxTokenKind::String
        );
        assert_eq!(
            map_highlight(HighlightKind::Comment),
            SyntaxTokenKind::Comment
        );
        assert_eq!(map_highlight(HighlightKind::Type), SyntaxTokenKind::Type);
        assert_eq!(
            map_highlight(HighlightKind::Punctuation),
            SyntaxTokenKind::Punctuation
        );
        assert_eq!(
            map_highlight(HighlightKind::Preprocessor),
            SyntaxTokenKind::Preprocessor
        );
    }

    #[test]
    fn intensity_mapping_covers_all_variants() {
        assert_eq!(map_intensity(DftIntensity::Novel), ChangeIntensity::Novel);
        assert_eq!(
            map_intensity(DftIntensity::NovelWord),
            ChangeIntensity::NovelWord
        );
        assert_eq!(
            map_intensity(DftIntensity::UnchangedContext),
            ChangeIntensity::UnchangedContext
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

        assert_eq!(output.files.len(), 1);
        let file = &output.files[0];
        assert_eq!(file.path, "src/lib.rs");
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(output.carbon.files.len(), 1);
        assert_eq!(output.carbon.files[0].path(), "src/lib.rs");
        assert_eq!(
            output.carbon.files[0]
                .old_text
                .as_ref()
                .and_then(|text| text.line_str(carbon::LineId(0))),
            Some("    old();")
        );
        assert_eq!(
            output.carbon.files[0]
                .new_text
                .as_ref()
                .and_then(|text| text.line_str(carbon::LineId(0))),
            Some("    new();")
        );
        let removed = &file.hunks[0].lines[0];
        let added = &file.hunks[0].lines[1];
        assert_eq!(output.text_buffer.view(removed.text_range), "    old();");
        assert_eq!(output.text_buffer.view(added.text_range), "    new();");
    }
}
