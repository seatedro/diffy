mod git_diff;

/// Keep rename detection bounded on monorepo-scale diffs. This mirrors Git's
/// `diff.renameLimit` behavior closely enough for UI use: preserve rename
/// detection for ordinary diffs, but skip exhaustive similarity scans when a
/// range touches thousands of files.
pub(crate) const RENAME_DETECTION_LIMIT: usize = 1000;

pub use git_diff::GitDiffBackend;
