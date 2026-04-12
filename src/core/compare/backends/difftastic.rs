use std::collections::HashMap;
use std::fs;
use std::path::Path;

use git2::{Delta, DiffOptions, ObjectType, Repository};
use vendored_difftastic::{
    ChangeIntensity as DftIntensity, DiffRequest, DiffStatus, HighlightKind, SemanticDiffResult,
};

use crate::core::compare::backends::DiffBackend;
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::{CompareMode, CompareSpec};
use crate::core::diff::{DiffLine, FileDiff, Hunk, LineKind};
use crate::core::error::{DiffyError, Result};
use crate::core::text::{ChangeIntensity, DiffTokenSpan, SyntaxTokenKind, TextBuffer, TokenBuffer};
use crate::core::vcs::git::{GitService, StatusItem, StatusScope, WORKDIR_REF};

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
        compare_changed_paths(vec![changed])
    }
}

impl DiffBackend for DifftasticBackend {
    fn compare(&self, spec: &CompareSpec, git: &GitService) -> Result<Option<CompareOutput>> {
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
        let changed_paths = collect_changed_paths(repo, &left, &right)?;

        Ok(Some(compare_changed_paths(changed_paths)?))
    }
}

fn compare_changed_paths(changed_paths: Vec<ChangedPath>) -> Result<CompareOutput> {
    let mut output = CompareOutput::default();
    let mut text_buffer = TextBuffer::default();
    let mut token_buffer = TokenBuffer::default();

    for changed in changed_paths {
        let display_path = changed
            .new_path
            .as_deref()
            .or(changed.old_path.as_deref())
            .unwrap_or_default();
        if changed.is_binary {
            output.files.push(FileDiff {
                path: display_path.to_owned(),
                status: changed.status,
                is_binary: true,
                ..FileDiff::default()
            });
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

fn collect_changed_paths(repo: &Repository, left: &str, right: &str) -> Result<Vec<ChangedPath>> {
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
        DifftasticBackend, collect_changed_paths, convert_semantic_result, map_highlight,
        map_intensity,
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

        let changed = collect_changed_paths(&repo, &head, WORKDIR_REF).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].new_path.as_deref(), Some("src/lib.rs"));
        assert_eq!(
            String::from_utf8_lossy(&changed[0].new_content),
            "fn answer() {\n    new();\n}\n"
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
            )
            .unwrap()
            .expect("difftastic result");

        assert_eq!(output.files.len(), 1);
        let file = &output.files[0];
        assert_eq!(file.path, "src/lib.rs");
        assert_eq!(file.hunks.len(), 1);
        let removed = &file.hunks[0].lines[0];
        let added = &file.hunks[0].lines[1];
        assert_eq!(output.text_buffer.view(removed.text_range), "    old();");
        assert_eq!(output.text_buffer.view(added.text_range), "    new();");
    }

    #[test]
    fn change_highlights_align_with_render_doc_text() {
        use super::convert_semantic_result;
        use crate::core::syntax::DiffSyntaxAnnotator;

        let old_src = "fn greet(name: &str) {\n    println!(\"hello {}\", name);\n}\n";
        let new_src = "fn greet(label: &str) {\n    println!(\"hi {}\", label);\n}\n";

        let semantic = vendored_difftastic::diff_bytes_semantic(vendored_difftastic::DiffRequest {
            display_path: "src/lib.rs",
            lhs_path: Some(Path::new("src/lib.rs")),
            rhs_path: Some(Path::new("src/lib.rs")),
            lhs_bytes: old_src.as_bytes(),
            rhs_bytes: new_src.as_bytes(),
        })
        .unwrap();

        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();
        let mut file = convert_semantic_result(
            &semantic,
            "src/lib.rs",
            "M",
            old_src,
            new_src,
            &mut text_buffer,
            &mut token_buffer,
        );

        DiffSyntaxAnnotator::new().annotate(&mut file, &mut text_buffer, &mut token_buffer);

        let doc =
            crate::ui::editor::render_doc::build_render_doc(&file, 0, &text_buffer, &token_buffer);

        use crate::ui::editor::render_doc::{RenderRowKind, STYLE_FLAG_NOVEL_WORD};

        for (i, line) in doc.lines.iter().enumerate() {
            if line.row_kind() != RenderRowKind::Modified {
                continue;
            }

            for (side, text_range, runs_range) in [
                ("left", line.left_text, line.left_runs),
                ("right", line.right_text, line.right_runs),
            ] {
                if !text_range.is_valid() {
                    continue;
                }
                let text = doc.line_text(text_range);
                let runs = doc.line_runs(runs_range);
                for run in runs {
                    if run.flags & STYLE_FLAG_NOVEL_WORD == 0 {
                        continue;
                    }
                    let start = run.byte_start as usize;
                    let end = start + run.byte_len as usize;
                    assert!(
                        end <= text.len(),
                        "line {i} {side} change run [{start}..{end}] exceeds text len {} ({:?})",
                        text.len(),
                        text
                    );
                    let highlighted = &text[start..end];
                    assert!(
                        !highlighted.is_empty(),
                        "line {i} {side} change run [{start}..{end}] maps to empty string"
                    );
                }
            }
        }
    }
}
