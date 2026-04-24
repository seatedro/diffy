//! Phase model for the compare pipeline + a minimal sink trait that lets
//! the backends surface progress without taking a hard dependency on
//! apprt. `apprt::ProgressReporter` implements this trait.
//!
//! Phases are emitted at real service boundaries only — never synthesized
//! from a timer. If a phase is reported, that work is actually starting.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComparePhase {
    /// libgit2 is opening the repository on disk.
    #[default]
    OpeningRepo,
    /// Resolving human-readable refs (branches/tags/sha prefixes) to full OIDs.
    ResolvingRefs,
    /// Walking git trees and detecting renames — the bulk of time for
    /// large diffs lives here. Emitted before `diff_tree_to_tree` +
    /// `find_similar`.
    EnumeratingChanges,
    /// Reading and semantically diffing individual changed files. Carries
    /// running counts so the UI can render a determinate bar.
    LoadingFiles { files_seen: u32, files_total: u32 },
    /// Fetching the commit range that drives the sidebar commit list.
    FetchingHistory,
    /// State layer is installing the file list + metadata on the UI side.
    PopulatingList,
    /// Preparing the first selected file for display (layout, syntax,
    /// token buffers). Final phase before the real diff UI takes over.
    RenderingFirstFile,
}

impl ComparePhase {
    pub fn label(self) -> String {
        match self {
            Self::OpeningRepo => "Opening repository\u{2026}".to_owned(),
            Self::ResolvingRefs => "Resolving refs\u{2026}".to_owned(),
            Self::EnumeratingChanges => "Enumerating changes\u{2026}".to_owned(),
            Self::LoadingFiles {
                files_seen,
                files_total,
            } => {
                if files_total == 0 {
                    "Reading files\u{2026}".to_owned()
                } else {
                    format!("Reading {files_seen} of {files_total} files\u{2026}")
                }
            }
            Self::FetchingHistory => "Fetching commit history\u{2026}".to_owned(),
            Self::PopulatingList => "Populating file list\u{2026}".to_owned(),
            Self::RenderingFirstFile => "Preparing first file\u{2026}".to_owned(),
        }
    }
}

/// A cheap, `Send + Sync` channel the backends use to publish phase
/// transitions. Implemented by `apprt::ProgressReporter`. Kept as an
/// object-safe trait so backends can take `Option<&dyn ProgressSink>`
/// without adding a generic parameter.
pub trait ProgressSink: Send + Sync {
    fn phase(&self, phase: ComparePhase);
}
