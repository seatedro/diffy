use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{Delta, DiffOptions, ObjectType, Repository};
use serde::Deserialize;

use crate::core::compare::backends::DiffBackend;
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::{CompareMode, CompareSpec};
use crate::core::diff::{DiffLine, FileDiff, Hunk, LineKind};
use crate::core::error::{DiffyError, Result};
use crate::core::text::{DiffTokenSpan, SyntaxTokenKind, TextBuffer, TokenBuffer};
use crate::core::vcs::git::GitService;

#[derive(Debug, Default, Clone, Copy)]
pub struct DifftasticBackend;

impl DifftasticBackend {
    pub fn is_available() -> bool {
        Command::new("sh")
            .arg("-lc")
            .arg("command -v difft >/dev/null 2>&1")
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

impl DiffBackend for DifftasticBackend {
    fn compare(&self, spec: &CompareSpec, git: &GitService) -> Result<Option<CompareOutput>> {
        if !Self::is_available() {
            return Ok(None);
        }

        let (left, right) = match spec.mode {
            CompareMode::TwoDot => (
                git.resolve_ref(&spec.left_ref)?,
                git.resolve_ref(&spec.right_ref)?,
            ),
            CompareMode::ThreeDot => {
                git.resolve_comparison(&spec.left_ref, &spec.right_ref, CompareMode::ThreeDot)?
            }
            CompareMode::SingleCommit => {
                git.resolve_comparison(&spec.left_ref, &spec.right_ref, CompareMode::SingleCommit)?
            }
        };

        let repo = Repository::open(git.repo_path())?;
        let changed_paths = collect_changed_paths(&repo, &left, &right)?;
        let temp_root = std::env::temp_dir().join(format!(
            "diffy-difftastic-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_root)?;

        let mut output = CompareOutput::default();
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        for (index, changed) in changed_paths.into_iter().enumerate() {
            if changed.is_binary {
                output.files.push(FileDiff {
                    path: changed
                        .new_path
                        .clone()
                        .or(changed.old_path.clone())
                        .unwrap_or_default(),
                    status: changed.status,
                    is_binary: true,
                    ..FileDiff::default()
                });
                continue;
            }

            let old_temp = temp_root.join(format!("old_{index}.txt"));
            let new_temp = temp_root.join(format!("new_{index}.txt"));
            fs::write(&old_temp, &changed.old_content)?;
            fs::write(&new_temp, &changed.new_content)?;

            let process = Command::new("difft")
                .args([
                    "--display",
                    "json",
                    old_temp.to_str().unwrap(),
                    new_temp.to_str().unwrap(),
                ])
                .env("DFT_UNSTABLE", "yes")
                .output()?;
            if !process.status.success() {
                return Err(DiffyError::General(format!(
                    "difftastic failed: {}",
                    String::from_utf8_lossy(&process.stderr).trim()
                )));
            }

            let file = parse_difftastic_json(
                &String::from_utf8_lossy(&process.stdout),
                changed
                    .new_path
                    .as_deref()
                    .or(changed.old_path.as_deref())
                    .unwrap_or_default(),
                &changed.status,
                &mut text_buffer,
                &mut token_buffer,
            )?;
            output.files.push(file);
        }

        let _ = fs::remove_dir_all(&temp_root);
        output.text_buffer = text_buffer;
        output.token_buffer = token_buffer;
        Ok(Some(output))
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
        .revparse_single(right)?
        .peel(ObjectType::Commit)?
        .peel_to_tree()?;
    let mut options = DiffOptions::new();
    options.context_lines(3);
    let mut diff =
        repo.diff_tree_to_tree(Some(&left_tree), Some(&right_tree), Some(&mut options))?;
    diff.find_similar(None)?;

    let mut changed = Vec::new();
    for delta in diff.deltas() {
        let old_content = load_blob_content(repo, delta.old_file().id())?;
        let new_content = load_blob_content(repo, delta.new_file().id())?;
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

fn load_blob_content(repo: &Repository, oid: git2::Oid) -> Result<Option<Vec<u8>>> {
    if oid.is_zero() {
        return Ok(None);
    }
    Ok(Some(repo.find_blob(oid)?.content().to_vec()))
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RootJson {
    File(FileJson),
    Files(Vec<FileJson>),
}

#[derive(Debug, Deserialize)]
struct FileJson {
    #[serde(default)]
    path: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    chunks: Vec<Vec<LineJson>>,
}

#[derive(Debug, Deserialize)]
struct LineJson {
    #[serde(default)]
    lhs: Option<SideJson>,
    #[serde(default)]
    rhs: Option<SideJson>,
}

#[derive(Debug, Deserialize)]
struct SideJson {
    #[serde(default)]
    line_number: Option<i32>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    changes: Vec<ChangeJson>,
}

#[derive(Debug, Deserialize)]
struct ChangeJson {
    #[serde(default)]
    content: String,
}

fn parse_difftastic_json(
    json: &str,
    fallback_path: &str,
    fallback_status: &str,
    text_buffer: &mut TextBuffer,
    token_buffer: &mut TokenBuffer,
) -> Result<FileDiff> {
    let root: RootJson = serde_json::from_str(json)?;
    let file_json = match root {
        RootJson::File(file) => file,
        RootJson::Files(mut files) => files.drain(..).next().ok_or_else(|| {
            DiffyError::Parse("difftastic JSON payload did not include a file object".to_owned())
        })?,
    };

    let mut file = FileDiff {
        path: if file_json.path.is_empty() {
            fallback_path.to_owned()
        } else {
            file_json.path
        },
        status: match file_json.status.as_str() {
            "created" => "A".to_owned(),
            "deleted" => "D".to_owned(),
            "unchanged" => "U".to_owned(),
            _ => fallback_status.to_owned(),
        },
        is_binary: file_json.language == "binary",
        ..FileDiff::default()
    };

    if file.is_binary {
        return Ok(file);
    }

    for chunk in file_json.chunks {
        let mut hunk = Hunk { ..Hunk::default() };
        let mut old_start = None;
        let mut new_start = None;
        let mut old_count = 0_i32;
        let mut new_count = 0_i32;
        for line in chunk {
            let lhs = line.lhs.as_ref().map(|side| side_parts(side)).transpose()?;
            let rhs = line.rhs.as_ref().map(|side| side_parts(side)).transpose()?;
            if let (Some((lhs_text, _, lhs_line)), Some((rhs_text, _, rhs_line))) = (&lhs, &rhs) {
                if lhs_text == rhs_text {
                    if let Some(line_number) = lhs_line {
                        old_start.get_or_insert(*line_number);
                        old_count += 1;
                    }
                    if let Some(line_number) = rhs_line {
                        new_start.get_or_insert(*line_number);
                        new_count += 1;
                    }
                    let range = text_buffer.append(lhs_text);
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Context,
                        old_line_number: *lhs_line,
                        new_line_number: *rhs_line,
                        text_range: range,
                        ..DiffLine::default()
                    });
                    continue;
                }
            }
            if let Some((text, tokens, line_number)) = lhs {
                if let Some(number) = line_number {
                    old_start.get_or_insert(number);
                    old_count += 1;
                }
                let text_range = text_buffer.append(&text);
                let change_tokens = token_buffer.append(&tokens);
                hunk.lines.push(DiffLine {
                    kind: LineKind::Removed,
                    old_line_number: line_number,
                    new_line_number: None,
                    text_range,
                    change_tokens,
                    ..DiffLine::default()
                });
                file.deletions += 1;
            }
            if let Some((text, tokens, line_number)) = rhs {
                if let Some(number) = line_number {
                    new_start.get_or_insert(number);
                    new_count += 1;
                }
                let text_range = text_buffer.append(&text);
                let change_tokens = token_buffer.append(&tokens);
                hunk.lines.push(DiffLine {
                    kind: LineKind::Added,
                    old_line_number: None,
                    new_line_number: line_number,
                    text_range,
                    change_tokens,
                    ..DiffLine::default()
                });
                file.additions += 1;
            }
        }
        if !hunk.lines.is_empty() {
            let computed_old_start = match old_start {
                Some(start) => start,
                None => new_start.map_or(0, |start| start.saturating_sub(1)),
            };
            let computed_new_start = match new_start {
                Some(start) => start,
                None => old_start.map_or(0, |start| start.saturating_sub(1)),
            };
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

    Ok(file)
}

fn side_parts(side: &SideJson) -> Result<(String, Vec<DiffTokenSpan>, Option<i32>)> {
    let mut text = String::new();
    let mut tokens = Vec::new();
    if !side.changes.is_empty() {
        for change in &side.changes {
            if change.content.is_empty() {
                continue;
            }
            let start = text.len() as u32;
            text.push_str(&change.content);
            tokens.push(DiffTokenSpan {
                offset: start,
                length: change.content.len() as u32,
                kind: SyntaxTokenKind::Normal,
            });
        }
    }
    if text.is_empty() {
        text = side.text.clone();
    }
    Ok((text, tokens, side.line_number.map(|line| line + 1)))
}

#[cfg(test)]
mod tests {
    use super::parse_difftastic_json;
    use crate::core::text::{TextBuffer, TokenBuffer};

    #[test]
    fn parse_difftastic_json_builds_real_hunk_headers_for_modified_chunks() {
        let json = r#"{
            "chunks":[[
                {
                    "lhs":{"line_number":0,"changes":[{"content":"old"}]},
                    "rhs":{"line_number":0,"changes":[{"content":"new"}]}
                }
            ]],
            "language":"Rust",
            "path":"src/lib.rs",
            "status":"changed"
        }"#;
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        let file =
            parse_difftastic_json(json, "src/lib.rs", "M", &mut text_buffer, &mut token_buffer)
                .unwrap();

        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].old_start, 1);
        assert_eq!(file.hunks[0].old_count, 1);
        assert_eq!(file.hunks[0].new_start, 1);
        assert_eq!(file.hunks[0].new_count, 1);
        assert_eq!(file.hunks[0].header, "@@ -1,1 +1,1 @@");
    }

    #[test]
    fn parse_difftastic_json_handles_pure_insert_headers() {
        let json = r#"{
            "chunks":[[
                {
                    "rhs":{"line_number":0,"changes":[{"content":"inserted"}]}
                }
            ]],
            "language":"Rust",
            "path":"src/lib.rs",
            "status":"changed"
        }"#;
        let mut text_buffer = TextBuffer::default();
        let mut token_buffer = TokenBuffer::default();

        let file =
            parse_difftastic_json(json, "src/lib.rs", "M", &mut text_buffer, &mut token_buffer)
                .unwrap();

        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].old_start, 0);
        assert_eq!(file.hunks[0].old_count, 0);
        assert_eq!(file.hunks[0].new_start, 1);
        assert_eq!(file.hunks[0].new_count, 1);
        assert_eq!(file.hunks[0].header, "@@ -0,0 +1,1 @@");
    }
}
