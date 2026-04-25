use crate::actions::RepositoryAction;
use crate::effects::Effect;
use crate::events::{RepositoryEvent, RepositorySyncReason};

use super::{AppState, AsyncStatus, LoadingSubject, WorkspaceMode};

pub(super) fn reduce_action(state: &mut AppState, action: RepositoryAction) -> Vec<Effect> {
    state.apply_repository_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: RepositoryEvent) -> Vec<Effect> {
    match event {
        RepositoryEvent::RepositorySnapshotReady(payload) => {
            state.handle_repository_snapshot(payload)
        }
        RepositoryEvent::RepositorySnapshotFailed {
            path,
            reason,
            message,
        } => {
            if state
                .compare
                .repo_path
                .with(&state.store, |p| p.as_ref() == Some(&path))
            {
                if reason == RepositorySyncReason::Open {
                    state
                        .repository
                        .status
                        .set(&state.store, AsyncStatus::Failed);
                    state.workspace_mode.set(&state.store, WorkspaceMode::Empty);
                    state.compare_progress.update(&state.store, |slot| {
                        if let Some(p) = slot.as_ref()
                            && matches!(p.subject, LoadingSubject::RepoOpen { .. })
                        {
                            *slot = None;
                        }
                    });
                    state.push_error(&message);
                } else {
                    state.last_error.set(&state.store, Some(message));
                }
            }
            Vec::new()
        }
        RepositoryEvent::StatusOperationFailed { path, message } => {
            if state
                .compare
                .repo_path
                .with(&state.store, |p| p.as_ref() == Some(&path))
            {
                state
                    .workspace
                    .status_operation_pending
                    .set(&state.store, false);
                state.push_error(&message);
            }
            Vec::new()
        }
        RepositoryEvent::CommitCreated { path } => {
            if state
                .compare
                .repo_path
                .with(&state.store, |p| p.as_ref() == Some(&path))
            {
                state.commit_editor.request_clear();
                state.push_info("Commit created.");
            }
            Vec::new()
        }
        RepositoryEvent::CommitFailed { path, message } => {
            if state
                .compare
                .repo_path
                .with(&state.store, |p| p.as_ref() == Some(&path))
            {
                state.push_error(&message);
            }
            Vec::new()
        }
        RepositoryEvent::ContextLinesReady(payload) => state.handle_context_lines_ready(payload),
        RepositoryEvent::ContextLinesFailed {
            generation: _,
            file_index: _,
            message,
        } => {
            state.push_error(&message);
            Vec::new()
        }
        RepositoryEvent::FetchProgress {
            toast_id,
            received_objects,
            total_objects,
            received_bytes,
        } => {
            let fraction = if total_objects > 0 {
                received_objects as f32 / total_objects as f32
            } else {
                0.0
            };
            state.update_toast_progress(toast_id, fraction);
            if total_objects > 0 {
                let kib = received_bytes / 1024;
                state.update_toast_message(
                    toast_id,
                    &format!(
                        "Fetching {received_objects}/{total_objects} objects ({kib} KiB)\u{2026}"
                    ),
                );
            }
            Vec::new()
        }
        RepositoryEvent::FetchComplete {
            toast_id,
            path: _,
            remote,
        } => {
            state.finish_progress_toast(toast_id, &format!("Fetched {remote}"), None);
            Vec::new()
        }
        RepositoryEvent::FetchFailed {
            toast_id,
            remote,
            message,
        } => {
            state.fail_progress_toast(
                toast_id,
                &format!("Fetch from {remote} failed"),
                Some(message),
            );
            Vec::new()
        }
        RepositoryEvent::PushProgress {
            toast_id,
            current,
            total,
            bytes,
        } => {
            let fraction = if total > 0 {
                current as f32 / total as f32
            } else {
                0.0
            };
            state.update_toast_progress(toast_id, fraction);
            if total > 0 {
                let kib = bytes / 1024;
                state.update_toast_message(
                    toast_id,
                    &format!("Pushing {current}/{total} objects ({kib} KiB)\u{2026}"),
                );
            }
            Vec::new()
        }
        RepositoryEvent::PushComplete {
            toast_id,
            path: _,
            remote,
            branch,
        } => {
            state.finish_progress_toast(toast_id, &format!("Pushed {branch} to {remote}"), None);
            Vec::new()
        }
        RepositoryEvent::PushFailed {
            toast_id,
            remote,
            message,
        } => {
            state.fail_progress_toast(toast_id, &format!("Push to {remote} failed"), Some(message));
            Vec::new()
        }
        RepositoryEvent::PullComplete {
            toast_id,
            path: _,
            remote,
            branch,
            already_up_to_date,
            behind,
        } => {
            let message = if already_up_to_date {
                format!("{branch} is already up to date with {remote}")
            } else {
                format!("Fast-forwarded {branch} by {behind} commit(s) from {remote}")
            };
            state.finish_progress_toast(toast_id, &message, None);
            Vec::new()
        }
        RepositoryEvent::PullFailed {
            toast_id,
            remote: _,
            branch,
            message,
        } => {
            state.fail_progress_toast(
                toast_id,
                &format!("Pull into {branch} failed"),
                Some(message),
            );
            Vec::new()
        }
    }
}
