use crate::actions::CompareAction;
use crate::effects::Effect;
use crate::events::CompareEvent;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: CompareAction) -> Vec<Effect> {
    state.apply_compare_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: CompareEvent) -> Vec<Effect> {
    match event {
        CompareEvent::CompareHistoryReady(payload) => state.handle_compare_history_ready(payload),
        CompareEvent::TextCompareFinished(payload) => state.handle_text_compare_finished(payload),
        CompareEvent::CompareHistoryFailed {
            generation: _,
            message: _,
        } => Vec::new(),
        CompareEvent::CompareFinished(payload) => state.handle_compare_finished(payload),
        CompareEvent::CompareFailed {
            generation,
            message,
        } => {
            if generation == state.workspace.compare_generation.get(&state.store) {
                state
                    .workspace
                    .status
                    .set(&state.store, AsyncStatus::Failed);
                state.workspace.mode.set(&state.store, WorkspaceMode::Empty);
                state.workspace.compare_progress.set(&state.store, None);
                state.push_error(&message);
            }
            Vec::new()
        }
        CompareEvent::TextCompareFailed {
            generation,
            message,
        } => state.handle_text_compare_failed(generation, message),
        CompareEvent::CompareProgressUpdate { generation, phase } => {
            state.handle_compare_progress_update(generation, phase);
            Vec::new()
        }
        CompareEvent::CompareStatsReady(payload) => state.handle_compare_stats_ready(payload),
        CompareEvent::CompareStatsFailed {
            generation,
            message: _,
        } => {
            if generation == state.workspace.compare_generation.get(&state.store) {
                state
                    .workspace
                    .compare_total_stats_loading
                    .set(&state.store, false);
                if let Some(stats) = state.exact_compare_total_stats_if_ready() {
                    state
                        .workspace
                        .compare_total_stats
                        .set(&state.store, Some(stats));
                    if !state.compare_stats_hydration_running() {
                        return state
                            .take_pending_compare_history_effect()
                            .into_iter()
                            .collect();
                    }
                    return Vec::new();
                }
                if let Some(effect) = state.start_compare_stats_hydration_if_idle() {
                    let is_background_stats = matches!(
                        &effect,
                        Effect::Compare(CompareEffect::LoadFileStats(task))
                            if task.request.priority == CompareWorkPriority::Warmup
                    );
                    let mut effects = vec![effect];
                    if is_background_stats
                        && let Some(effect) = state.take_pending_compare_history_effect()
                    {
                        effects.push(effect);
                    }
                    return effects;
                }
                if !state.compare_stats_hydration_running()
                    && let Some(effect) = state.take_pending_compare_history_effect()
                {
                    return vec![effect];
                }
            }
            Vec::new()
        }
        CompareEvent::CompareFileFinished(payload) => state.handle_compare_file_finished(payload),
        CompareEvent::CompareFileStatsReady(payload) => {
            state.handle_compare_file_stats_ready(payload)
        }
        CompareEvent::CompareFileStatsFailed {
            generation,
            message: _,
        } => {
            if generation == state.workspace.compare_generation.get(&state.store) {
                state.set_compare_stats_hydration(CompareStatsHydrationState::Failed);
                if !state
                    .workspace
                    .compare_total_stats_loading
                    .get(&state.store)
                    && let Some(effect) = state.take_pending_compare_history_effect()
                {
                    return vec![effect];
                }
            }
            Vec::new()
        }
        CompareEvent::CompareFileFailed {
            generation,
            path,
            message,
        } => {
            if generation == state.workspace.compare_generation.get(&state.store) {
                let matches_loading = state
                    .workspace
                    .active_file_loading
                    .with(&state.store, |loading| {
                        loading.as_ref().is_some_and(|loading| loading.path == path)
                    });
                if matches_loading {
                    state.workspace.active_file_loading.set(&state.store, None);
                    state.workspace.compare_progress.set(&state.store, None);
                    state.push_error(&message);
                }
            }
            Vec::new()
        }
        CompareEvent::StatusDiffFinished(payload) => state.handle_status_diff_finished(payload),
        CompareEvent::StatusDiffFailed {
            generation,
            index: _,
            message,
        } => {
            if generation == state.workspace.status_generation.get(&state.store) {
                state
                    .workspace
                    .status
                    .set(&state.store, AsyncStatus::Failed);
                state.push_error(&message);
            }
            Vec::new()
        }
        CompareEvent::RefResolved {
            query,
            generation,
            short_oid,
            summary,
        } => {
            if generation
                == state
                    .overlays
                    .picker
                    .ref_resolve_generation
                    .get(&state.store)
            {
                state
                    .overlays
                    .picker
                    .entries
                    .update(&state.store, |entries| {
                        if let Some(entry) = entries
                            .iter_mut()
                            .find(|e| e.value == query && e.detail == "Resolving\u{2026}")
                        {
                            entry.detail = format!("{short_oid} \u{2014} {summary}");
                        }
                    });
            }
            Vec::new()
        }
        CompareEvent::RefResolveFailed { generation } => {
            if generation
                == state
                    .overlays
                    .picker
                    .ref_resolve_generation
                    .get(&state.store)
            {
                state
                    .overlays
                    .picker
                    .entries
                    .update(&state.store, |entries| {
                        if let Some(entry) =
                            entries.iter_mut().find(|e| e.detail == "Resolving\u{2026}")
                        {
                            entry.detail = "Use typed ref".to_owned();
                        }
                    });
            }
            Vec::new()
        }
    }
}

impl AppState {
    pub(super) fn apply_compare_action(&mut self, action: CompareAction) -> Vec<Effect> {
        use CompareAction::*;
        match action {
            SetLeftRef(value) => {
                let mut effects = self.update_compare_field(CompareField::Left, value);
                effects.extend(self.persist_settings_effect());
                effects
            }
            SetRightRef(value) => {
                let mut effects = self.update_compare_field(CompareField::Right, value);
                effects.extend(self.persist_settings_effect());
                effects
            }
            SwapRefs => self.swap_refs(),
            SetActiveRefField(field) => self.set_active_ref_field(field),
            SwapDraftRefs => self.swap_draft_refs(),
            CommitRefPicker => {
                if self.overlays_top() != Some(OverlaySurface::RefPicker) {
                    return Vec::new();
                }
                self.commit_ref_picker()
            }
            CancelRefPicker => {
                if self.overlays_top() != Some(OverlaySurface::RefPicker) {
                    return Vec::new();
                }
                self.cancel_ref_picker()
            }
            SetCompareMode(mode) => {
                let profile = self.vcs_ui_profile();
                if !profile.accepts_compare_mode(mode) {
                    if self.overlays_top() == Some(OverlaySurface::CompareMenu) {
                        self.pop_overlay();
                    }
                    return Vec::new();
                }
                self.compare.mode.set(&self.store, mode);
                if self.overlays_top() == Some(OverlaySurface::CompareMenu) {
                    self.pop_overlay();
                }
                self.persist_settings_effect()
            }
            CycleCompareMode => {
                let next = self
                    .vcs_ui_profile()
                    .next_compare_mode(self.compare.mode.get(&self.store));
                self.compare.mode.set(&self.store, next);
                self.persist_settings_effect()
            }
            OpenCompareMenu => {
                self.push_overlay(OverlaySurface::CompareMenu, None);
                Vec::new()
            }
            ApplyComparePreset(preset) => self.apply_compare_preset(&preset),
            SetLayoutMode(layout) => {
                self.compare.layout.set(&self.store, layout);
                self.editor.layout.set(&self.store, layout);
                let mut effects = self.rebuild_command_palette();
                effects.extend(self.persist_settings_effect());
                effects
            }
            SetRenderer(renderer) => {
                self.compare.renderer.set(&self.store, renderer);
                if self.workspace.source.get(&self.store) == WorkspaceSource::TextCompare {
                    self.mark_text_compare_dirty();
                }
                self.persist_settings_effect()
            }
            StartCompare => {
                self.github.pull_request.active.set(&self.store, None);
                self.github
                    .pull_request
                    .review_composer
                    .set(&self.store, ReviewCommentComposerState::default());
                self.review_comment_editor.request_clear();
                self.kickoff_compare()
            }
            CancelCompare => self.cancel_compare(),
            SelectSidebarCommit(oid) => self.drill_into_commit(&oid),
            ClearSidebarCommit => self.restore_pre_drill_compare(),
            PreviewPullRequest => self.preview_pull_request(),
        }
    }

    fn drill_into_commit(&mut self, oid: &str) -> Vec<Effect> {
        if self
            .workspace
            .pre_drill_compare
            .with(&self.store, |p| p.is_none())
        {
            let left = self.compare.left_ref.get(&self.store);
            let right = self.compare.right_ref.get(&self.store);
            let mode = self.compare.mode.get(&self.store);
            self.workspace
                .pre_drill_compare
                .set(&self.store, Some((left, right, mode)));
        }
        self.compare
            .left_ref
            .set(&self.store, oid[..7.min(oid.len())].to_owned());
        self.compare.right_ref.set(&self.store, String::new());
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        self.compare
            .mode
            .set(&self.store, CompareMode::SingleCommit);
        self.kickoff_compare()
    }

    fn restore_pre_drill_compare(&mut self) -> Vec<Effect> {
        let mut taken: Option<(String, String, CompareMode)> = None;
        self.workspace
            .pre_drill_compare
            .update(&self.store, |p| taken = p.take());
        let Some((left, right, mode)) = taken else {
            return Vec::new();
        };
        self.compare.left_ref.set(&self.store, left);
        self.compare.right_ref.set(&self.store, right);
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        self.compare.mode.set(&self.store, mode);
        self.kickoff_compare()
    }
}

pub(super) const LARGE_COMPARE_FILE_LINES: i32 = 1_500;

pub(super) const COMPARE_STATS_CHUNK_SIZE: usize = 64;

pub(super) const COMPARE_STATS_BACKGROUND_CHUNK_SIZE: usize = 128 * 1024;

pub(super) const COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT: usize = 10_000;

pub(super) const COMPARE_STATS_VISIBLE_OVERSCAN_ROWS: usize = 32;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompareField {
    #[default]
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Store)]
pub struct CompareState {
    pub repo_path: Option<PathBuf>,
    pub left_ref: String,
    pub right_ref: String,
    pub mode: CompareMode,
    pub layout: LayoutMode,
    pub renderer: RendererKind,
    pub resolved_left: Option<String>,
    pub resolved_right: Option<String>,
}

impl Default for CompareState {
    fn default() -> Self {
        Self {
            repo_path: None,
            left_ref: String::new(),
            right_ref: String::new(),
            mode: CompareMode::default(),
            layout: LayoutMode::default(),
            renderer: RendererKind::default(),
            resolved_left: None,
            resolved_right: None,
        }
    }
}

pub use crate::core::compare::ComparePhase;

/// What the progress panel is about. Drives chip rendering: compare
/// shows a left⇄right ref pair, repo-open shows a single folder chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadingSubject {
    Compare {
        left_label: String,
        right_label: String,
    },
    RepoOpen {
        name: String,
    },
}

/// Transient progress state for a long-running workspace operation
/// (compare or repo open). Present iff something is in flight and the
/// reveal delay has either elapsed or was set to zero. Cleared when the
/// operation lands or the user cancels.
///
/// `reveal_at_ms` controls when the panel is rendered. Compares show
/// immediately; repo-open still uses the short delay to avoid flashing a
/// loading panel for tiny repositories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareProgress {
    pub generation: u64,
    pub phase: ComparePhase,
    pub subject: LoadingSubject,
    pub started_at_ms: u64,
    pub reveal_at_ms: u64,
    /// Total file count — first known from a backend `LoadingFiles`
    /// emission, re-confirmed by `CompareFinished`. Unused for RepoOpen.
    pub file_count_total: Option<u32>,
    /// Files read so far during `LoadingFiles`. Zero before, frozen
    /// after.
    pub files_loaded: u32,
}

/// Delay between kicking off an op and revealing the loading UI —
/// fast ops under this threshold show no loading flash at all.
pub const COMPARE_REVEAL_DELAY_MS: u64 = 500;

pub(super) fn vcs_compare_request(
    mode: CompareMode,
    left_ref: String,
    right_ref: String,
    layout: LayoutMode,
    renderer: RendererKind,
) -> VcsCompareRequest {
    let compare_spec = match mode {
        CompareMode::SingleCommit => {
            let revision = if right_ref.is_empty() {
                left_ref
            } else {
                right_ref
            };
            VcsCompareSpec::Change { revision }
        }
        CompareMode::TwoDot => VcsCompareSpec::Range {
            from: left_ref,
            to: right_ref,
        },
        CompareMode::ThreeDot => VcsCompareSpec::MergeBaseRange {
            base: left_ref,
            head: right_ref,
        },
    };
    VcsCompareRequest {
        spec: compare_spec,
        layout,
        renderer,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompareStatsHydrationState {
    #[default]
    Idle,
    Running,
    Failed,
}

pub(super) fn matching_persisted_compare<'a>(
    startup: &'a StartupOptions,
    settings: &'a Settings,
) -> Option<&'a PersistedCompare> {
    settings.last_compare.as_ref().filter(|compare| {
        startup.args.repo.is_some() && compare.repo_path.as_ref() == startup.args.repo.as_ref()
    })
}

pub(super) fn compare_refs_are_valid(mode: CompareMode, left_ref: &str, right_ref: &str) -> bool {
    match mode {
        CompareMode::SingleCommit => !left_ref.is_empty() || !right_ref.is_empty(),
        CompareMode::TwoDot | CompareMode::ThreeDot => {
            !left_ref.is_empty() && !right_ref.is_empty()
        }
    }
}

pub(super) fn estimated_carbon_file_rows_with_overhead(file: &carbon::FileDiff) -> u32 {
    if file.is_binary {
        return 4;
    }
    estimated_carbon_file_rows(file).saturating_add(1).max(1)
}

pub(super) fn estimated_carbon_file_rows(file: &carbon::FileDiff) -> u32 {
    if file.hunks.is_empty() {
        return file.additions.saturating_add(file.deletions).max(1);
    }

    let mut rows = 0_u32;
    for (hunk_index, hunk) in file.hunks.iter().enumerate() {
        if !file.is_partial {
            let gap_len = if hunk_index == 0 {
                hunk.old_start_index().min(hunk.new_start_index())
            } else {
                let prev = &file.hunks[hunk_index - 1];
                hunk.old_start_index()
                    .saturating_sub(prev.old_end_index())
                    .min(hunk.new_start_index().saturating_sub(prev.new_end_index()))
            };
            rows = rows.saturating_add((gap_len > 0) as u32);
        }

        rows = rows.saturating_add(1);
        for block in file.hunk_blocks(hunk) {
            rows = rows.saturating_add(match block.kind {
                carbon::BlockKind::Context => block.old.len.min(block.new.len),
                carbon::BlockKind::Change => block.old.len.saturating_add(block.new.len),
            });
        }

        if !file.is_partial && hunk_index + 1 == file.hunks.len() {
            let old_end = file
                .old_text
                .as_ref()
                .map(|text| text.line_count())
                .unwrap_or_else(|| hunk.old_end_index());
            let new_end = file
                .new_text
                .as_ref()
                .map(|text| text.line_count())
                .unwrap_or_else(|| hunk.new_end_index());
            let gap_len = old_end
                .saturating_sub(hunk.old_end_index())
                .min(new_end.saturating_sub(hunk.new_end_index()));
            rows = rows.saturating_add((gap_len > 0) as u32);
        }
    }
    rows
}

pub(super) fn compare_summary_file_entry(summary: &CompareFileSummary) -> FileListEntry {
    FileListEntry {
        path: summary.paths.display_path_ref(),
    }
}

pub(super) fn compare_output_file_entry_meta(
    output: &CompareOutput,
    index: usize,
) -> Option<FileListEntryMeta> {
    if let Some(summary) = output.file_summaries.get(index) {
        let (additions, deletions) = summary.fallback_stats();
        return Some(FileListEntryMeta {
            status: carbon_list_status(summary.status),
            additions,
            deletions,
            is_binary: summary.is_binary,
        });
    }
    output.carbon.files.get(index).map(carbon_file_entry_meta)
}

pub(super) fn carbon_file_entry_meta(file: &carbon::FileDiff) -> FileListEntryMeta {
    let (additions, deletions) = carbon_file_stats(file);
    FileListEntryMeta {
        status: carbon_list_status(file.status),
        additions,
        deletions,
        is_binary: file.is_binary,
    }
}

pub(super) fn compare_output_summary_is_deferred(output: &CompareOutput, index: usize) -> bool {
    if let Some(summary) = output.file_summaries.get(index) {
        return summary.is_partial;
    }
    output
        .carbon
        .files
        .get(index)
        .is_some_and(|file| file.is_partial && file.hunks.is_empty())
}

pub(super) fn compare_output_deferred_summary(
    output: &CompareOutput,
    index: usize,
) -> Option<CompareFileSummary> {
    if let Some(summary) = output.file_summaries.get(index) {
        return summary.is_partial.then(|| summary.clone());
    }
    output
        .carbon
        .files
        .get(index)
        .filter(|file| file.is_partial && file.hunks.is_empty())
        .map(CompareFileSummary::from_file)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct CompareStatsSnapshot {
    pub(super) hydrated_total: (i32, i32),
    pub(super) deferred_count: usize,
}

pub(super) fn compare_output_stats_snapshot(output: &CompareOutput) -> CompareStatsSnapshot {
    let mut snapshot = CompareStatsSnapshot::default();
    output.for_each_summary(|_, summary| {
        if summary.stats_deferred {
            snapshot.deferred_count = snapshot.deferred_count.saturating_add(1);
        } else {
            let stats = summary.fallback_stats();
            snapshot.hydrated_total = (
                snapshot.hydrated_total.0.saturating_add(stats.0),
                snapshot.hydrated_total.1.saturating_add(stats.1),
            );
        }
    });
    snapshot
}

pub(super) fn compare_output_has_deferred_stats(output: &CompareOutput) -> bool {
    if output.file_summaries.is_empty() {
        output.carbon.files.iter().any(|file| file.stats_deferred)
    } else {
        output
            .file_summaries
            .iter()
            .any(|summary| summary.stats_deferred)
    }
}

pub(super) fn carbon_file_stats(file: &carbon::FileDiff) -> (i32, i32) {
    if file.additions > 0 || file.deletions > 0 || file.stats_deferred {
        return (
            u32_to_i32_saturating(file.additions),
            u32_to_i32_saturating(file.deletions),
        );
    }
    let mut additions = 0_i32;
    let mut deletions = 0_i32;
    for block in &file.blocks {
        if block.kind == carbon::BlockKind::Change {
            additions = additions.saturating_add(block.new.len.min(i32::MAX as u32) as i32);
            deletions = deletions.saturating_add(block.old.len.min(i32::MAX as u32) as i32);
        }
    }
    (additions, deletions)
}

pub(super) fn u32_to_i32_saturating(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

impl AppState {
    pub(super) fn compare_file_is_large(&self, index: usize) -> bool {
        if self.workspace.source.get(&self.store) == WorkspaceSource::TextCompare {
            return false;
        }
        if self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .is_some_and(|output| compare_output_summary_is_deferred(output, index))
        }) {
            return true;
        }

        let meta = self.file_list_entry_meta(index);
        !meta.is_binary && meta.additions.saturating_add(meta.deletions) >= LARGE_COMPARE_FILE_LINES
    }

    pub(super) fn compare_refs(&self) -> (String, String) {
        let left_ref = self
            .compare
            .resolved_left
            .get(&self.store)
            .unwrap_or_else(|| self.compare.left_ref.get(&self.store));
        let right_ref = self
            .compare
            .resolved_right
            .get(&self.store)
            .unwrap_or_else(|| self.compare.right_ref.get(&self.store));
        (left_ref, right_ref)
    }
}

impl AppState {
    /// Clear the workspace back to a blank "no compare loaded" state. Replaces
    /// the former `WorkspaceState::clear_compare(&mut self)` method.
    pub(super) fn workspace_clear_compare(&mut self) {
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::None);
        self.workspace.status.set(&self.store, AsyncStatus::Idle);
        self.workspace
            .status_operation_pending
            .set(&self.store, false);
        self.workspace.status_generation.set(&self.store, 0);
        self.clear_syntax_inflight();
        self.workspace.files.set(&self.store, Vec::new());
        self.workspace
            .status_file_changes
            .set(&self.store, Vec::new());
        self.workspace.selected_file_index.set(&self.store, None);
        self.workspace.selected_file_path.set(&self.store, None);
        self.workspace.selected_change_bucket.set(&self.store, None);
        self.workspace.compare_output.set(&self.store, None);
        self.workspace.compare_total_stats.set(&self.store, None);
        self.workspace.compare_hydrated_stats.set(&self.store, None);
        self.workspace
            .compare_deferred_stats_remaining
            .set(&self.store, None);
        self.workspace
            .compare_deferred_stats_cursor
            .set(&self.store, 0);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.set_compare_stats_hydration(CompareStatsHydrationState::Idle);
        self.workspace.active_file.set(&self.store, None);
        self.workspace.active_file_loading.set(&self.store, None);
        self.clear_file_cache();
        self.workspace.raw_diff_len.set(&self.store, 0);
        self.workspace.used_fallback.set(&self.store, false);
        self.workspace
            .fallback_message
            .set(&self.store, String::new());
        self.workspace.sidebar_auto_width.set(&self.store, None);
        self.workspace.range_commits.set(&self.store, Vec::new());
        self.workspace
            .compare_history_pending
            .set(&self.store, None);
        self.workspace.pre_drill_compare.set(&self.store, None);
        self.workspace.expansions.update(&self.store, |m| m.clear());
        self.clear_file_scroll_layout();
        self.workspace.global_scroll_top_px.set(&self.store, 0);
    }

    #[profiling::function]
    pub(super) fn handle_compare_finished(&mut self, payload: CompareFinished) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

        let history_left = payload.resolved_left.clone();
        let history_right = self
            .vcs_ui_profile()
            .history_right_ref(&payload.resolved_right);
        self.workspace
            .status_operation_pending
            .set(&self.store, false);
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Compare);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace.mode.set(&self.store, WorkspaceMode::Ready);
        self.compare.layout.set(&self.store, payload.request.layout);
        self.compare
            .renderer
            .set(&self.store, payload.request.renderer);
        self.compare
            .resolved_left
            .set(&self.store, Some(payload.resolved_left));
        self.compare
            .resolved_right
            .set(&self.store, Some(payload.resolved_right));
        self.workspace
            .raw_diff_len
            .set(&self.store, payload.output.raw_diff_len);
        self.workspace
            .used_fallback
            .set(&self.store, payload.output.used_fallback);
        self.workspace
            .fallback_message
            .set(&self.store, payload.output.fallback_message.clone());
        let total_files = payload.output.file_count() as u32;
        let stats_snapshot = compare_output_stats_snapshot(&payload.output);
        let has_deferred_stats = stats_snapshot.deferred_count > 0;
        let eager_total_stats = (!has_deferred_stats).then_some(stats_snapshot.hydrated_total);
        self.workspace
            .compare_output
            .set(&self.store, Some(payload.output));
        self.workspace.files.set(&self.store, Vec::new());
        self.workspace
            .compare_total_stats
            .set(&self.store, eager_total_stats);
        self.workspace.compare_hydrated_stats.set(
            &self.store,
            has_deferred_stats.then_some(stats_snapshot.hydrated_total),
        );
        self.workspace
            .compare_deferred_stats_remaining
            .set(&self.store, Some(stats_snapshot.deferred_count));
        self.workspace
            .compare_deferred_stats_cursor
            .set(&self.store, 0);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.set_compare_stats_hydration(CompareStatsHydrationState::Idle);
        self.workspace.active_file_loading.set(&self.store, None);
        self.workspace.sidebar_auto_width.set(&self.store, None);
        self.clear_file_cache();
        self.reset_file_scroll_layout();
        self.workspace.global_scroll_top_px.set(&self.store, 0);
        // Record the discovered file count + advance the phase. The progress
        // panel stays up until the first file finishes mounting (or, for
        // small-file fast paths, is cleared by install_compare_active_file).
        self.workspace.compare_progress.update(&self.store, |slot| {
            if let Some(p) = slot.as_mut() {
                let p = Arc::make_mut(p);
                p.file_count_total = Some(total_files);
                p.phase = ComparePhase::PopulatingList;
            }
        });
        if self
            .workspace
            .pre_drill_compare
            .with(&self.store, |p| p.is_none())
        {
            self.workspace
                .range_commits
                .set(&self.store, payload.range_commits);
        }
        self.file_list.scroll_offset_px.set(&self.store, 0.0);
        self.file_list
            .commits_scroll_offset_px
            .set(&self.store, 0.0);
        self.editor_clear_document();
        // Clear overlays before claiming focus so the overlay restore target
        // does not clobber the file list focus below.
        self.clear_overlays();
        self.set_focus(Some(FocusTarget::FileList));

        let preferred_index = self
            .startup
            .preferred_file_index
            .or(self.workspace.selected_file_index.get(&self.store));
        let preferred_path = self
            .startup
            .preferred_file_path
            .clone()
            .or_else(|| self.workspace.selected_file_path.get(&self.store));

        let file_count = self.workspace_file_count();
        let index_for_path = preferred_path
            .as_deref()
            .and_then(|path| self.workspace_file_index_for_path(path));

        let mut effects = Vec::new();
        let mut selected_syntax_paths = Vec::new();
        let should_load_history = self
            .workspace
            .pre_drill_compare
            .with(&self.store, |p| p.is_none());
        let history_effect = should_load_history
            .then(|| self.compare_history_request(history_left, history_right))
            .flatten()
            .and_then(|request| {
                if has_deferred_stats {
                    self.workspace
                        .compare_history_pending
                        .set(&self.store, Some(request));
                    None
                } else {
                    Some(self.compare_history_effect(request))
                }
            });
        if let Some(index) = index_for_path
            .or(preferred_index.filter(|index| *index < file_count))
            .or_else(|| (file_count > 0).then_some(0))
        {
            if let Some(path) = self.workspace_file_path_at(index) {
                selected_syntax_paths.push(path);
            }
            effects.extend(self.select_file(index, true));
            if let Some(effect) = self.start_compare_stats_hydration_if_idle() {
                effects.push(effect);
            }
            if let Some(effect) = self.start_compare_total_stats_if_needed() {
                effects.push(effect);
            }
        } else {
            self.workspace.selected_file_index.set(&self.store, None);
            self.workspace.selected_file_path.set(&self.store, None);
            self.workspace.selected_change_bucket.set(&self.store, None);
            self.workspace.active_file.set(&self.store, None);
            self.workspace.active_file_loading.set(&self.store, None);
            // No files to select — the compare succeeded but has no diffs.
            // Tear down the progress panel; the "repo ready" hint takes over.
            self.workspace.compare_progress.set(&self.store, None);
            self.editor_clear_document();
        }
        if let Some(effect) = self.syntax_pack_warmup_effect_for_compare(&selected_syntax_paths) {
            effects.insert(0, effect);
        }
        if let Some(effect) = history_effect {
            effects.push(effect);
        }

        let (used_fallback, fallback_message) = (
            self.workspace.used_fallback.get(&self.store),
            self.workspace.fallback_message.get(&self.store),
        );
        if used_fallback && !fallback_message.is_empty() {
            self.push_info(&fallback_message);
        }
        effects
    }

    pub(super) fn handle_compare_history_ready(
        &mut self,
        payload: CompareHistoryReady,
    ) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }
        if self
            .workspace
            .pre_drill_compare
            .with(&self.store, |p| p.is_some())
        {
            return Vec::new();
        }
        self.workspace
            .range_commits
            .set(&self.store, payload.range_commits);
        Vec::new()
    }

    #[profiling::function]
    pub(super) fn handle_compare_file_finished(
        &mut self,
        payload: CompareFileFinished,
    ) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

        let matches_selected = self
            .workspace
            .selected_file_path
            .get(&self.store)
            .as_deref()
            == Some(payload.path.as_str());
        let matches_loading = self
            .workspace
            .active_file_loading
            .with(&self.store, |loading| {
                loading.as_ref().is_some_and(|loading| {
                    loading.index == payload.index && loading.path == payload.path
                })
            });
        let matches_cache_loading =
            self.workspace
                .file_cache_loading
                .with(&self.store, |loading| {
                    loading
                        .get(&payload.index)
                        .is_some_and(|loading| loading.path == payload.path)
                });
        if !matches_selected && !matches_cache_loading {
            return Vec::new();
        }

        if matches_selected && matches_loading {
            self.install_compare_active_file(payload.index, payload.path, payload.prepared);
        } else {
            let left_ref = self
                .compare
                .resolved_left
                .get(&self.store)
                .unwrap_or_else(|| self.compare.left_ref.get(&self.store));
            let right_ref = self
                .compare
                .resolved_right
                .get(&self.store)
                .unwrap_or_else(|| self.compare.right_ref.get(&self.store));
            let active_file = self.build_active_file(
                payload.index,
                payload.path,
                payload.prepared,
                left_ref,
                right_ref,
            );
            self.cache_active_file(active_file);
        }
        let mut effects = self.sync_editor_scroll_from_global();
        if matches_selected {
            effects.extend(self.request_active_file_syntax_effect());
        }
        if let Some(effect) = self.start_compare_stats_hydration_if_idle() {
            effects.push(effect);
        } else if let Some(effect) = self.start_compare_total_stats_if_needed() {
            effects.push(effect);
        }
        effects
    }

    pub(super) fn handle_compare_stats_ready(&mut self, payload: CompareStatsReady) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

        self.workspace
            .compare_total_stats
            .set(&self.store, Some((payload.additions, payload.deletions)));
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        let mut effects = Vec::new();
        if let Some(effect) = self.start_compare_stats_hydration_if_idle() {
            let is_background_stats = matches!(
                &effect,
                Effect::Compare(CompareEffect::LoadFileStats(task))
                    if task.request.priority == CompareWorkPriority::Warmup
            );
            effects.push(effect);
            if is_background_stats && let Some(effect) = self.take_pending_compare_history_effect()
            {
                effects.push(effect);
            }
        } else if !self.compare_stats_hydration_running()
            && let Some(effect) = self.take_pending_compare_history_effect()
        {
            effects.push(effect);
        }
        effects
    }

    pub(super) fn handle_compare_file_stats_ready(
        &mut self,
        payload: CompareFileStatsReady,
    ) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

        self.apply_compare_file_stats(&payload.stats);
        let mut effects = self.sync_editor_scroll_from_global();
        if !payload.request_complete {
            return effects;
        }
        if let Some(effect) = self.next_compare_stats_hydration_effect() {
            effects.push(effect);
            effects
        } else {
            self.set_compare_stats_hydration(CompareStatsHydrationState::Idle);
            let history_effect = self.take_pending_compare_history_effect();
            if let Some(stats) = self.exact_compare_total_stats_if_ready() {
                if !self.workspace.compare_total_stats_loading.get(&self.store) {
                    self.workspace
                        .compare_total_stats
                        .set(&self.store, Some(stats));
                    self.workspace
                        .compare_total_stats_loading
                        .set(&self.store, false);
                }
                if let Some(effect) = history_effect {
                    effects.push(effect);
                }
                return effects;
            }
            if let Some(effect) = self.start_compare_total_stats_if_needed() {
                effects.push(effect);
            }
            if let Some(effect) = history_effect {
                effects.push(effect);
            }
            effects
        }
    }

    pub(super) fn compare_stats_hydration_running(&self) -> bool {
        self.workspace.compare_stats_hydration.get(&self.store)
            == CompareStatsHydrationState::Running
    }

    pub(super) fn compare_stats_hydration_failed(&self) -> bool {
        self.workspace.compare_stats_hydration.get(&self.store)
            == CompareStatsHydrationState::Failed
    }

    pub(super) fn set_compare_stats_hydration(&self, state: CompareStatsHydrationState) {
        self.workspace
            .compare_stats_hydration
            .set(&self.store, state);
    }

    pub(super) fn start_compare_stats_hydration_if_idle(&mut self) -> Option<Effect> {
        if self.compare_stats_hydration_running() || self.compare_stats_hydration_failed() {
            return None;
        }

        let effect = self.next_compare_stats_hydration_effect()?;
        self.set_compare_stats_hydration(CompareStatsHydrationState::Running);
        Some(effect)
    }

    pub(super) fn start_visible_compare_stats_hydration(&mut self) -> Option<Effect> {
        if self.compare_stats_hydration_failed() {
            return None;
        }
        let prioritize_visible = self
            .workspace
            .compare_output
            .with(&self.store, |maybe_output| {
                maybe_output.as_ref().is_some_and(|output| {
                    output.file_count() > COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT
                })
            });
        if !prioritize_visible {
            return self.start_compare_stats_hydration_if_idle();
        }
        let visible_files = self.visible_compare_stats_hydration_items();
        if visible_files.is_empty() {
            return self.start_compare_stats_hydration_if_idle();
        }
        let effect = self.compare_file_stats_hydration_effect(
            visible_files,
            CompareWorkPriority::VisibleSidebarStats,
        )?;
        self.set_compare_stats_hydration(CompareStatsHydrationState::Running);
        Some(effect)
    }

    pub(super) fn start_compare_total_stats_if_needed(&mut self) -> Option<Effect> {
        if self
            .workspace
            .compare_total_stats
            .get(&self.store)
            .is_some()
            || self.workspace.compare_total_stats_loading.get(&self.store)
        {
            return None;
        }
        let repo_path = self.compare.repo_path.get(&self.store)?;
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, true);

        Some(
            CompareEffect::LoadStats(Task {
                generation: self.workspace.compare_generation.get(&self.store),
                request: CompareStatsRequest {
                    repo_path,
                    request: vcs_compare_request(
                        self.compare.mode.get(&self.store),
                        self.compare.left_ref.get(&self.store),
                        self.compare.right_ref.get(&self.store),
                        self.compare.layout.get(&self.store),
                        self.compare.renderer.get(&self.store),
                    ),
                    priority: CompareWorkPriority::TotalStats,
                },
            })
            .into(),
        )
    }

    pub(super) fn next_compare_stats_hydration_effect(&self) -> Option<Effect> {
        let prioritize_visible = self
            .workspace
            .compare_output
            .with(&self.store, |maybe_output| {
                maybe_output.as_ref().is_some_and(|output| {
                    output.file_count() > COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT
                })
            });
        let (files, priority) = if prioritize_visible {
            let visible_files = self.visible_compare_stats_hydration_items();
            if !visible_files.is_empty() {
                (visible_files, CompareWorkPriority::VisibleSidebarStats)
            } else {
                (
                    self.next_deferred_compare_stats_items(COMPARE_STATS_BACKGROUND_CHUNK_SIZE),
                    CompareWorkPriority::Warmup,
                )
            }
        } else {
            (
                self.next_deferred_compare_stats_items(COMPARE_STATS_BACKGROUND_CHUNK_SIZE),
                CompareWorkPriority::Warmup,
            )
        };
        if files.is_empty() {
            return None;
        }

        self.compare_file_stats_hydration_effect(files, priority)
    }

    pub(super) fn compare_file_stats_hydration_effect(
        &self,
        files: Vec<CompareFileStatsItem>,
        priority: CompareWorkPriority,
    ) -> Option<Effect> {
        if files.is_empty() {
            return None;
        }
        let repo_path = self.compare.repo_path.get(&self.store)?;
        Some(
            CompareEffect::LoadFileStats(Task {
                generation: self.workspace.compare_generation.get(&self.store),
                request: CompareFileStatsRequest {
                    repo_path,
                    request: vcs_compare_request(
                        self.compare.mode.get(&self.store),
                        self.compare.left_ref.get(&self.store),
                        self.compare.right_ref.get(&self.store),
                        self.compare.layout.get(&self.store),
                        self.compare.renderer.get(&self.store),
                    ),
                    files,
                    priority,
                },
            })
            .into(),
        )
    }

    pub(super) fn compare_history_request(
        &self,
        left_ref: String,
        right_ref: String,
    ) -> Option<CompareHistoryRequest> {
        Some(CompareHistoryRequest {
            repo_path: self.compare.repo_path.get(&self.store)?,
            left_ref,
            right_ref,
        })
    }

    pub(super) fn compare_history_effect(&self, request: CompareHistoryRequest) -> Effect {
        CompareEffect::LoadHistory(Task {
            generation: self.workspace.compare_generation.get(&self.store),
            request,
        })
        .into()
    }

    pub(super) fn take_pending_compare_history_effect(&mut self) -> Option<Effect> {
        if self
            .workspace
            .pre_drill_compare
            .with(&self.store, |p| p.is_some())
        {
            self.workspace
                .compare_history_pending
                .set(&self.store, None);
            return None;
        }
        let pending = self.workspace.compare_history_pending.get(&self.store)?;
        self.workspace
            .compare_history_pending
            .set(&self.store, None);
        Some(self.compare_history_effect(pending))
    }

    pub(super) fn next_deferred_compare_stats_items(
        &self,
        limit: usize,
    ) -> Vec<CompareFileStatsItem> {
        if limit == 0
            || self
                .workspace
                .compare_deferred_stats_remaining
                .get(&self.store)
                == Some(0)
        {
            return Vec::new();
        }

        let cursor = self
            .workspace
            .compare_deferred_stats_cursor
            .get(&self.store);
        let (items, next_cursor) =
            self.workspace
                .compare_output
                .with(&self.store, |maybe_output| {
                    let Some(output) = maybe_output.as_ref() else {
                        return (Vec::new(), None);
                    };
                    let file_count = output.file_count();
                    if file_count == 0 {
                        return (Vec::new(), None);
                    }
                    let mut items = Vec::new();
                    let mut index = cursor.min(file_count - 1);
                    let mut scanned = 0_usize;
                    while scanned < file_count && items.len() < limit {
                        if let Some(target) = output.deferred_stats_target_at(index) {
                            items.push(CompareFileStatsItem { index, target });
                        }
                        index = if index + 1 == file_count {
                            0
                        } else {
                            index + 1
                        };
                        scanned += 1;
                    }
                    (items, Some(index))
                });
        if let Some(next_cursor) = next_cursor {
            self.workspace
                .compare_deferred_stats_cursor
                .set(&self.store, next_cursor);
        }
        items
    }

    pub(super) fn visible_compare_stats_hydration_items(&self) -> Vec<CompareFileStatsItem> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Compare
            || self.file_list.tab.get(&self.store) != SidebarTab::Files
        {
            return Vec::new();
        }

        let stride = self.file_list_row_stride();
        if stride <= 0.0 {
            return Vec::new();
        }
        let scroll_px = self.file_list.scroll_offset_px.get(&self.store);
        let viewport_px = self.file_list.viewport_height.get(&self.store);
        let first = (scroll_px / stride).floor().max(0.0) as usize;
        let visible = (viewport_px / stride).ceil().max(1.0) as usize;
        let start = first.saturating_sub(COMPARE_STATS_VISIBLE_OVERSCAN_ROWS);
        let end = first
            .saturating_add(visible)
            .saturating_add(COMPARE_STATS_VISIBLE_OVERSCAN_ROWS);

        let filter = self
            .file_list
            .filter
            .with(&self.store, |filter| filter.clone());
        if !filter.is_empty() {
            let filtered_indices = self.workspace_file_filter_matches(&filter);
            let end = end.min(filtered_indices.len());
            if start >= end {
                return Vec::new();
            }
            return self.compare_stats_hydration_items_for_indices(
                filtered_indices[start..end].iter().copied(),
            );
        }

        if self.file_list.mode.get(&self.store) == SidebarMode::TreeView {
            let expanded_folders = self.file_list.expanded_folders.get(&self.store);
            let tree_indices = crate::ui::components::file_tree_visible_file_indices_by(
                |visit| {
                    self.for_each_workspace_file_path(|index, path| visit(index, path));
                },
                &expanded_folders,
                start..end,
            );
            return self.compare_stats_hydration_items_for_indices(tree_indices);
        }

        let end = end.min(self.workspace_file_count());
        if start >= end {
            return Vec::new();
        }
        self.compare_stats_hydration_items_for_indices(start..end)
    }

    pub(super) fn compare_stats_hydration_items_for_indices(
        &self,
        indices: impl IntoIterator<Item = usize>,
    ) -> Vec<CompareFileStatsItem> {
        self.workspace
            .compare_output
            .with(&self.store, |maybe_output| {
                let Some(output) = maybe_output.as_ref() else {
                    return Vec::new();
                };
                let mut items = Vec::new();
                for index in indices {
                    if items.len() >= COMPARE_STATS_CHUNK_SIZE {
                        break;
                    }
                    if let Some(target) = output.deferred_stats_target_at(index) {
                        items.push(CompareFileStatsItem { index, target });
                    }
                }
                items
            })
    }

    pub(super) fn exact_compare_total_stats_if_ready(&self) -> Option<(i32, i32)> {
        if let Some(remaining) = self
            .workspace
            .compare_deferred_stats_remaining
            .get(&self.store)
        {
            if remaining > 0 {
                return None;
            }
            if let Some(total) = self.workspace.compare_hydrated_stats.get(&self.store) {
                return Some(total);
            }
        }

        let ready = self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .is_some_and(|output| !compare_output_has_deferred_stats(output))
        });
        if !ready {
            return None;
        }
        self.workspace.compare_output.with(&self.store, |output| {
            let output = output.as_ref()?;
            let mut total = (0_i32, 0_i32);
            output.for_each_summary(|_, summary| {
                let stats = summary.fallback_stats();
                total = (
                    total.0.saturating_add(stats.0),
                    total.1.saturating_add(stats.1),
                );
            });
            Some(total)
        })
    }

    pub(super) fn apply_compare_file_stats(&mut self, stats: &[CompareFileStat]) {
        if stats.is_empty() {
            return;
        }

        let old_scroll_heights = stats
            .iter()
            .map(|stat| (stat.index, self.file_scroll_height_px(stat.index)))
            .collect::<Vec<_>>();

        let mut stats_delta = (0_i32, 0_i32);
        let mut newly_hydrated = 0_usize;
        self.workspace
            .compare_output
            .update(&self.store, |maybe_output| {
                let Some(output) = maybe_output.as_mut() else {
                    return;
                };
                for stat in stats {
                    let additions = i32_to_u32_nonnegative(stat.additions);
                    let deletions = i32_to_u32_nonnegative(stat.deletions);

                    if !output.file_summaries.is_empty() {
                        let Some(summary) = output.file_summaries.get_mut(stat.index) else {
                            continue;
                        };
                        if summary.path() != stat.path {
                            continue;
                        }
                        let old_stats = summary.fallback_stats();
                        let was_deferred = summary.stats_deferred;
                        summary.additions = additions;
                        summary.deletions = deletions;
                        summary.stats_deferred = false;
                        stats_delta = (
                            stats_delta
                                .0
                                .saturating_add(stat.additions.saturating_sub(old_stats.0)),
                            stats_delta
                                .1
                                .saturating_add(stat.deletions.saturating_sub(old_stats.1)),
                        );
                        newly_hydrated = newly_hydrated.saturating_add(was_deferred as usize);
                        continue;
                    }

                    if let Some(file) = output.carbon.files.get_mut(stat.index)
                        && file.path() == stat.path
                    {
                        let old_stats = carbon_file_stats(file);
                        let was_deferred = file.stats_deferred;
                        file.additions = additions;
                        file.deletions = deletions;
                        file.stats_deferred = false;
                        stats_delta = (
                            stats_delta
                                .0
                                .saturating_add(stat.additions.saturating_sub(old_stats.0)),
                            stats_delta
                                .1
                                .saturating_add(stat.deletions.saturating_sub(old_stats.1)),
                        );
                        newly_hydrated = newly_hydrated.saturating_add(was_deferred as usize);
                    }
                }
            });

        if stats_delta != (0, 0) {
            self.workspace
                .compare_hydrated_stats
                .update(&self.store, |total| {
                    let current = total.get_or_insert((0, 0));
                    *current = (
                        current.0.saturating_add(stats_delta.0),
                        current.1.saturating_add(stats_delta.1),
                    );
                });
        }
        if newly_hydrated > 0 {
            self.workspace
                .compare_deferred_stats_remaining
                .update(&self.store, |remaining| {
                    if let Some(count) = remaining.as_mut() {
                        *count = count.saturating_sub(newly_hydrated);
                    }
                });
        }

        let mut rebuilt_viewport_doc = false;
        self.workspace.active_file.update(&self.store, |slot| {
            let Some(active) = slot.as_mut() else {
                return;
            };
            for stat in stats {
                if apply_compare_stat_to_active_file(active, stat) {
                    rebuilt_viewport_doc = true;
                    break;
                }
            }
        });
        self.workspace.file_cache.update(&self.store, |files| {
            for active in files.values_mut() {
                for stat in stats {
                    if apply_compare_stat_to_active_file(active, stat) {
                        rebuilt_viewport_doc = true;
                        break;
                    }
                }
            }
        });
        if rebuilt_viewport_doc {
            self.viewport_document_cache = None;
        }

        let dragging_scrollbar = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.is_some());
        if dragging_scrollbar {
            self.workspace
                .file_scroll_recompute_pending
                .set(&self.store, true);
        } else {
            self.update_file_scroll_heights(old_scroll_heights);
            if self.settings.continuous_scroll {
                self.clamp_global_scroll_top_px();
            }
        }
    }

    pub(super) fn kickoff_compare(&mut self) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            self.push_error("Open a repository before starting a compare.");
            return Vec::new();
        };

        let mode = self.compare.mode.get(&self.store);
        let left_ref = self.compare.left_ref.get(&self.store);
        let right_ref = self.compare.right_ref.get(&self.store);
        if !compare_refs_are_valid(mode, &left_ref, &right_ref) {
            self.push_error("Provide the required refs for the selected mode.");
            return Vec::new();
        }

        let active_pr = self.github.pull_request.active.get(&self.store);
        let active_pr_still_matches = active_pr.as_ref().is_some_and(|key| {
            self.github.pull_request.cache.with(&self.store, |cache| {
                matches!(
                    cache.get(key).map(|entry| &entry.diff),
                    Some(PrPeekDiff::Ready {
                        left_ref: pr_left,
                        right_ref: pr_right,
                        ..
                    }) if pr_left == &left_ref && pr_right == &right_ref
                )
            })
        });
        if !active_pr_still_matches {
            self.github.pull_request.active.set(&self.store, None);
            self.github
                .pull_request
                .review_composer
                .set(&self.store, ReviewCommentComposerState::default());
            self.review_comment_editor.request_clear();
        }

        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Compare);
        let next_gen = self
            .workspace
            .compare_generation
            .get(&self.store)
            .saturating_add(1);
        self.workspace.compare_generation.set(&self.store, next_gen);
        let syntax_epoch_effect = self.invalidate_syntax_epoch_effect();
        self.workspace.compare_total_stats.set(&self.store, None);
        self.workspace.compare_hydrated_stats.set(&self.store, None);
        self.workspace
            .compare_deferred_stats_remaining
            .set(&self.store, None);
        self.workspace
            .compare_deferred_stats_cursor
            .set(&self.store, 0);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.set_compare_stats_hydration(CompareStatsHydrationState::Idle);
        self.workspace.expansions.update(&self.store, |m| m.clear());
        self.clear_overlays();
        self.sync_settings_snapshot();

        let started_at_ms = self.clock_ms;
        let reveal_at_ms = started_at_ms;
        let has_prior_state = self.workspace_file_count() > 0
            || self
                .workspace
                .active_file
                .with(&self.store, |af| af.is_some());

        if !has_prior_state {
            self.workspace.mode.set(&self.store, WorkspaceMode::Loading);
            self.workspace.status.set(&self.store, AsyncStatus::Loading);
        }

        let profile = self.vcs_ui_profile();
        let left_label = profile.compare_ref_display_label(&left_ref);
        let right_label = profile.compare_ref_display_label(&right_ref);
        self.workspace.compare_progress.set(
            &self.store,
            Some(Arc::new(CompareProgress {
                generation: next_gen,
                phase: ComparePhase::OpeningRepo,
                subject: LoadingSubject::Compare {
                    left_label,
                    right_label,
                },
                started_at_ms,
                reveal_at_ms,
                file_count_total: None,
                files_loaded: 0,
            })),
        );

        let renderer = self.compare.renderer.get(&self.store);
        let layout = self.compare.layout.get(&self.store);
        vec![
            syntax_epoch_effect,
            SettingsEffect::SaveSettings(self.settings.clone()).into(),
            CompareEffect::Run(Task {
                generation: next_gen,
                request: CompareRequest {
                    repo_path,
                    request: vcs_compare_request(mode, left_ref, right_ref, layout, renderer),
                    github_token: self.github_access_token.clone(),
                },
            })
            .into(),
        ]
    }

    /// Soft-cancel an in-flight compare. Bumps the generation so any
    /// result that eventually arrives is dropped by the guard, clears the
    /// progress panel, and returns the viewport to the default empty state.
    /// We do not attempt to interrupt backend work mid-flight; stale-result
    /// guards keep late answers from mutating newer state.
    pub(super) fn cancel_compare(&mut self) -> Vec<Effect> {
        if self
            .workspace
            .compare_progress
            .with(&self.store, |p| p.is_none())
        {
            return Vec::new();
        }
        let next_gen = self
            .workspace
            .compare_generation
            .get(&self.store)
            .saturating_add(1);
        self.workspace.compare_generation.set(&self.store, next_gen);
        let syntax_epoch_effect = self.invalidate_syntax_epoch_effect();
        self.workspace.compare_progress.set(&self.store, None);
        self.workspace.active_file_loading.set(&self.store, None);
        // Only revert the workspace mode if kickoff flipped it to Loading
        // (i.e. no prior state was preserved). When the user cancels a
        // re-compare, the old diff is still mounted and should stay visible.
        if self.workspace.mode.get(&self.store) == WorkspaceMode::Loading {
            self.workspace.mode.set(&self.store, WorkspaceMode::Empty);
            self.workspace.status.set(&self.store, AsyncStatus::Idle);
        }
        vec![syntax_epoch_effect]
    }

    pub(super) fn handle_compare_progress_update(&mut self, generation: u64, phase: ComparePhase) {
        // Only apply when the progress slot matches the reporter's
        // generation — stale workers silently lose their updates.
        self.workspace.compare_progress.update(&self.store, |slot| {
            if let Some(p) = slot.as_mut()
                && p.generation == generation
            {
                let p = Arc::make_mut(p);
                // Pull counts out of LoadingFiles so the determinate bar
                // reads directly from durable struct fields (cheaper than
                // pattern-matching in the render path, and lets the total
                // survive the phase transition to PopulatingList).
                if let ComparePhase::LoadingFiles {
                    files_seen,
                    files_total,
                } = phase
                {
                    p.files_loaded = files_seen;
                    if files_total > 0 {
                        p.file_count_total = Some(files_total);
                    }
                }
                p.phase = phase;
            }
        });
    }

    pub(super) fn swap_refs(&mut self) -> Vec<Effect> {
        let left = self.compare.left_ref.get(&self.store);
        let right = self.compare.right_ref.get(&self.store);
        let profile = self.vcs_ui_profile();
        if left.trim().is_empty()
            || right.trim().is_empty()
            || !profile.can_swap_ref(&right)
            || !profile.can_swap_ref(&left)
        {
            return Vec::new();
        }
        let resolved_left = self.compare.resolved_left.get(&self.store);
        let resolved_right = self.compare.resolved_right.get(&self.store);
        self.compare.left_ref.set(&self.store, right);
        self.compare.right_ref.set(&self.store, left);
        self.compare.resolved_left.set(&self.store, resolved_right);
        self.compare.resolved_right.set(&self.store, resolved_left);
        self.workspace.pre_drill_compare.set(&self.store, None);
        let mut effects = self.persist_settings_effect();
        let has_repo = self.compare.repo_path.with(&self.store, |p| p.is_some());
        let not_loading = self.workspace.status.get(&self.store) != AsyncStatus::Loading;
        let refs_valid = compare_refs_are_valid(
            self.compare.mode.get(&self.store),
            &self.compare.left_ref.get(&self.store),
            &self.compare.right_ref.get(&self.store),
        );
        if has_repo && not_loading && refs_valid {
            effects.extend(self.kickoff_compare());
        }
        effects
    }

    pub(super) fn update_compare_field(
        &mut self,
        field: CompareField,
        value: String,
    ) -> Vec<Effect> {
        self.workspace.pre_drill_compare.set(&self.store, None);
        match field {
            CompareField::Left => {
                self.compare.left_ref.set(&self.store, value);
                self.compare.resolved_left.set(&self.store, None);
            }
            CompareField::Right => {
                self.compare.right_ref.set(&self.store, value);
                self.compare.resolved_right.set(&self.store, None);
            }
        }
        self.auto_select_compare_mode();
        let active_field = self.overlays.ref_picker.active_field.get(&self.store);
        let mut effects = if matches!(self.overlays_top(), Some(OverlaySurface::RefPicker))
            && active_field == field
        {
            self.rebuild_ref_picker(field)
        } else {
            Vec::new()
        };
        effects.extend(self.rebuild_command_palette());
        effects
    }

    pub(super) fn auto_select_compare_mode(&mut self) {
        let profile = self.vcs_ui_profile();
        if !profile.should_auto_select_trunk_mode() {
            return;
        }
        let left = self.compare.left_ref.get(&self.store);
        let right = self.compare.right_ref.get(&self.store);
        if left.is_empty() || right.is_empty() {
            return;
        }
        if left == right && !profile.is_working_copy_ref(&right) {
            self.compare
                .mode
                .set(&self.store, CompareMode::SingleCommit);
            return;
        }
        let is_trunk = |r: &str| matches!(r, "main" | "master" | "develop" | "development");
        if is_trunk(&left) != is_trunk(&right) {
            self.compare.mode.set(&self.store, CompareMode::ThreeDot);
        }
    }
}
