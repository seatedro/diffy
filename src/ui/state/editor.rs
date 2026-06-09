use crate::actions::EditorAction;
use crate::effects::Effect;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: EditorAction) -> Vec<Effect> {
    state.apply_editor_action(action)
}

impl AppState {
    pub(super) fn apply_editor_action(&mut self, action: EditorAction) -> Vec<Effect> {
        use EditorAction::*;
        match action {
            ScrollViewportLines(delta) => {
                let mut effects = self.scroll_viewport_lines(delta);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            ScrollViewportPx(delta_px) => {
                let mut effects = self.scroll_viewport_px(delta_px);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            ScrollViewportPages(delta) => {
                let mut effects = self.scroll_viewport_pages(delta);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            ScrollViewportTo(px) => {
                self.editor.scroll_top_px.set(&self.store, px);
                self.editor_clamp_scroll();
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            ScrollViewportToGlobal(px) => self.scroll_viewport_to_global(px),
            BeginViewportScrollbarDrag {
                content_height_px,
                viewport_height_px,
                scroll_top_px,
                max_scroll_top_px,
            } => {
                self.begin_viewport_scrollbar_drag(
                    content_height_px,
                    viewport_height_px,
                    scroll_top_px,
                    max_scroll_top_px,
                );
                Vec::new()
            }
            EndViewportScrollbarDrag => {
                self.end_viewport_scrollbar_drag();
                let current = self.workspace.global_scroll_top_px.get(&self.store);
                let mut effects = self.scroll_viewport_to_global(current);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            ScrollViewportHalfPage(dir) => {
                let mut effects = self.scroll_viewport_half_page(dir);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            HoverReviewAddButton(hovered) => {
                self.editor
                    .review_add_hovered
                    .set_if_changed(&self.store, hovered);
                Vec::new()
            }
            HoverViewportRow(row) => {
                self.editor.hovered_row.set(&self.store, row);
                if row.is_none() {
                    self.editor.hovered_render_line_index.set(&self.store, None);
                    self.editor.hovered_hunk_index.set(&self.store, None);
                }
                Vec::new()
            }
            MoveRowCursor(delta) => {
                self.move_editor_row_cursor(delta);
                Vec::new()
            }
            FocusViewport => {
                self.set_focus(Some(FocusTarget::Editor));
                Vec::new()
            }
            GoToNextHunk => {
                self.navigate_to_hunk(true);
                if self.settings.continuous_scroll {
                    self.sync_global_scroll_from_editor();
                }
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            GoToPreviousHunk => {
                self.navigate_to_hunk(false);
                if self.settings.continuous_scroll {
                    self.sync_global_scroll_from_editor();
                }
                self.request_active_file_syntax_effect()
                    .into_iter()
                    .collect()
            }
            GoToNextFile => {
                let mut effects = self.navigate_to_file(true);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            GoToPreviousFile => {
                let mut effects = self.navigate_to_file(false);
                effects.extend(self.request_active_file_syntax_effect());
                effects
            }
            OpenSearch => {
                self.open_search();
                Vec::new()
            }
            CloseSearch => {
                self.close_search();
                Vec::new()
            }
            SearchNext => {
                self.search_navigate(1);
                Vec::new()
            }
            SearchPrevious => {
                self.search_navigate(-1);
                Vec::new()
            }
            EditorClick(x, y) => {
                match self.ui.focus.get(&self.store) {
                    Some(FocusTarget::SettingsSteeringPrompt) => {
                        self.steering_prompt_editor.click(x, y);
                    }
                    Some(FocusTarget::ReviewCommentEditor) => {
                        self.review_comment_editor.click(x, y);
                    }
                    Some(FocusTarget::TextCompareLeft) => {
                        self.text_compare.left_editor.click(x, y);
                    }
                    Some(FocusTarget::TextCompareRight) => {
                        self.text_compare.right_editor.click(x, y);
                    }
                    _ => {
                        self.commit_editor.click(x, y);
                    }
                }
                Vec::new()
            }
            EditorDrag(x, y) => {
                match self.ui.focus.get(&self.store) {
                    Some(FocusTarget::SettingsSteeringPrompt) => {
                        self.steering_prompt_editor.drag(x, y);
                    }
                    Some(FocusTarget::ReviewCommentEditor) => {
                        self.review_comment_editor.drag(x, y);
                    }
                    Some(FocusTarget::TextCompareLeft) => {
                        self.text_compare.left_editor.drag(x, y);
                    }
                    Some(FocusTarget::TextCompareRight) => {
                        self.text_compare.right_editor.drag(x, y);
                    }
                    _ => {
                        self.commit_editor.drag(x, y);
                    }
                }
                Vec::new()
            }
            EditorScrollPx(delta) => {
                match self.ui.focus.get(&self.store) {
                    Some(FocusTarget::SettingsSteeringPrompt) => {
                        self.steering_prompt_editor.scroll(delta as f32);
                    }
                    Some(FocusTarget::ReviewCommentEditor) => {
                        self.review_comment_editor.scroll(delta as f32);
                    }
                    Some(FocusTarget::TextCompareLeft) => {
                        self.text_compare.left_editor.scroll(delta as f32);
                    }
                    Some(FocusTarget::TextCompareRight) => {
                        self.text_compare.right_editor.scroll(delta as f32);
                    }
                    _ => {
                        self.commit_editor.scroll(delta as f32);
                    }
                }
                Vec::new()
            }
            BeginViewportTextSelection { point, generation } => {
                // Mutually exclusive with a review-card text selection.
                self.github
                    .pull_request
                    .card_text_selection
                    .set(&self.store, None);
                self.editor.text_selection.set(
                    &self.store,
                    Some(crate::editor::diff::state::ViewportTextSelection::new(
                        generation, point,
                    )),
                );
                Vec::new()
            }
            ExtendViewportTextSelection(point) => {
                self.editor.text_selection.update(&self.store, |selection| {
                    if let Some(selection) = selection {
                        selection.focus = point;
                    }
                });
                Vec::new()
            }
            ClearViewportTextSelection => {
                self.editor.text_selection.set(&self.store, None);
                self.github
                    .pull_request
                    .card_text_selection
                    .set(&self.store, None);
                Vec::new()
            }
            ExpandContextAbove(hunk_index, amount) => self.expand_context(
                hunk_index,
                crate::editor::diff::expansion::ExpandDirection::Above,
                amount,
            ),
            ExpandContextBelow(hunk_index, amount) => self.expand_context(
                hunk_index,
                crate::editor::diff::expansion::ExpandDirection::Below,
                amount,
            ),
            ExpandAllContext => self.expand_all_context(),
        }
    }
}

impl AppState {
    pub(super) fn expand_context(
        &mut self,
        hunk_index: usize,
        direction: crate::editor::diff::expansion::ExpandDirection,
        amount: u32,
    ) -> Vec<Effect> {
        use crate::editor::diff::expansion::ExpandDirection;
        use crate::events::ContextDirection;

        if amount == 0 {
            return Vec::new();
        }

        let ctx_direction = match direction {
            ExpandDirection::Above => ContextDirection::Above,
            ExpandDirection::Below => ContextDirection::Below,
        };
        self.dispatch_context_expansion(hunk_index, ctx_direction, amount)
    }

    pub(super) fn expand_all_context(&mut self) -> Vec<Effect> {
        use crate::events::ContextDirection;
        self.dispatch_context_expansion(0, ContextDirection::All, 0)
    }

    pub(super) fn dispatch_context_expansion(
        &mut self,
        hunk_index: usize,
        direction: crate::events::ContextDirection,
        amount: u32,
    ) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };

        let generation = self.workspace.compare_generation.get(&self.store);
        let Some((
            file_index,
            path,
            old_reference,
            new_reference,
            cached_old_lines,
            cached_new_lines,
        )) = self.workspace.active_file.with(&self.store, |af| {
            let active = af.as_ref()?;
            if active.carbon_file.hunks.is_empty() {
                return None;
            }
            Some((
                active.index,
                active.path.clone(),
                active.left_ref.clone(),
                if active.right_ref.is_empty() {
                    active.left_ref.clone()
                } else {
                    active.right_ref.clone()
                },
                active.old_file_lines.clone(),
                active.file_lines.clone(),
            ))
        })
        else {
            return Vec::new();
        };

        if let (Some(old_lines), Some(new_lines)) = (cached_old_lines, cached_new_lines) {
            self.apply_context_expansion(direction, hunk_index, amount, old_lines, new_lines);
            let mut effects = vec![self.invalidate_syntax_epoch_effect()];
            effects.extend(self.request_active_file_syntax_effect());
            return effects;
        }

        vec![
            RepositoryEffect::FetchContextLines(crate::effects::FetchContextLinesRequest {
                repo_path,
                old_reference,
                new_reference,
                path,
                generation,
                file_index,
                hunk_index,
                direction,
                amount,
            })
            .into(),
        ]
    }

    pub(super) fn handle_context_lines_ready(
        &mut self,
        payload: crate::events::ContextLinesReady,
    ) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation.get(&self.store) {
            return Vec::new();
        }

        let matches_active = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref()
                .is_some_and(|a| a.index == payload.file_index && a.path == payload.path)
        });
        if !matches_active {
            return Vec::new();
        }

        let old_lines = Arc::new(payload.old_lines);
        let new_lines = Arc::new(payload.new_lines);
        self.apply_context_expansion(
            payload.direction,
            payload.hunk_index,
            payload.amount,
            old_lines,
            new_lines,
        );
        let mut effects = vec![self.invalidate_syntax_epoch_effect()];
        effects.extend(self.request_active_file_syntax_effect());
        effects
    }

    pub(super) fn apply_context_expansion(
        &mut self,
        direction: crate::events::ContextDirection,
        hunk_index: usize,
        amount: u32,
        old_lines: Arc<Vec<String>>,
        new_lines: Arc<Vec<String>>,
    ) {
        use crate::events::ContextDirection;

        let Some((
            active_index,
            active_path,
            mut carbon_file,
            mut expansion,
            mut carbon_overlays,
            mut token_buffer,
        )) = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref().map(|a| {
                (
                    a.index,
                    a.path.clone(),
                    (*a.carbon_file).clone(),
                    a.carbon_expansion.clone(),
                    a.carbon_overlays.clone(),
                    a.token_buffer.clone(),
                )
            })
        })
        else {
            return;
        };

        hydrate_carbon_full_text(&mut carbon_file, &old_lines, &new_lines);
        match direction {
            ContextDirection::Above => {
                carbon::expand_context(
                    &carbon_file,
                    &mut expansion,
                    carbon::HunkId(hunk_index as u32),
                    carbon::ExpansionDirection::Above,
                    amount,
                );
            }
            ContextDirection::Below => {
                carbon::expand_context(
                    &carbon_file,
                    &mut expansion,
                    carbon::HunkId(hunk_index as u32),
                    carbon::ExpansionDirection::Below,
                    amount,
                );
            }
            ContextDirection::All => {
                let hunk_ids = carbon_file
                    .hunks
                    .iter()
                    .map(|hunk| hunk.id)
                    .collect::<Vec<_>>();
                for hunk_id in hunk_ids {
                    let caps = carbon::expansion_caps(&carbon_file, hunk_id);
                    carbon::expand_context(
                        &carbon_file,
                        &mut expansion,
                        hunk_id,
                        carbon::ExpansionDirection::Above,
                        caps.above,
                    );
                    carbon::expand_context(
                        &carbon_file,
                        &mut expansion,
                        hunk_id,
                        carbon::ExpansionDirection::Below,
                        caps.below,
                    );
                }
            }
        }
        self.workspace.expansions.update(&self.store, |map| {
            map.insert(active_path.clone(), expansion.clone());
        });

        let preserve_change_tokens = carbon_overlays.has_change_tokens();
        carbon_overlays.clear_syntax();
        if !preserve_change_tokens {
            token_buffer.clear();
        }
        let render_doc = build_render_doc_from_carbon(
            &carbon_file,
            active_index,
            &expansion,
            &carbon_overlays,
            &token_buffer,
        );
        let total_lines = new_lines.len() as u32;

        let preserved_scroll = self.editor.scroll_top_px.get(&self.store);

        self.workspace.active_file.update(&self.store, |af| {
            if let Some(active) = af.as_mut() {
                active.carbon_file = Arc::new(carbon_file);
                active.carbon_expansion = expansion;
                active.carbon_overlays = carbon_overlays;
                active.token_buffer = token_buffer;
                active.render_doc = Arc::new(render_doc);
                active.file_line_count = Some(total_lines);
                active.old_file_lines = Some(old_lines);
                active.file_lines = Some(new_lines);
                active.syntax_pending.clear();
                active.syntax_covered.clear();
            }
        });
        self.editor_clear_document();
        self.editor.scroll_top_px.set(&self.store, preserved_scroll);
    }

    pub(super) fn current_hunk_index_from_hover(&self) -> Option<i16> {
        self.editor
            .hovered_hunk_index
            .get(&self.store)
            .or_else(|| self.editor_current_hunk_index().map(|(idx, _)| idx as i16))
    }

    pub(super) fn current_render_line_index_from_hover(&self) -> Option<usize> {
        self.editor
            .hovered_render_line_index
            .get(&self.store)
            .or_else(|| self.editor.hovered_row.get(&self.store))
    }

    pub(super) fn apply_hunk_operation(
        &mut self,
        operation: FileOperation,
        explicit_hunk: Option<i16>,
    ) -> Vec<Effect> {
        tracing::debug!(
            ?operation,
            ?explicit_hunk,
            source = ?self.workspace.source.get(&self.store),
            pending = self.workspace.status_operation_pending.get(&self.store),
            hovered_row = ?self.editor.hovered_row.get(&self.store),
            hovered_hunk_index = ?self.editor.hovered_hunk_index.get(&self.store),
            "apply_hunk_operation: entered"
        );
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            tracing::debug!("apply_hunk_operation: bail: source != Status");
            return Vec::new();
        }
        if !self
            .repository
            .capabilities
            .with(&self.store, |capabilities| {
                capabilities.is_some_and(|capabilities| capabilities.partial_hunk_mutation)
            })
        {
            self.push_error("This repository backend does not support hunk operations.");
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            tracing::debug!("apply_hunk_operation: bail: status_operation_pending=true");
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            tracing::debug!("apply_hunk_operation: bail: no repo_path");
            return Vec::new();
        };
        let Some(bucket) = self.workspace.selected_change_bucket.get(&self.store) else {
            tracing::debug!("apply_hunk_operation: bail: no selected_change_bucket");
            return Vec::new();
        };
        let resolved = explicit_hunk.or_else(|| self.current_hunk_index_from_hover());
        let hunk_index = match resolved {
            Some(idx) if idx >= 0 => idx as usize,
            _ => {
                tracing::debug!(?resolved, "apply_hunk_operation: bail: no hunk_index");
                return Vec::new();
            }
        };

        let patch_text = self.workspace.active_file.with(&self.store, |af| {
            let active = af.as_ref()?;
            patch::format_carbon_hunk_patch(
                &active.carbon_file,
                hunk_index,
                operation != FileOperation::Stage,
            )
        });
        let Some(patch) = patch_text else {
            tracing::debug!(
                hunk_index,
                "apply_hunk_operation: bail: format_hunk_patch returned None"
            );
            return Vec::new();
        };

        tracing::debug!(
            ?operation,
            hunk_index,
            "apply_hunk_operation: dispatching ApplyPatchOperation"
        );
        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        vec![
            RepositoryEffect::ApplyPatchOperation(PatchOperationRequest {
                repo_path,
                patch,
                bucket,
                operation,
            })
            .into(),
        ]
    }

    pub(super) fn toggle_line_selection(&mut self, row: usize, _extend: bool) {
        let line_opt = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref()
                .and_then(|active| active.render_doc.lines.get(row).copied())
        });
        let Some(line) = line_opt else {
            return;
        };
        let kind = line.row_kind();
        if !matches!(
            kind,
            crate::editor::diff::render_doc::RenderRowKind::Added
                | crate::editor::diff::render_doc::RenderRowKind::Removed
                | crate::editor::diff::render_doc::RenderRowKind::Modified
        ) {
            return;
        }
        if line.hunk_index < 0 {
            return;
        }
        let hunk_id = line.hunk_index as u32;
        self.editor.line_selection.update(&self.store, |ls| {
            if line.old_line_index >= 0 {
                ls.toggle(hunk_id, carbon::DiffSide::Old, line.old_line_index as u32);
            }
            if line.new_line_index >= 0 {
                ls.toggle(hunk_id, carbon::DiffSide::New, line.new_line_index as u32);
            }
            ls.last_toggled_row = Some(row);
        });
    }

    pub(super) fn toggle_line_selection_range(&mut self, row: usize, anchor: usize) {
        self.insert_line_selection_range(row, anchor, false);
    }

    pub(super) fn set_line_selection_range(&mut self, row: usize, anchor: usize) {
        self.insert_line_selection_range(row, anchor, true);
    }

    pub(super) fn insert_line_selection_range(
        &mut self,
        row: usize,
        anchor: usize,
        clear_first: bool,
    ) {
        let (start, end) = if row <= anchor {
            (row, anchor)
        } else {
            (anchor, row)
        };
        let lines = self.workspace.active_file.with(&self.store, |af| {
            let Some(active) = af.as_ref() else {
                return Vec::new();
            };
            (start..=end)
                .filter_map(|r| active.render_doc.lines.get(r).copied())
                .collect::<Vec<_>>()
        });
        if lines.is_empty() {
            return;
        }
        // Staging only selects changed lines; in PR review mode a comment can anchor
        // to any line (incl. context), like GitHub.
        let review = self.pull_request_review_enabled();
        self.editor.line_selection.update(&self.store, |ls| {
            if clear_first {
                ls.clear();
            }
            for line in &lines {
                use crate::editor::diff::render_doc::RenderRowKind;
                let kind = line.row_kind();
                if !kind.is_body() || line.hunk_index < 0 {
                    continue;
                }
                if !review
                    && !matches!(
                        kind,
                        RenderRowKind::Added | RenderRowKind::Removed | RenderRowKind::Modified
                    )
                {
                    continue;
                }
                let hunk_id = line.hunk_index as u32;
                if line.old_line_index >= 0 {
                    ls.entries
                        .insert(crate::editor::diff::state::LineSelectionKey {
                            file_path: None,
                            hunk_id,
                            side: carbon::DiffSide::Old,
                            source_index: line.old_line_index as u32,
                        });
                }
                if line.new_line_index >= 0 {
                    ls.entries
                        .insert(crate::editor::diff::state::LineSelectionKey {
                            file_path: None,
                            hunk_id,
                            side: carbon::DiffSide::New,
                            source_index: line.new_line_index as u32,
                        });
                }
            }
            ls.last_toggled_row = Some(row);
        });
    }

    pub(super) fn toggle_current_line_selection(&mut self) {
        let Some(row) = self.current_render_line_index_from_hover() else {
            self.push_error("Move the row cursor to a changed line before selecting lines.");
            return;
        };
        self.toggle_line_selection(row, false);
    }

    pub(super) fn toggle_current_line_selection_range(&mut self) {
        let Some(row) = self.current_render_line_index_from_hover() else {
            self.push_error("Move the row cursor to a changed line before selecting lines.");
            return;
        };
        let anchor = self
            .editor
            .line_selection
            .with(&self.store, |ls| ls.last_toggled_row);
        if let Some(anchor) = anchor {
            self.toggle_line_selection_range(row, anchor);
        } else {
            self.toggle_line_selection(row, false);
        }
    }

    pub(super) fn apply_line_selection_operation(
        &mut self,
        operation: FileOperation,
    ) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Status {
            return Vec::new();
        }
        if self.workspace.status_operation_pending.get(&self.store) {
            return Vec::new();
        }
        if self
            .editor
            .line_selection
            .with(&self.store, |ls| ls.is_empty())
        {
            return Vec::new();
        }
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let Some(bucket) = self.workspace.selected_change_bucket.get(&self.store) else {
            return Vec::new();
        };
        let reverse = operation != FileOperation::Stage;

        let (hunk_indices, selection_snapshot) =
            self.editor.line_selection.with(&self.store, |ls| {
                let indices: Vec<u32> = ls
                    .entries
                    .iter()
                    .map(|key| key.hunk_id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                (indices, ls.clone())
            });

        let patches = self.workspace.active_file.with(&self.store, |af| {
            let Some(active) = af.as_ref() else {
                return Vec::new();
            };
            let mut patches = Vec::new();
            for hunk_idx in hunk_indices {
                let selected = selection_snapshot
                    .selected_lines_for_hunk(hunk_idx)
                    .into_iter()
                    .map(|key| patch::CarbonLineSelection {
                        side: key.side,
                        source_index: key.source_index,
                    })
                    .collect::<Vec<_>>();
                let patch = patch::format_carbon_lines_patch(
                    &active.carbon_file,
                    carbon::u32_to_usize_saturating(hunk_idx),
                    &selected,
                    reverse,
                );
                if let Some(p) = patch {
                    patches.push(p);
                }
            }
            patches
        });

        self.editor
            .line_selection
            .update(&self.store, |ls| ls.clear());

        if patches.is_empty() {
            return Vec::new();
        }

        self.workspace
            .status_operation_pending
            .set(&self.store, true);
        patches
            .into_iter()
            .map(|p| {
                RepositoryEffect::ApplyPatchOperation(PatchOperationRequest {
                    repo_path: repo_path.clone(),
                    patch: p,
                    bucket,
                    operation,
                })
                .into()
            })
            .collect()
    }

    pub(super) fn navigate_to_hunk(&mut self, forward: bool) {
        let current = self.editor.scroll_top_px.get(&self.store);
        let target = self.editor.hunk_positions.with(&self.store, |positions| {
            if positions.is_empty() {
                return None;
            }
            if forward {
                positions
                    .iter()
                    .find(|&&y| y > current)
                    .or_else(|| positions.first())
                    .copied()
            } else {
                positions
                    .iter()
                    .rev()
                    .find(|&&y| y < current)
                    .or_else(|| positions.last())
                    .copied()
            }
        });
        if let Some(y) = target {
            self.editor.scroll_top_px.set(&self.store, y);
            self.editor_clamp_scroll();
        }
    }

    pub(super) fn navigate_to_file(&mut self, forward: bool) -> Vec<Effect> {
        let Some(current) = self.reconcile_selected_file_index_from_path() else {
            return Vec::new();
        };
        let count = self.workspace_file_count();
        if count == 0 {
            return Vec::new();
        }
        let target = if forward {
            current.saturating_add(1).min(count.saturating_sub(1))
        } else {
            current.saturating_sub(1)
        };
        if target == current {
            return Vec::new();
        }

        if self.settings.continuous_scroll {
            return self.select_file(target, true);
        }

        self.select_file(target, true)
    }

    pub(super) fn open_search(&mut self) {
        self.editor.search.open.set(&self.store, true);
        let len = self.editor.search.query.with(&self.store, |q| q.len());
        self.text_edit.cursor.set(&self.store, len);
        self.text_edit.anchor.set(&self.store, 0);
        self.text_edit
            .cursor_moved_at_ms
            .set(&self.store, self.clock_ms);
        self.ui
            .focus
            .set(&self.store, Some(FocusTarget::SearchInput));
        self.editor.focused.set(&self.store, false);
        self.recompute_search_matches();
    }

    pub(super) fn close_search(&mut self) {
        self.editor.search.open.set(&self.store, false);
        self.editor.search.matches.set(&self.store, Arc::default());
        self.editor.search.active_index.set(&self.store, None);
        self.set_focus(Some(FocusTarget::Editor));
    }

    pub(super) fn recompute_search_matches(&mut self) {
        use crate::editor::diff::state::MatchSide;

        self.editor.search.matches.set(&self.store, Arc::default());
        self.editor.search.active_index.set(&self.store, None);

        let query = self
            .editor
            .search
            .query
            .with(&self.store, |q| q.to_ascii_lowercase());
        if query.is_empty() {
            return;
        }

        let new_matches: Vec<SearchMatch> = self.workspace.active_file.with(&self.store, |af| {
            let Some(active_file) = af.as_ref() else {
                return Vec::new();
            };
            let doc = &active_file.render_doc;
            let mut new_matches: Vec<SearchMatch> = Vec::new();
            for (line_idx, line) in doc.lines.iter().enumerate() {
                let line_idx = line_idx as u32;
                if line.left_text.is_valid() {
                    let text = doc.line_text(line.left_text);
                    let lower = text.to_ascii_lowercase();
                    let mut start = 0;
                    while let Some(pos) = lower[start..].find(&query) {
                        let byte_start = (start + pos) as u32;
                        new_matches.push(SearchMatch {
                            line_index: line_idx,
                            byte_start,
                            byte_len: query.len() as u32,
                            side: MatchSide::Left,
                        });
                        start += pos + query.len();
                    }
                }
                if line.right_text.is_valid() {
                    let text = doc.line_text(line.right_text);
                    let lower = text.to_ascii_lowercase();
                    let mut start = 0;
                    while let Some(pos) = lower[start..].find(&query) {
                        let byte_start = (start + pos) as u32;
                        new_matches.push(SearchMatch {
                            line_index: line_idx,
                            byte_start,
                            byte_len: query.len() as u32,
                            side: MatchSide::Right,
                        });
                        start += pos + query.len();
                    }
                }
            }
            new_matches
        });

        let has_matches = !new_matches.is_empty();
        self.editor
            .search
            .matches
            .set(&self.store, Arc::new(new_matches));
        if has_matches {
            self.editor.search.active_index.set(&self.store, Some(0));
        }
    }

    pub(super) fn search_navigate(&mut self, direction: i32) {
        let count = self.editor.search.matches.with(&self.store, |m| m.len());
        if count == 0 {
            return;
        }

        let current = self
            .editor
            .search
            .active_index
            .get(&self.store)
            .unwrap_or(0);
        let next = if direction > 0 {
            if current + 1 >= count { 0 } else { current + 1 }
        } else {
            if current == 0 { count - 1 } else { current - 1 }
        };
        self.editor.search.active_index.set(&self.store, Some(next));
        self.scroll_to_search_match(next);
    }

    pub(super) fn scroll_to_search_match(&mut self, match_index: usize) {
        let y_pos = self
            .editor
            .search_match_y_positions
            .with(&self.store, |v| v.get(match_index).copied());
        let target_y = if let Some(y) = y_pos {
            y
        } else {
            let m = self
                .editor
                .search
                .matches
                .with(&self.store, |m| m.get(match_index).copied());
            let Some(m) = m else {
                return;
            };
            self.estimate_line_y(m.line_index)
        };

        let viewport_h = self.editor.viewport_height_px.get(&self.store);
        let centered = target_y.saturating_sub(viewport_h / 3);
        let max = self.editor_max_scroll_top_px();
        self.editor
            .scroll_top_px
            .set(&self.store, centered.min(max));
    }

    pub(super) fn estimate_line_y(&self, line_index: u32) -> u32 {
        let content_height = self.editor.content_height_px.get(&self.store);
        if content_height == 0 {
            return 0;
        }
        let total_lines = self.workspace.active_file.with(&self.store, |af| {
            af.as_ref()
                .map(|active_file| active_file.render_doc.lines.len() as u32)
                .unwrap_or(0)
        });
        if total_lines == 0 {
            return 0;
        }
        let avg_height = content_height / total_lines;
        line_index.saturating_mul(avg_height)
    }

    /// Clear document-specific editor state (scroll, content, hunks, etc.)
    pub fn editor_clear_document(&mut self) {
        self.editor.doc_generation.set(&self.store, 0);
        self.editor.scroll_top_px.set(&self.store, 0);
        self.editor.content_height_px.set(&self.store, 0);
        self.editor.hovered_row.set(&self.store, None);
        self.editor.hovered_render_line_index.set(&self.store, None);
        self.editor.hovered_hunk_index.set(&self.store, None);
        self.editor.visible_row_start.set(&self.store, None);
        self.editor.visible_row_end.set(&self.store, None);
        self.editor.hunk_positions.set(&self.store, Arc::default());
        self.editor.file_positions.set(&self.store, Arc::default());
        self.editor
            .search_match_y_positions
            .set(&self.store, Arc::default());
        self.editor
            .line_selection
            .update(&self.store, |ls| ls.clear());
        self.editor.text_selection.set(&self.store, None);
        self.context_menu.close();
    }

    pub fn editor_max_scroll_top_px(&self) -> u32 {
        let content = self.editor.content_height_px.get(&self.store);
        let viewport = self.editor.viewport_height_px.get(&self.store);
        content.saturating_sub(viewport.max(1))
    }

    pub fn editor_clamp_scroll(&mut self) {
        let max = self.editor_max_scroll_top_px();
        let cur = self.editor.scroll_top_px.get(&self.store);
        self.editor.scroll_top_px.set(&self.store, cur.min(max));
    }

    pub fn editor_current_hunk_index(&self) -> Option<(usize, usize)> {
        let scroll = self.editor.scroll_top_px.get(&self.store);
        self.editor.hunk_positions.with(&self.store, |positions| {
            if positions.is_empty() {
                return None;
            }
            let idx = positions
                .partition_point(|&y| y <= scroll)
                .saturating_sub(1);
            Some((idx, positions.len()))
        })
    }

    pub(super) fn move_editor_row_cursor(&mut self, delta: i32) {
        let Some(start) = self.editor.visible_row_start.get(&self.store) else {
            return;
        };
        let Some(end) = self.editor.visible_row_end.get(&self.store) else {
            return;
        };
        if start >= end {
            return;
        }
        let max = end.saturating_sub(1);
        let Some(current) = self
            .editor
            .hovered_row
            .get(&self.store)
            .filter(|row| *row >= start && *row <= max)
        else {
            self.editor
                .hovered_row
                .set(&self.store, Some(if delta < 0 { max } else { start }));
            return;
        };
        let next = if delta < 0 {
            current
                .saturating_sub(delta.unsigned_abs() as usize)
                .max(start)
        } else {
            current.saturating_add(delta as usize).min(max)
        };
        self.editor.hovered_row.set(&self.store, Some(next));
    }
}
