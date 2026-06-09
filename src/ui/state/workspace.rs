use crate::actions::WorkspaceAction;
use crate::effects::{Effect, RepositoryEffect};
use crate::events::RepositorySyncReason;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: WorkspaceAction) -> Vec<Effect> {
    state.apply_workspace_action(action)
}

impl AppState {
    pub(super) fn apply_workspace_action(&mut self, action: WorkspaceAction) -> Vec<Effect> {
        match action {
            WorkspaceAction::OpenRepository(path) => self.open_repository(path),
            WorkspaceAction::NewTextCompare => self.new_text_compare(),
            WorkspaceAction::ShowWorkingTree => self.show_working_tree(),
            WorkspaceAction::RefreshRepository => self.refresh_repository(),
        }
    }

    fn refresh_repository(&mut self) -> Vec<Effect> {
        let Some(path) = self.compare.repo_path.get(&self.store) else {
            self.push_error("Open a repository before refreshing.");
            return Vec::new();
        };
        if self.workspace.source.get(&self.store) == WorkspaceSource::Compare {
            self.kickoff_compare()
        } else {
            vec![
                RepositoryEffect::SyncRepository {
                    path,
                    reason: RepositorySyncReason::Rescan,
                    reporter_generation: None,
                }
                .into(),
            ]
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceMode {
    #[default]
    Empty,
    Loading,
    Ready,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceSource {
    #[default]
    None,
    Status,
    Compare,
    TextCompare,
}

#[derive(Debug, Clone, Default, Store)]
pub struct WorkspaceState {
    pub mode: WorkspaceMode,
    /// Arc-wrapped so per-frame UI snapshots clone a pointer, not the
    /// label strings inside.
    pub compare_progress: Option<Arc<CompareProgress>>,
    pub source: WorkspaceSource,
    pub status: AsyncStatus,
    pub status_operation_pending: bool,
    /// Shared generation counter for repo *and* text compares. Must only move
    /// forward within a session: `CompareScheduler` keeps a monotonic epoch
    /// high-water mark and silently drops jobs stamped below it, so any path
    /// that writes this signal must bump from the current value (or take a
    /// max), never assign an independent counter.
    pub compare_generation: u64,
    pub status_generation: u64,
    pub files: Vec<FileListEntry>,
    pub status_file_changes: Vec<FileChange>,
    pub selected_file_index: Option<usize>,
    pub selected_file_path: Option<String>,
    pub selected_change_bucket: Option<ChangeBucket>,
    pub compare_output: Option<CompareOutput>,
    pub compare_total_stats: Option<(i32, i32)>,
    pub compare_hydrated_stats: Option<(i32, i32)>,
    pub compare_deferred_stats_remaining: Option<usize>,
    pub compare_deferred_stats_cursor: usize,
    pub compare_total_stats_loading: bool,
    pub compare_stats_hydration: CompareStatsHydrationState,
    pub active_file: Option<ActiveFile>,
    pub active_file_loading: Option<ActiveFileLoading>,
    pub file_cache: HashMap<usize, ActiveFile>,
    pub file_cache_loading: HashMap<usize, ActiveFileLoading>,
    pub raw_diff_len: usize,
    pub used_fallback: bool,
    pub fallback_message: String,
    pub sidebar_auto_width: Option<SidebarWidthCache>,
    pub range_commits: Vec<VcsChange>,
    pub compare_history_pending: Option<CompareHistoryRequest>,
    pub pre_drill_compare: Option<(String, String, CompareMode)>,
    pub expansions: HashMap<String, carbon::ExpansionState>,
    pub file_content_heights: Vec<Option<u32>>,
    pub file_scroll_total_height_px: u32,
    pub pending_file_content_heights: HashMap<usize, u32>,
    pub file_scroll_recompute_pending: bool,
    pub global_scroll_top_px: u32,
    pub measured_px_per_row_q16: u32,
    pub viewport_scrollbar_drag: Option<ViewportScrollbarDragState>,
}

pub fn workspace_mode_name(mode: WorkspaceMode) -> &'static str {
    match mode {
        WorkspaceMode::Empty => "empty",
        WorkspaceMode::Loading => "loading",
        WorkspaceMode::Ready => "ready",
    }
}

impl AppState {
    /// Returns true when the workspace is in `Ready` mode.
    pub fn is_workspace_ready(&self) -> bool {
        self.workspace.mode.get(&self.store) == WorkspaceMode::Ready
    }
}
