//! Compresses a unified-diff patch to stay under a byte budget before it's
//! handed to the model.
//!
//! Strategy: if the raw patch already fits, return it. Otherwise first shorten
//! any absurdly long lines (binary blobs, minified files) so they don't
//! dominate, then drop whole hunks starting from the file with the most
//! hunks remaining until the total byte count is under budget.

const LONG_LINE_LIMIT: usize = 256;
const LINE_TRUNCATION_MARKER: &str = "...[truncated]";
const MAX_SUMMARY_BYTES: usize = 6_000;
const MAX_SUMMARY_FILES: usize = 80;
const MIN_PATCH_BYTES: usize = 4_000;

/// One file's worth of unified diff, with a cursor pointing at how many of
/// its hunks we're still willing to send.
struct FilePatch {
    header: String,
    hunks: Vec<String>,
    hunks_to_keep: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSummary {
    status: &'static str,
    path: String,
    old_path: Option<String>,
}

impl FilePatch {
    fn parse(patch_str: &str) -> Option<Self> {
        let lines: Vec<&str> = patch_str.lines().collect();
        if lines.len() < 2 {
            return None;
        }
        let header = format!("{}\n{}\n", lines[0], lines[1]);

        let mut hunks = Vec::new();
        let mut current = String::new();
        for line in &lines[2..] {
            if line.starts_with("@@") {
                if !current.is_empty() {
                    hunks.push(std::mem::take(&mut current));
                }
                current.push_str(line);
                current.push('\n');
            } else if !current.is_empty() {
                current.push_str(line);
                current.push('\n');
            }
        }
        if !current.is_empty() {
            hunks.push(current);
        }
        if hunks.is_empty() {
            return None;
        }

        let hunks_to_keep = hunks.len();
        Some(FilePatch {
            header,
            hunks,
            hunks_to_keep,
        })
    }

    fn byte_size(&self) -> usize {
        self.header.len()
            + self.hunks[..self.hunks_to_keep]
                .iter()
                .map(String::len)
                .sum::<usize>()
    }

    fn format_patch(&self) -> String {
        let mut out = self.header.clone();
        for hunk in &self.hunks[..self.hunks_to_keep] {
            out.push_str(hunk);
        }
        let skipped = self.hunks.len() - self.hunks_to_keep;
        if skipped > 0 {
            out.push_str(&format!("[...skipped {skipped} hunks...]\n"));
        }
        out
    }
}

fn split_by_file(patch: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();

    for line in patch.lines() {
        if line.starts_with("---") && !current.is_empty() {
            result.push(current.trim_end_matches('\n').into());
            current = String::new();
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.is_empty() {
        result.push(current.trim_end_matches('\n').into());
    }
    result
}

fn drop_hunks_until(patch: &str, max_bytes: usize) -> String {
    if patch.len() <= max_bytes {
        return patch.to_string();
    }

    let mut files: Vec<FilePatch> = split_by_file(patch)
        .iter()
        .filter_map(|chunk| FilePatch::parse(chunk))
        .collect();
    if files.is_empty() {
        return patch.to_string();
    }

    let mut total: usize = files.iter().map(FilePatch::byte_size).sum();
    while total > max_bytes {
        let Some(idx) = files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.hunks_to_keep > 1)
            .max_by_key(|(_, f)| f.hunks_to_keep)
            .map(|(i, _)| i)
        else {
            break;
        };
        let file = &mut files[idx];
        let before = file.byte_size();
        file.hunks_to_keep -= 1;
        total = total.saturating_sub(before.saturating_sub(file.byte_size()));
    }

    files
        .iter()
        .map(FilePatch::format_patch)
        .collect::<Vec<_>>()
        .join("\n")
}

fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn shorten_long_lines(patch: &str) -> String {
    let mut out = String::with_capacity(patch.len());
    for line in patch.lines() {
        if line.len() > LONG_LINE_LIMIT {
            let cut = floor_char_boundary(line, LONG_LINE_LIMIT);
            out.push_str(&line[..cut]);
            out.push_str(LINE_TRUNCATION_MARKER);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

pub fn compress_commit_diff(diff_text: &str, max_bytes: usize) -> String {
    if diff_text.len() <= max_bytes {
        return diff_text.to_string();
    }

    let shortened = shorten_long_lines(diff_text);
    if shortened.len() <= max_bytes {
        return shortened;
    }

    drop_hunks_until(&shortened, max_bytes)
}

pub fn build_commit_diff_payload(diff_text: &str, max_bytes: usize) -> String {
    let summary = summarize_commit_diff(diff_text, MAX_SUMMARY_BYTES);
    let overhead = "\n\nPatch excerpt:\n".len();
    let patch_budget = max_bytes
        .saturating_sub(summary.len().saturating_add(overhead))
        .max(MIN_PATCH_BYTES.min(max_bytes));
    let patch = compress_commit_diff(diff_text, patch_budget);
    format!("{summary}\n\nPatch excerpt:\n{patch}")
}

fn summarize_commit_diff(diff_text: &str, max_bytes: usize) -> String {
    let files = parse_file_summaries(diff_text);
    let mut out = String::new();
    out.push_str("Repository diff summary:\n");
    if files.is_empty() {
        out.push_str("- No file-level summary could be parsed from the patch.\n");
        return out;
    }

    out.push_str(&format!("- {} files changed\n", files.len()));
    let counts = status_counts(&files);
    out.push_str(&format!(
        "- Status counts: added {}, modified {}, deleted {}, renamed {}, copied {}, type changed {}, other {}\n",
        counts.added,
        counts.modified,
        counts.deleted,
        counts.renamed,
        counts.copied,
        counts.type_changed,
        counts.other
    ));
    let areas = top_level_areas(&files);
    if !areas.is_empty() {
        out.push_str("- Top-level areas:");
        for (area, count) in areas.into_iter().take(12) {
            out.push_str(&format!(" {area}({count})"));
        }
        out.push('\n');
    }
    out.push_str("- Files:\n");
    for file in files.iter().take(MAX_SUMMARY_FILES) {
        if let Some(old_path) = &file.old_path {
            out.push_str(&format!(
                "  {} {} -> {}\n",
                file.status, old_path, file.path
            ));
        } else {
            out.push_str(&format!("  {} {}\n", file.status, file.path));
        }
        if out.len() >= max_bytes {
            truncate_summary(&mut out, max_bytes);
            return out;
        }
    }
    let omitted = files.len().saturating_sub(MAX_SUMMARY_FILES);
    if omitted > 0 {
        out.push_str(&format!(
            "  ... {omitted} more files omitted from summary\n"
        ));
    }
    if out.len() > max_bytes {
        truncate_summary(&mut out, max_bytes);
    }
    out
}

#[derive(Default)]
struct StatusCounts {
    added: usize,
    modified: usize,
    deleted: usize,
    renamed: usize,
    copied: usize,
    type_changed: usize,
    other: usize,
}

fn status_counts(files: &[FileSummary]) -> StatusCounts {
    let mut counts = StatusCounts::default();
    for file in files {
        match file.status {
            "A" => counts.added += 1,
            "M" => counts.modified += 1,
            "D" => counts.deleted += 1,
            "R" => counts.renamed += 1,
            "C" => counts.copied += 1,
            "T" => counts.type_changed += 1,
            _ => counts.other += 1,
        }
    }
    counts
}

fn top_level_areas(files: &[FileSummary]) -> Vec<(String, usize)> {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for file in files {
        let area = file.path.split('/').next().unwrap_or(file.path.as_str());
        *counts.entry(area.to_owned()).or_default() += 1;
    }
    let mut areas = counts.into_iter().collect::<Vec<_>>();
    areas.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    areas
}

fn parse_file_summaries(diff_text: &str) -> Vec<FileSummary> {
    let mut files = Vec::new();
    let mut current: Option<FileSummary> = None;
    for line in diff_text.lines() {
        if let Some((old_path, path)) = parse_diff_git_line(line) {
            if let Some(file) = current.take() {
                files.push(file);
            }
            current = Some(FileSummary {
                status: "M",
                path,
                old_path: (old_path != "/dev/null").then_some(old_path),
            });
            continue;
        }

        let Some(file) = current.as_mut() else {
            continue;
        };
        if line.starts_with("new file mode ") {
            file.status = "A";
            file.old_path = None;
        } else if line.starts_with("deleted file mode ") {
            file.status = "D";
        } else if line.starts_with("similarity index ") {
            file.status = "R";
        } else if line.starts_with("copy from ") {
            file.status = "C";
            file.old_path = Some(line.trim_start_matches("copy from ").to_owned());
        } else if line.starts_with("copy to ") {
            file.status = "C";
            file.path = line.trim_start_matches("copy to ").to_owned();
        } else if line.starts_with("rename from ") {
            file.status = "R";
            file.old_path = Some(line.trim_start_matches("rename from ").to_owned());
        } else if line.starts_with("rename to ") {
            file.status = "R";
            file.path = line.trim_start_matches("rename to ").to_owned();
        } else if line.starts_with("old mode ") || line.starts_with("new mode ") {
            if file.status == "M" {
                file.status = "T";
            }
        }
    }
    if let Some(file) = current {
        files.push(file);
    }
    files
}

fn parse_diff_git_line(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("diff --git ")?;
    let (old, new) = rest.split_once(" b/")?;
    let old = old.strip_prefix("a/").unwrap_or(old).to_owned();
    Some((old, new.to_owned()))
}

fn truncate_summary(summary: &mut String, max_bytes: usize) {
    let marker = "\n  ... summary truncated ...\n";
    let limit = max_bytes.saturating_sub(marker.len());
    let cut = floor_char_boundary(summary, limit);
    summary.truncate(cut);
    summary.push_str(marker);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_when_small_enough() {
        let diff = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(compress_commit_diff(diff, 1024), diff);
    }

    #[test]
    fn shrinks_long_lines() {
        let long_line: String = "a".repeat(400);
        let diff = format!("--- a/x\n+++ b/x\n@@ -1 +1 @@\n+{long_line}\n");
        let compressed = compress_commit_diff(&diff, 300);
        assert!(compressed.len() < diff.len());
        assert!(compressed.contains(LINE_TRUNCATION_MARKER));
    }

    #[test]
    fn drops_hunks_from_largest_file() {
        let hunk = "@@ -1,3 +1,3 @@\n-x\n+y\n z\n";
        let big_file = format!("--- a/big\n+++ b/big\n{hunk}{hunk}{hunk}{hunk}{hunk}");
        let small_file = format!("--- a/small\n+++ b/small\n{hunk}");
        let diff = format!("{big_file}\n{small_file}");
        let compressed = compress_commit_diff(&diff, big_file.len() / 2);
        assert!(compressed.contains("[...skipped"));
    }

    #[test]
    fn payload_keeps_summary_for_files_outside_patch_excerpt() {
        let mut diff = String::new();
        for index in 0..20 {
            diff.push_str(&format!(
                "diff --git a/src/module{index}.rs b/src/module{index}.rs\n--- a/src/module{index}.rs\n+++ b/src/module{index}.rs\n@@ -1 +1 @@\n-old{index}\n+new{index}\n"
            ));
        }

        let payload = build_commit_diff_payload(&diff, 900);
        assert!(payload.contains("Repository diff summary:"));
        assert!(payload.contains("- 20 files changed"));
        assert!(payload.contains("M src/module19.rs"));
        assert!(payload.contains("Patch excerpt:"));
    }

    #[test]
    fn summary_reports_renames() {
        let diff = "diff --git a/src/old.rs b/src/new.rs\nsimilarity index 100%\nrename from src/old.rs\nrename to src/new.rs\n";
        let payload = build_commit_diff_payload(diff, 1024);
        assert!(payload.contains("renamed 1"));
        assert!(payload.contains("R src/old.rs -> src/new.rs"));
    }
}
