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
                    state.workspace.mode.set(&state.store, WorkspaceMode::Empty);
                    state
                        .workspace
                        .compare_progress
                        .update(&state.store, |slot| {
                            if let Some(p) = slot.as_ref()
                                && matches!(p.subject, LoadingSubject::RepoOpen { .. })
                            {
                                *slot = None;
                            }
                        });
                    state.push_error(&message);
                } else {
                    state.ui.last_error.set(&state.store, Some(message));
                }
            }
            Vec::new()
        }
        RepositoryEvent::WorkerStopped => {
            state
                .workspace
                .status_operation_pending
                .set(&state.store, false);
            state.push_error("Version control worker stopped. Restart Diffy.");
            Vec::new()
        }
        RepositoryEvent::FileOperationFailed { path, message } => {
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
        RepositoryEvent::VcsOperationComplete {
            toast_id,
            path,
            operation: _,
            message,
        } => {
            if state
                .compare
                .repo_path
                .with(&state.store, |p| p.as_ref() == Some(&path))
            {
                state.finish_progress_toast(toast_id, &message, None);
            }
            Vec::new()
        }
        RepositoryEvent::VcsOperationFailed {
            toast_id,
            operation,
            message,
        } => {
            state.fail_progress_toast(
                toast_id,
                &format!("{} failed", operation.label()),
                Some(message),
            );
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
        RepositoryEvent::PublishComplete {
            toast_id,
            path: _,
            label,
        } => {
            state.finish_progress_toast(toast_id, &label, None);
            Vec::new()
        }
        RepositoryEvent::PublishFailed { toast_id, message } => {
            state.fail_progress_toast(toast_id, "Publish failed", Some(message));
            Vec::new()
        }
        RepositoryEvent::PublishPlanReady {
            toast_id,
            path,
            plan,
        } => {
            if state
                .compare
                .repo_path
                .with(&state.store, |p| p.as_ref() != Some(&path))
            {
                return Vec::new();
            }
            if let Some(id) = toast_id {
                state.finish_progress_toast(id, "Publish options ready", None);
            }
            state.repository.publish_plan.set(&state.store, Some(plan));
            Vec::new()
        }
        RepositoryEvent::PublishPlanFailed { toast_id, message } => {
            if let Some(id) = toast_id {
                state.fail_progress_toast(id, "Publish options failed", Some(message));
            } else {
                state.push_error(&format!("Publish options failed: {message}"));
            }
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
            } else if behind == 0 {
                format!("Pulled {branch} from {remote}")
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
            StageSelectedFile => self.apply_selected_status_operation(FileOperation::Stage),
            UnstageSelectedFile => self.apply_selected_status_operation(FileOperation::Unstage),
            DiscardSelectedFile => self.apply_selected_status_operation(FileOperation::Discard),
            StageFile(index) => self.apply_file_status_operation(index, FileOperation::Stage),
            UnstageFile(index) => self.apply_file_status_operation(index, FileOperation::Unstage),
            StageAllFiles => self.apply_batch_scope_operation(
                &[ChangeBucket::Unstaged, ChangeBucket::Untracked],
                FileOperation::Stage,
            ),
            UnstageAllFiles => {
                self.apply_batch_scope_operation(&[ChangeBucket::Staged], FileOperation::Unstage)
            }
            StageHunk => self.apply_hunk_operation(FileOperation::Stage, None),
            UnstageHunk => self.apply_hunk_operation(FileOperation::Unstage, None),
            DiscardHunk => self.apply_hunk_operation(FileOperation::Discard, None),
            StageHunkAt(i) => self.apply_hunk_operation(FileOperation::Stage, Some(i)),
            UnstageHunkAt(i) => self.apply_hunk_operation(FileOperation::Unstage, Some(i)),
            DiscardHunkAt(i) => self.apply_hunk_operation(FileOperation::Discard, Some(i)),
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
            SetLineSelectionRange { row, anchor } => {
                self.set_line_selection_range(row, anchor);
                Vec::new()
            }
            SetLineSelectionFromDocument { entries, last_row } => {
                self.editor.line_selection.update(&self.store, |selection| {
                    selection.entries.clear();
                    selection.entries.extend(entries);
                    selection.last_toggled_row = Some(last_row);
                });
                Vec::new()
            }
            ToggleCurrentLineSelection => {
                self.toggle_current_line_selection();
                Vec::new()
            }
            ToggleCurrentLineSelectionRange => {
                self.toggle_current_line_selection_range();
                Vec::new()
            }
            StageSelectedLines => self.apply_line_selection_operation(FileOperation::Stage),
            UnstageSelectedLines => self.apply_line_selection_operation(FileOperation::Unstage),
            DiscardSelectedLines => self.apply_line_selection_operation(FileOperation::Discard),
            ClearLineSelection => {
                self.editor
                    .line_selection
                    .update(&self.store, |ls| ls.clear());
                Vec::new()
            }
            SubmitCommit => self.submit_commit(),
            RunOperation(operation) => self.start_vcs_operation(operation),
            FetchRemote(remote) => self.start_fetch_remote(remote),
            FetchAllRemotes => self.start_fetch_all_remotes(),
            PushCurrentBranch { force_with_lease } if force_with_lease => {
                self.start_push_current_branch(true)
            }
            PushCurrentBranch { .. } | PublishDefault => self.start_publish_default(),
            OpenPublishMenu => self.start_open_publish_menu(),
            Publish(action) => self.start_publish_action(action),
            PullCurrentBranch => self.start_pull_current_branch(),
        }
    }

    fn submit_commit(&mut self) -> Vec<Effect> {
        let capabilities = self.repository.capabilities.get(&self.store);
        if !capabilities.is_some_and(|capabilities| capabilities.create_commit) {
            self.push_error("This repository backend does not support commits.");
            return Vec::new();
        }
        let has_staging_area = capabilities.is_some_and(|capabilities| capabilities.staging_area);
        let message = self.commit_editor.text().trim().to_owned();
        if message.is_empty() {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let has_committable_changes =
            self.workspace
                .status_file_changes
                .with(&self.store, |changes| {
                    changes.iter().any(|change| {
                        if has_staging_area {
                            change.bucket == ChangeBucket::Staged
                        } else {
                            matches!(
                                change.bucket,
                                ChangeBucket::WorkingCopy | ChangeBucket::Conflicted
                            )
                        }
                    })
                });
        if !has_committable_changes {
            return Vec::new();
        }
        vec![RepositoryEffect::CreateCommit(CommitRequest { repo_path, message }).into()]
    }

    fn start_vcs_operation(&mut self, operation: VcsOperation) -> Vec<Effect> {
        if !self.repository.location.with(&self.store, |location| {
            vcs_operation_available_for_location(&operation, location.as_ref())
        }) {
            self.push_error("This operation is not available for the current repository backend.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before running repository operations.");
            return Vec::new();
        };
        let toast_id = self.push_progress_toast(&format!("{}\u{2026}", operation.progress_label()));
        vec![
            RepositoryEffect::RunOperation(VcsOperationRequest {
                repo_path,
                operation,
                toast_id,
            })
            .into(),
        ]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct RepositoryState {
    pub status: AsyncStatus,
    pub location: Option<RepoLocation>,
    pub capabilities: Option<RepoCapabilities>,
    pub refs: Vec<VcsRef>,
    pub changes: Vec<VcsChange>,
    pub operation_log: Vec<VcsOperationLogEntry>,
    pub file_changes: Vec<FileChange>,
    pub publish_plan: Option<PublishPlan>,
}

pub(super) fn active_publish_ref(refs: &[VcsRef]) -> Option<VcsRef> {
    refs.iter()
        .find(|reference| {
            reference.active && matches!(reference.kind, RefKind::Branch | RefKind::Bookmark)
        })
        .cloned()
}

pub(super) fn upstream_pair(upstream: &str) -> Option<(String, String)> {
    upstream
        .split_once('/')
        .map(|(remote, branch)| (remote.to_owned(), branch.to_owned()))
}

pub(super) fn remote_names_from_refs(refs: &[VcsRef]) -> std::collections::BTreeSet<String> {
    let mut remotes = std::collections::BTreeSet::new();
    for reference in refs {
        if let Some((remote, _)) = reference
            .upstream
            .as_deref()
            .and_then(|upstream| upstream.split_once('/'))
        {
            remotes.insert(remote.to_owned());
        }
        if matches!(
            reference.kind,
            RefKind::RemoteBranch | RefKind::RemoteBookmark
        ) && let Some((remote, _)) = reference.name.split_once('/')
        {
            remotes.insert(remote.to_owned());
        }
    }
    remotes
}

impl AppState {
    pub(super) fn status_refs_for_bucket(&self, bucket: ChangeBucket) -> (String, String) {
        self.vcs_ui_profile().status_compare_refs(bucket)
    }

    pub(super) fn vcs_ui_profile(&self) -> crate::ui::vcs::VcsUiProfile {
        self.repository.location.with(&self.store, |location| {
            crate::ui::vcs::profile(location.as_ref())
        })
    }
}

impl AppState {
    pub(super) fn open_repository(&mut self, path: PathBuf) -> Vec<Effect> {
        let path = normalize_repository_open_path(path);
        self.workspace.mode.set(&self.store, WorkspaceMode::Loading);
        self.compare.repo_path.set(&self.store, Some(path.clone()));
        self.compare.left_ref.set(&self.store, String::new());
        self.compare.right_ref.set(&self.store, String::new());
        self.compare.mode.set(&self.store, CompareMode::default());
        self.compare.resolved_left.set(&self.store, None);
        self.compare.resolved_right.set(&self.store, None);
        self.repository
            .status
            .set(&self.store, AsyncStatus::Loading);
        self.repository.location.set(&self.store, None);
        self.repository.capabilities.set(&self.store, None);
        self.repository.refs.set(&self.store, Vec::new());
        self.repository.changes.set(&self.store, Vec::new());
        self.repository.operation_log.set(&self.store, Vec::new());
        self.repository.file_changes.set(&self.store, Vec::new());
        self.repository.publish_plan.set(&self.store, None);
        self.workspace_clear_compare();
        self.reset_file_list();
        self.editor_clear_document();
        self.editor.focused.set(&self.store, false);
        self.ui.last_error.set(&self.store, None);
        self.github.pull_request.cache.update(&self.store, |c| {
            c.clear();
        });
        self.github
            .pull_request
            .pending_confirm
            .set(&self.store, None);
        self.clear_overlays();
        self.ui.focus.set(&self.store, Some(FocusTarget::TitleBar));
        self.sync_settings_snapshot();

        // Seed the progress panel with a repo-open subject. We piggy-back
        // on `compare_generation` as the loading generation — any in-flight
        // compare is invalidated when the user opens a new repo anyway,
        // and `handle_compare_progress_update` just matches on whatever
        // generation the panel records.
        let next_gen = self
            .workspace
            .compare_generation
            .get(&self.store)
            .saturating_add(1);
        self.workspace.compare_generation.set(&self.store, next_gen);
        let syntax_epoch_effect = self.invalidate_syntax_epoch_effect();
        let repo_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("repository")
            .to_owned();
        // Always delay the panel reveal — a tiny repo that opens in under
        // the threshold should finish without ever flashing a loading UI.
        // Unlike re-compare (which can preserve the old diff during the
        // grace window), repo-open has nothing to fall back to visually;
        // the empty background / previous workspace is what the user sees
        // for 500ms, which is a cheap price for zero flash on fast ops.
        let started_at_ms = self.clock_ms;
        let reveal_at_ms = started_at_ms.saturating_add(COMPARE_REVEAL_DELAY_MS);
        self.workspace.compare_progress.set(
            &self.store,
            Some(Arc::new(CompareProgress {
                generation: next_gen,
                phase: ComparePhase::OpeningRepo,
                subject: LoadingSubject::RepoOpen { name: repo_name },
                started_at_ms,
                reveal_at_ms,
                file_count_total: None,
                files_loaded: 0,
            })),
        );

        vec![
            syntax_epoch_effect,
            SettingsEffect::SaveSettings(self.settings.clone()).into(),
            RepositoryEffect::SyncRepository {
                path: path.clone(),
                reason: RepositorySyncReason::Open,
                reporter_generation: Some(next_gen),
            }
            .into(),
            RepositoryEffect::WatchRepository { path: Some(path) }.into(),
        ]
    }

    pub(super) fn handle_repository_snapshot(
        &mut self,
        payload: RepositorySnapshot,
    ) -> Vec<Effect> {
        tracing::debug!(
            path = %payload.path.display(),
            reason = ?payload.reason,
            change_kind = ?payload.change_kind,
            pending = self.workspace.status_operation_pending.get(&self.store),
            status_gen = self.workspace.status_generation.get(&self.store),
            "handle_repository_snapshot: entered"
        );
        if self
            .compare
            .repo_path
            .with(&self.store, |p| p.as_ref() != Some(&payload.path))
        {
            tracing::warn!("handle_repository_snapshot: path mismatch, ignored");
            return Vec::new();
        }

        self.repository.status.set(&self.store, AsyncStatus::Ready);
        self.repository
            .location
            .set(&self.store, Some(payload.location.clone()));
        self.repository
            .capabilities
            .set(&self.store, Some(payload.capabilities));
        let file_changes = payload.file_changes;
        self.repository.refs.set(&self.store, payload.refs);
        self.repository.changes.set(&self.store, payload.changes);
        self.repository
            .operation_log
            .set(&self.store, payload.operation_log);
        self.repository
            .file_changes
            .set(&self.store, file_changes.clone());
        self.repository
            .publish_plan
            .set(&self.store, payload.publish_plan);
        self.workspace
            .status_file_changes
            .set(&self.store, file_changes);

        // Tear down a repo-open progress panel. Compare-subject progress
        // survives — a kickoff_compare may be queued below and will
        // replace it atomically via its own seeding path.
        self.workspace.compare_progress.update(&self.store, |slot| {
            if let Some(p) = slot.as_ref()
                && matches!(p.subject, LoadingSubject::RepoOpen { .. })
            {
                *slot = None;
            }
        });

        match payload.reason {
            RepositorySyncReason::Open => {
                if let Some(ref store) = self.frecency {
                    store.record_access(&format!("repo:{}", payload.path.display()));
                }
                let mut effects = self.persist_settings_effect();
                if let Some(url) = self.startup.pending_pr_url.clone() {
                    self.startup.pending_pr_url = None;
                    self.startup.auto_compare_pending = false;
                    self.github
                        .pull_request
                        .status
                        .set(&self.store, AsyncStatus::Loading);
                    if let Some(parsed) = crate::core::forge::github::parse_pr_url(&url) {
                        let key: PrKey = (parsed.owner, parsed.repo, parsed.number);
                        self.github.pull_request.cache.update(&self.store, |c| {
                            c.entry(key.clone()).or_insert_with(|| PrCacheEntry {
                                meta: PrPeekMeta::Loading,
                                diff: PrPeekDiff::Loading,
                                last_peek_ms: 0,
                            });
                        });
                        self.github
                            .pull_request
                            .pending_confirm
                            .set(&self.store, Some(key));
                    }
                    effects.push(
                        GitHubEffect::LoadPullRequest {
                            url,
                            repo_path: payload.path,
                            github_token: self.github_access_token.clone(),
                        }
                        .into(),
                    );
                } else if self.startup.auto_compare_pending {
                    self.startup.auto_compare_pending = false;
                    effects.extend(self.kickoff_compare());
                } else if self.startup.bootstrap_compare_started {
                    self.startup.bootstrap_compare_started = false;
                } else if let Some(persisted) = self.settings.last_compare.as_ref().filter(|c| {
                    c.repo_path.as_ref() == Some(&payload.path)
                        && compare_refs_are_valid(c.mode, &c.left_ref, &c.right_ref)
                }) {
                    self.compare
                        .left_ref
                        .set(&self.store, persisted.left_ref.clone());
                    self.compare
                        .right_ref
                        .set(&self.store, persisted.right_ref.clone());
                    self.compare.mode.set(&self.store, persisted.mode);
                    effects.extend(self.kickoff_compare());
                } else {
                    let profile = crate::ui::vcs::profile(Some(&payload.location));
                    let (left, right, mode) = profile.default_compare();
                    self.compare.left_ref.set(&self.store, left.to_owned());
                    self.compare.right_ref.set(&self.store, right.to_owned());
                    self.compare.mode.set(&self.store, mode);
                    effects.extend(self.activate_status_view(true));
                }
                effects
            }
            RepositorySyncReason::Dirty | RepositorySyncReason::Rescan => {
                if self.workspace.source.get(&self.store) == WorkspaceSource::Status {
                    return self.activate_status_view(false);
                }

                let (mode, left_ref, right_ref) = (
                    self.compare.mode.get(&self.store),
                    self.compare.left_ref.get(&self.store),
                    self.compare.right_ref.get(&self.store),
                );
                if !compare_refs_are_valid(mode, &left_ref, &right_ref) {
                    return Vec::new();
                }

                match payload.change_kind {
                    Some(RepositoryChangeKind::Metadata | RepositoryChangeKind::Both) => {
                        self.kickoff_compare()
                    }
                    Some(RepositoryChangeKind::Worktree)
                        if self.vcs_ui_profile().is_working_copy_ref(&right_ref) =>
                    {
                        self.kickoff_compare()
                    }
                    _ => Vec::new(),
                }
            }
        }
    }

    pub(super) fn handle_status_diff_finished(
        &mut self,
        payload: StatusDiffFinished,
    ) -> Vec<Effect> {
        let current_gen = self.workspace.status_generation.get(&self.store);
        tracing::debug!(
            payload_gen = payload.generation,
            current_gen,
            payload_index = payload.index,
            payload_path = %payload.file_change.path,
            payload_bucket = ?payload.file_change.bucket,
            "handle_status_diff_finished: entered"
        );
        if payload.generation != current_gen {
            tracing::debug!(
                "handle_status_diff_finished: generation mismatch, discarding (pending NOT cleared)"
            );
            return Vec::new();
        }
        let matches = self.repository.file_changes.with(&self.store, |changes| {
            match changes.get(payload.index) {
                Some(current) => current == &payload.file_change,
                None => false,
            }
        });
        if !matches {
            let current_change_at_idx = self.repository.file_changes.with(&self.store, |changes| {
                changes
                    .get(payload.index)
                    .map(|change| format!("{}:{:?}", change.path, change.bucket))
                    .unwrap_or_else(|| "<out of range>".to_owned())
            });
            tracing::debug!(
                current_change_at_idx,
                "handle_status_diff_finished: file change mismatch, discarding (pending NOT cleared)"
            );
            return Vec::new();
        }
        let matches_selection = self.workspace.selected_file_index.get(&self.store)
            == Some(payload.index)
            && self
                .workspace
                .selected_file_path
                .get(&self.store)
                .as_deref()
                == Some(payload.file_change.path.as_str())
            && self.workspace.selected_change_bucket.get(&self.store)
                == Some(payload.file_change.bucket);
        let output = payload.output;

        let Some(carbon_file) = output.carbon.files.first() else {
            self.clear_file_cache_loading(payload.index);
            if matches_selection {
                self.workspace.active_file.set(&self.store, None);
                self.workspace.active_file_loading.set(&self.store, None);
                self.editor_clear_document();
            }
            return Vec::new();
        };
        let prepared = prepare_active_file(payload.index, carbon_file);
        let bucket = payload.file_change.bucket;
        let (left_ref, right_ref) = self.status_refs_for_bucket(bucket);
        let active_file = self.build_active_file(
            payload.index,
            payload.file_change.path.clone(),
            prepared,
            left_ref,
            right_ref,
        );
        let active_file = self.cache_active_file(active_file);
        if !matches_selection {
            return Vec::new();
        }

        tracing::debug!("handle_status_diff_finished: clearing status_operation_pending");
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Status);
        self.workspace
            .status_operation_pending
            .set(&self.store, false);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace.mode.set(&self.store, WorkspaceMode::Ready);
        self.workspace
            .used_fallback
            .set(&self.store, output.used_fallback);
        self.workspace
            .fallback_message
            .set(&self.store, output.fallback_message.clone());
        self.workspace
            .raw_diff_len
            .set(&self.store, output.raw_diff_len);
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
        self.workspace.active_file_loading.set(&self.store, None);

        self.workspace
            .selected_file_index
            .set(&self.store, Some(payload.index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(payload.file_change.path.clone()));
        self.workspace
            .selected_change_bucket
            .set(&self.store, Some(bucket));
        // Preserve scroll/hover/positional editor state when refreshing the
        // same file (e.g. after staging a hunk). Only reset when the path
        // changed (navigating to a different file).
        let same_file = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref().is_some_and(|a| {
                a.path == payload.file_change.path
                    && a.left_ref == active_file.left_ref
                    && a.right_ref == active_file.right_ref
            })
        });
        self.workspace
            .active_file
            .set(&self.store, Some(active_file));
        if !same_file {
            self.editor_clear_document();
            self.editor
                .line_selection
                .update(&self.store, |ls| ls.clear());
        }
        if self.editor.search.open.get(&self.store) {
            self.recompute_search_matches();
        }
        let mut effects = self.sync_editor_scroll_from_global();
        effects.extend(self.request_active_file_syntax_effect());
        effects
    }

    pub(super) fn activate_status_view(&mut self, reset_scroll: bool) -> Vec<Effect> {
        tracing::debug!(
            reset_scroll,
            pending = self.workspace.status_operation_pending.get(&self.store),
            status_gen = self.workspace.status_generation.get(&self.store),
            status_file_changes_count = self
                .workspace
                .status_file_changes
                .with(&self.store, |i| i.len()),
            "activate_status_view: entered"
        );
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Status);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace.mode.set(&self.store, WorkspaceMode::Ready);
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
        self.workspace.active_file_loading.set(&self.store, None);
        let new_files = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| build_status_file_entries(changes));
        self.workspace.files.set(&self.store, new_files);
        let next_status_gen = self
            .workspace
            .status_generation
            .get(&self.store)
            .saturating_add(1);
        self.workspace
            .status_generation
            .set(&self.store, next_status_gen);
        let syntax_epoch_effect = self.invalidate_syntax_epoch_effect();
        self.clear_file_cache();
        self.workspace.sidebar_auto_width.set(&self.store, None);
        self.workspace.used_fallback.set(&self.store, false);
        self.workspace
            .fallback_message
            .set(&self.store, String::new());
        self.workspace.raw_diff_len.set(&self.store, 0);
        self.reset_file_scroll_layout();
        if reset_scroll {
            self.file_list.scroll_offset_px.set(&self.store, 0.0);
            self.workspace.global_scroll_top_px.set(&self.store, 0);
        } else if self.settings.continuous_scroll {
            self.clamp_global_scroll_top_px();
        }

        let current_path = self.workspace.selected_file_path.get(&self.store);
        let current_bucket = self.workspace.selected_change_bucket.get(&self.store);
        let (status_syntax_paths, selected_index, selected_syntax_paths) = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| {
                let paths = changes
                    .iter()
                    .flat_map(file_change_syntax_paths)
                    .collect::<Vec<_>>();
                let selected_index =
                    if let Some((path, bucket)) = current_path.clone().zip(current_bucket) {
                        if let Some(idx) = changes
                            .iter()
                            .position(|change| change.path == path && change.bucket == bucket)
                        {
                            Some(idx)
                        } else {
                            None
                        }
                    } else if let Some(path) = current_path.as_deref() {
                        if let Some(idx) = changes.iter().position(|change| change.path == path) {
                            Some(idx)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                    .or_else(|| (!changes.is_empty()).then_some(0));
                let selected_paths = selected_index
                    .and_then(|index| changes.get(index))
                    .map(file_change_syntax_paths)
                    .unwrap_or_default();
                (paths, selected_index, selected_paths)
            });

        tracing::debug!(
            ?selected_index,
            "activate_status_view: resolved selected_index"
        );
        match selected_index {
            Some(index) => {
                let mut effects = self.select_status_item(index, false);
                effects.insert(0, syntax_epoch_effect);
                if let Some(effect) = self.syntax_pack_warmup_effect_for_paths(
                    &status_syntax_paths,
                    &selected_syntax_paths,
                ) {
                    effects.insert(0, effect);
                }
                effects
            }
            None => {
                tracing::debug!("activate_status_view: no selection, clearing pending");
                self.workspace
                    .status_operation_pending
                    .set(&self.store, false);
                self.workspace.selected_file_index.set(&self.store, None);
                self.workspace.selected_file_path.set(&self.store, None);
                self.workspace.selected_change_bucket.set(&self.store, None);
                self.workspace.active_file.set(&self.store, None);
                self.workspace.active_file_loading.set(&self.store, None);
                self.editor_clear_document();
                vec![syntax_epoch_effect]
            }
        }
    }

    pub(super) fn show_working_tree(&mut self) -> Vec<Effect> {
        let (left, right, mode) = self.vcs_ui_profile().working_copy_compare();
        self.compare.left_ref.set(&self.store, left.to_owned());
        self.compare.right_ref.set(&self.store, right.to_owned());
        self.compare.mode.set(&self.store, mode);
        let mut effects = self.persist_settings_effect();
        effects.extend(self.activate_status_view(true));
        effects
    }

    pub(super) fn select_status_item(&mut self, index: usize, reveal: bool) -> Vec<Effect> {
        let Some(file_change) = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| changes.get(index).cloned())
        else {
            tracing::warn!(
                index,
                "select_status_item: index out of range, returning empty"
            );
            return Vec::new();
        };
        tracing::debug!(
            index,
            path = %file_change.path,
            bucket = ?file_change.bucket,
            status_gen = self.workspace.status_generation.get(&self.store),
            "select_status_item: dispatching LoadStatusDiff"
        );
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            tracing::warn!("select_status_item: no repo_path");
            return Vec::new();
        };

        self.workspace
            .source
            .set(&self.store, WorkspaceSource::Status);
        // Keep the current document visible while the new diff loads — no
        // Loading state, no tear-down. handle_status_diff_finished swaps the
        // ActiveFile atomically when the fresh diff arrives.
        self.workspace
            .selected_file_index
            .set(&self.store, Some(index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(file_change.path.clone()));
        self.workspace
            .selected_change_bucket
            .set(&self.store, Some(file_change.bucket));
        let (left_ref, right_ref) = self.status_refs_for_bucket(file_change.bucket);
        let active_matches_selection = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref().is_some_and(|active| {
                active.index == index
                    && active.path == file_change.path
                    && active.left_ref == left_ref
                    && active.right_ref == right_ref
            })
        });
        if active_matches_selection {
            self.workspace.active_file_loading.set(&self.store, None);
            self.clear_file_cache_loading(index);
            self.file_list.hovered_index.set(&self.store, Some(index));
            if reveal {
                self.reveal_file_list_row(index);
            }
            let mut effects = self.sync_editor_scroll_from_global();
            effects.push(ensure_syntax_packs_for_file_change_effect(&file_change));
            effects.extend(self.request_active_file_syntax_effect());
            return effects;
        } else if let Some(mut active_file) = self.cached_status_file_at(index, &file_change) {
            active_file.last_used_tick = self.next_file_working_set_tick();
            self.workspace.active_file_loading.set(&self.store, None);
            self.workspace
                .active_file
                .set(&self.store, Some(active_file.clone()));
            self.cache_active_file(active_file);
            self.editor_clear_document();
            self.file_list.hovered_index.set(&self.store, Some(index));
            if reveal {
                self.reveal_file_list_row(index);
            }
            let mut effects = self.sync_editor_scroll_from_global();
            effects.push(ensure_syntax_packs_for_file_change_effect(&file_change));
            effects.extend(self.request_active_file_syntax_effect());
            return effects;
        } else {
            let should_load = self.should_enqueue_file_load(
                index,
                &file_change.path,
                CompareWorkPriority::InteractiveSelectedFile,
            );
            self.workspace.active_file_loading.set(
                &self.store,
                Some(ActiveFileLoading {
                    index,
                    path: file_change.path.clone(),
                    priority: CompareWorkPriority::InteractiveSelectedFile,
                }),
            );
            self.mark_file_cache_loading(
                index,
                file_change.path.clone(),
                CompareWorkPriority::InteractiveSelectedFile,
            );
            self.file_list.hovered_index.set(&self.store, Some(index));
            if reveal {
                self.reveal_file_list_row(index);
            }

            let mut effects = vec![ensure_syntax_packs_for_file_change_effect(&file_change)];
            if should_load {
                let generation = self.workspace.status_generation.get(&self.store);
                let renderer = self.compare.renderer.get(&self.store);
                effects.push(
                    RepositoryEffect::LoadStatusDiff {
                        task: Task {
                            generation,
                            request: StatusDiffRequest {
                                repo_path,
                                file_change,
                                renderer,
                            },
                        },
                        index,
                    }
                    .into(),
                );
            }
            return effects;
        }
    }

    pub(super) fn apply_selected_status_operation(
        &mut self,
        operation: FileOperation,
    ) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.staging_area)
            })
        {
            self.push_error("This repository backend does not support staging operations.");
            return Vec::new();
        }
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let Some(index) = self.workspace.selected_file_index.get(&self.store) else {
            return Vec::new();
        };
        let Some(file_change) = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| changes.get(index).cloned())
        else {
            return Vec::new();
        };

        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        vec![
            RepositoryEffect::ApplyFileOperation(FileOperationRequest {
                repo_path,
                file_change,
                operation,
            })
            .into(),
        ]
    }

    pub(super) fn apply_file_status_operation(
        &mut self,
        index: usize,
        operation: FileOperation,
    ) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.staging_area)
            })
        {
            self.push_error("This repository backend does not support staging operations.");
            return Vec::new();
        }
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let Some(file_change) = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| changes.get(index).cloned())
        else {
            return Vec::new();
        };

        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        vec![
            RepositoryEffect::ApplyFileOperation(FileOperationRequest {
                repo_path,
                file_change,
                operation,
            })
            .into(),
        ]
    }

    pub(super) fn apply_batch_scope_operation(
        &mut self,
        buckets: &[ChangeBucket],
        operation: FileOperation,
    ) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.staging_area)
            })
        {
            self.push_error("This repository backend does not support staging operations.");
            return Vec::new();
        }
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let file_changes: Vec<FileChange> =
            self.workspace
                .status_file_changes
                .with(&self.store, |changes| {
                    changes
                        .iter()
                        .filter(|change| buckets.contains(&change.bucket))
                        .cloned()
                        .collect()
                });
        if file_changes.is_empty() {
            return Vec::new();
        }

        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        vec![
            RepositoryEffect::ApplyBatchFileOperation(BatchFileOperationRequest {
                repo_path,
                file_changes,
                operation,
            })
            .into(),
        ]
    }

    pub(super) fn start_fetch_remote(&mut self, remote: String) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.remotes)
            })
        {
            self.push_error("This repository backend does not support remotes.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before fetching.");
            return Vec::new();
        };
        let toast_id = self.push_progress_toast(&format!("Fetching {remote}\u{2026}"));
        vec![
            RepositoryEffect::FetchRemote(FetchRemoteRequest {
                repo_path,
                remote,
                toast_id,
            })
            .into(),
        ]
    }

    pub(super) fn start_fetch_all_remotes(&mut self) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.remotes)
            })
        {
            self.push_error("This repository backend does not support remotes.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before fetching.");
            return Vec::new();
        };
        let remotes = self.repository.refs.with(&self.store, |refs| {
            remote_names_from_refs(refs).into_iter().collect::<Vec<_>>()
        });
        if remotes.is_empty() {
            self.push_error("No remotes are configured for this repository.");
            return Vec::new();
        }
        remotes
            .into_iter()
            .flat_map(|remote| {
                let toast_id = self.push_progress_toast(&format!("Fetching {remote}\u{2026}"));
                std::iter::once(
                    RepositoryEffect::FetchRemote(FetchRemoteRequest {
                        repo_path: repo_path.clone(),
                        remote,
                        toast_id,
                    })
                    .into(),
                )
            })
            .collect()
    }

    pub(super) fn start_publish_default(&mut self) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.remotes)
            })
        {
            self.push_error("This repository backend does not support publishing.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before publishing.");
            return Vec::new();
        };
        let toast_id = self.push_progress_toast(&format!(
            "{}\u{2026}",
            self.vcs_ui_profile().publish_command_label()
        ));
        vec![
            RepositoryEffect::PublishDefault(PublishRequest {
                repo_path,
                action: None,
                toast_id,
            })
            .into(),
        ]
    }

    pub(super) fn start_open_publish_menu(&mut self) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.remotes)
            })
        {
            self.push_error("This repository backend does not support publishing.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before publishing.");
            return Vec::new();
        };
        self.push_overlay(OverlaySurface::PublishMenu, None);
        vec![
            RepositoryEffect::LoadPublishPlan(PublishPlanRequest {
                repo_path,
                toast_id: None,
            })
            .into(),
        ]
    }

    pub(super) fn start_publish_action(&mut self, action: PublishAction) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before publishing.");
            return Vec::new();
        };
        if self.overlays_top() == Some(OverlaySurface::PublishMenu) {
            self.pop_overlay();
        }
        let toast_id = self.push_progress_toast(&format!("{}\u{2026}", action.label));
        vec![
            RepositoryEffect::PublishDefault(PublishRequest {
                repo_path,
                action: Some(action),
                toast_id,
            })
            .into(),
        ]
    }

    pub(super) fn start_push_current_branch(&mut self, force_with_lease: bool) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.remotes)
            })
        {
            self.push_error("This repository backend does not support push.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before pushing.");
            return Vec::new();
        };
        let Some(branch_ref) = self
            .repository
            .refs
            .with(&self.store, |refs| active_publish_ref(refs))
        else {
            self.push_error("No active branch or bookmark to push.");
            return Vec::new();
        };
        let branch = branch_ref.name;
        let (remote, refspec) = match branch_ref.upstream.as_deref().and_then(upstream_pair) {
            Some((remote, upstream_branch)) => (
                remote,
                format!("refs/heads/{branch}:refs/heads/{upstream_branch}"),
            ),
            None => {
                // No upstream configured yet — default to `origin/<branch>`.
                let remotes = self.repository.refs.with(&self.store, |refs| {
                    remote_names_from_refs(refs).into_iter().collect::<Vec<_>>()
                });
                let remote = if remotes.iter().any(|n| n == "origin") {
                    "origin".to_owned()
                } else if let Some(first) = remotes.first() {
                    first.clone()
                } else {
                    self.push_error("No remotes are configured for this repository.");
                    return Vec::new();
                };
                (remote, format!("refs/heads/{branch}:refs/heads/{branch}"))
            }
        };
        let label = if force_with_lease {
            format!("Force-pushing {branch} to {remote}\u{2026}")
        } else {
            format!("Pushing {branch} to {remote}\u{2026}")
        };
        let toast_id = self.push_progress_toast(&label);
        vec![
            RepositoryEffect::Push(PushRequest {
                repo_path,
                remote,
                refspec,
                force_with_lease,
                toast_id,
            })
            .into(),
        ]
    }

    pub(super) fn start_pull_current_branch(&mut self) -> Vec<Effect> {
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.pull_fast_forward)
            })
        {
            self.push_error("This repository backend does not support fast-forward pull.");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.with(&self.store, |p| p.clone()) else {
            self.push_error("Open a repository before pulling.");
            return Vec::new();
        };
        let Some(branch_ref) = self
            .repository
            .refs
            .with(&self.store, |refs| active_publish_ref(refs))
        else {
            self.push_error("No active branch or bookmark to pull into.");
            return Vec::new();
        };
        let branch = branch_ref.name;
        let (remote, upstream_branch) = match branch_ref.upstream.as_deref().and_then(upstream_pair)
        {
            Some(pair) => pair,
            None => {
                self.push_error(&format!(
                    "No upstream configured for {branch}. Push once to set one."
                ));
                return Vec::new();
            }
        };
        let toast_id = self.push_progress_toast(&format!("Pulling {branch} from {remote}\u{2026}"));
        vec![
            RepositoryEffect::PullFf(PullFfRequest {
                repo_path,
                remote,
                branch: upstream_branch,
                toast_id,
            })
            .into(),
        ]
    }
}
