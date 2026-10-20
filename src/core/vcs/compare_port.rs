use std::collections::HashMap;
use std::sync::Arc;

use diffy_diff::{DiffStats, FileStatus};

use crate::core::compare::service::SEMANTIC_UNAVAILABLE;
use crate::core::compare::{CompareEntry, CompareOutput};
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::model::FileChangeStatus;
use crate::core::vcs::patch::parse_exact_patch;

pub fn output_from_patch(raw_diff: &[u8]) -> Result<CompareOutput> {
    CompareOutput::from_patch_bytes(raw_diff)
}

pub fn with_semantic_fallback(mut output: CompareOutput) -> CompareOutput {
    output.used_fallback = true;
    output.fallback_message = SEMANTIC_UNAVAILABLE.to_owned();
    output
}

pub fn file_status(status: FileChangeStatus) -> FileStatus {
    match status {
        FileChangeStatus::Added | FileChangeStatus::Untracked => FileStatus::Added,
        FileChangeStatus::Deleted => FileStatus::Deleted,
        FileChangeStatus::Renamed => FileStatus::Renamed,
        FileChangeStatus::Copied => FileStatus::Copied,
        FileChangeStatus::Modified
        | FileChangeStatus::TypeChanged
        | FileChangeStatus::Binary
        | FileChangeStatus::Conflicted => FileStatus::Modified,
    }
}

pub fn deferred_entry(
    old_path: Option<&str>,
    path: &str,
    status: FileChangeStatus,
) -> CompareEntry {
    let (old_path, new_path) = match status {
        FileChangeStatus::Added | FileChangeStatus::Untracked => (None, Some(path)),
        FileChangeStatus::Deleted => (Some(old_path.unwrap_or(path)), None),
        _ => (Some(old_path.unwrap_or(path)), Some(path)),
    };
    CompareEntry::deferred(old_path, new_path, file_status(status))
        .with_binary(status == FileChangeStatus::Binary)
}

pub fn entry_stats(entry: &CompareEntry) -> (i32, i32) {
    match entry.stats {
        DiffStats::Known {
            additions,
            deletions,
        } => (saturating(additions), saturating(deletions)),
        DiffStats::Pending => (0, 0),
    }
}

pub fn patch_stats(raw_diff: &[u8]) -> Result<HashMap<String, (i32, i32)>> {
    let bytes: Arc<[u8]> = raw_diff.into();
    let files = parse_exact_patch(&bytes).map_err(|error| DiffyError::Parse(error.to_string()))?;
    Ok(files
        .iter()
        .filter_map(|file| {
            let path = String::from_utf8_lossy(file.path()?).into_owned();
            Some((
                path,
                (saturating(file.additions()), saturating(file.deletions())),
            ))
        })
        .collect())
}

fn saturating(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_entries_keep_change_paths() {
        let added = deferred_entry(None, "a", FileChangeStatus::Added);
        assert_eq!(added.status(), FileStatus::Added);
        assert_eq!(added.path(), "a");
        let renamed = deferred_entry(Some("old"), "new", FileChangeStatus::Renamed);
        assert_eq!(renamed.paths.old_path().as_deref(), Some("old"));
        assert_eq!(renamed.paths.new_path().as_deref(), Some("new"));
        assert!(renamed.stats.is_pending());
    }

    #[test]
    fn patch_stats_count_each_file() {
        let stats = patch_stats(b"--- a/f\n+++ b/f\n@@ -1 +1,2 @@\n-a\n+b\n+c\n").unwrap();
        assert_eq!(stats.get("f"), Some(&(2, 1)));
    }
}
