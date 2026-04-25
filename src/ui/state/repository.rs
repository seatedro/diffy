use crate::actions::RepositoryAction;
use crate::effects::Effect;
use crate::events::{RepositoryEvent, RepositorySyncReason};

use super::*;

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

impl AppState {
    pub(super) fn apply_repository_action(&mut self, action: RepositoryAction) -> Vec<Effect> {
        use RepositoryAction::*;
        match action {
            StageSelectedFile => self.apply_selected_status_operation(StatusOperation::Stage),
            UnstageSelectedFile => self.apply_selected_status_operation(StatusOperation::Unstage),
            DiscardSelectedFile => self.apply_selected_status_operation(StatusOperation::Discard),
            StageFile(index) => self.apply_file_status_operation(index, StatusOperation::Stage),
            UnstageFile(index) => self.apply_file_status_operation(index, StatusOperation::Unstage),
            StageAllFiles => self.apply_batch_scope_operation(
                &[StatusScope::Unstaged, StatusScope::Untracked],
                StatusOperation::Stage,
            ),
            UnstageAllFiles => {
                self.apply_batch_scope_operation(&[StatusScope::Staged], StatusOperation::Unstage)
            }
            StageHunk => self.apply_hunk_operation(StatusOperation::Stage, None),
            UnstageHunk => self.apply_hunk_operation(StatusOperation::Unstage, None),
            DiscardHunk => self.apply_hunk_operation(StatusOperation::Discard, None),
            StageHunkAt(i) => self.apply_hunk_operation(StatusOperation::Stage, Some(i)),
            UnstageHunkAt(i) => self.apply_hunk_operation(StatusOperation::Unstage, Some(i)),
            DiscardHunkAt(i) => self.apply_hunk_operation(StatusOperation::Discard, Some(i)),
            ToggleLineSelection(row) => {
                self.toggle_line_selection(row, false);
                let entries_len = self
                    .editor
                    .line_selection
                    .with(&self.store, |ls| ls.entries.len());
                tracing::info!(row, entries = entries_len, "ToggleLineSelection");
                Vec::new()
            }
            ToggleLineSelectionRange(row, anchor) => {
                self.toggle_line_selection_range(row, anchor);
                Vec::new()
            }
            StageSelectedLines => self.apply_line_selection_operation(StatusOperation::Stage),
            UnstageSelectedLines => self.apply_line_selection_operation(StatusOperation::Unstage),
            DiscardSelectedLines => self.apply_line_selection_operation(StatusOperation::Discard),
            ClearLineSelection => {
                self.editor
                    .line_selection
                    .update(&self.store, |ls| ls.clear());
                Vec::new()
            }
            SubmitCommit => self.submit_commit(),
            FetchRemote(remote) => self.start_fetch_remote(remote),
            FetchAllRemotes => self.start_fetch_all_remotes(),
            PushCurrentBranch { force_with_lease } => {
                self.start_push_current_branch(force_with_lease)
            }
            PullCurrentBranch => self.start_pull_current_branch(),
        }
    }

    fn submit_commit(&mut self) -> Vec<Effect> {
        let message = self.commit_editor.text().trim().to_owned();
        if message.is_empty() {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let has_staged = self.workspace.status_items.with(&self.store, |items| {
            items.iter().any(|item| item.scope == StatusScope::Staged)
        });
        if !has_staged {
            return Vec::new();
        }
        vec![RepositoryEffect::CreateCommit(CommitRequest { repo_path, message }).into()]
    }
}
