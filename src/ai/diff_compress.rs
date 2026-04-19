//! Compresses a unified-diff patch to stay under a byte budget before it's
//! handed to the model.
//!
//! Strategy: if the raw patch already fits, return it. Otherwise first shorten
//! any absurdly long lines (binary blobs, minified files) so they don't
//! dominate, then drop whole hunks starting from the file with the most
//! hunks remaining until the total byte count is under budget.

const LONG_LINE_LIMIT: usize = 256;
const LINE_TRUNCATION_MARKER: &str = "...[truncated]";

/// One file's worth of unified diff, with a cursor pointing at how many of
/// its hunks we're still willing to send.
struct FilePatch {
    header: String,
    hunks: Vec<String>,
    hunks_to_keep: usize,
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
}
