use crate::actions::CompareAction;
use crate::effects::Effect;
use crate::events::CompareEvent;

use super::{AppState, AsyncStatus, WorkspaceMode};

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
                if let Some(effect) = state.start_compare_stats_hydration_if_idle() {
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
                state
                    .workspace
                    .compare_stats_hydration_active
                    .set(&state.store, false);
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
