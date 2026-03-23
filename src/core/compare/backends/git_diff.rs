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
        let left = git.resolve_commit_oid(&spec.left_ref)?;
        let right = git.resolve_commit_oid(&spec.right_ref)?;

        let left_commit = repo.find_commit(left)?;
        let right_commit = repo.find_commit(right)?;
        let left_tree = left_commit.tree()?;
        let right_tree = right_commit.tree()?;

        let mut options = DiffOptions::new();
        options.context_lines(3);
        let mut diff = repo.diff_tree_to_tree(Some(&left_tree), Some(&right_tree), Some(&mut options))?;
        diff.find_similar(None)?;

        let mut output = CompareOutput::default();
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        let deltas: Vec<_> = diff.deltas().collect();
        for (delta_idx, delta) in deltas.iter().enumerate() {
            let mut file = FileDiff {
                path: delta.new_file().path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
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
                        let content = std::str::from_utf8(line.content()).unwrap_or_default().trim_end_matches('\n');
                        let text_range = text_buffer.append(content);

                        let origin = line.origin();
                        let (kind, old_num, new_num, tokens) = if origin == '-' {
                            let removed = vec![DiffTokenSpan { offset: 0, length: content.len() as u32, kind: SyntaxTokenKind::Normal }];
                            let range = token_buffer.append(&removed);
                            let old = old_line;
                            old_line += 1;
                            (LineKind::Removed, Some(old), None, range)
                        } else if origin == '+' {
                            let added = vec![DiffTokenSpan { offset: 0, length: content.len() as u32, kind: SyntaxTokenKind::Normal }];
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
                            current_hunk.lines.iter().filter(|l| l.kind != LineKind::Added).count(),
                            current_hunk.new_start,
                            current_hunk.lines.iter().filter(|l| l.kind != LineKind::Removed).count()
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
