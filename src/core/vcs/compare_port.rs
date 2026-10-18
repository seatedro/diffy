use crate::core::compare::backends::compare_output_from_raw_patch;
use crate::core::compare::{CompareFileSummary, CompareOutput};
use crate::core::error::Result;
use crate::core::vcs::model::FileChangeStatus;

pub type SummaryStatus = carbon::FileStatus;

pub fn output_from_patch(raw_diff: &[u8]) -> Result<CompareOutput> {
    compare_output_from_raw_patch(&String::from_utf8_lossy(raw_diff))
}

pub fn summary_from_change(
    old_path: Option<&str>,
    path: &str,
    status: FileChangeStatus,
    stats_deferred: bool,
) -> CompareFileSummary {
    CompareFileSummary::from_paths_status(
        old_path,
        Some(path),
        summary_status(status),
        stats_deferred,
    )
}

pub fn summary_status(status: FileChangeStatus) -> SummaryStatus {
    match status {
        FileChangeStatus::Added | FileChangeStatus::Untracked => SummaryStatus::Added,
        FileChangeStatus::Deleted => SummaryStatus::Deleted,
        FileChangeStatus::Renamed => SummaryStatus::Renamed,
        FileChangeStatus::TypeChanged => SummaryStatus::ModeChanged,
        FileChangeStatus::Binary => SummaryStatus::Binary,
        FileChangeStatus::Copied | FileChangeStatus::Modified | FileChangeStatus::Conflicted => {
            SummaryStatus::Modified
        }
    }
}
