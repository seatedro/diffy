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
                state.workspace_mode.set(&state.store, WorkspaceMode::Empty);
                state.compare_progress.set(&state.store, None);
                state.push_error(&message);
            }
            Vec::new()
        }
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
                    return vec![effect];
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
                    state.compare_progress.set(&state.store, None);
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
