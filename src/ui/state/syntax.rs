use std::collections::HashSet;

use crate::actions::SyntaxAction;
use crate::effects::{Effect, LoadFileSyntaxRequest};
use crate::events::SyntaxEvent;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SyntaxInflightKey {
    generation: u64,
    request_id: u64,
}

#[derive(Debug, Default)]
pub(super) struct SyntaxRequestTracker {
    next_request_id: u64,
    epoch: u64,
    inflight: HashSet<SyntaxInflightKey>,
}

impl SyntaxRequestTracker {
    pub(super) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(super) fn next_request_id(&mut self) -> u64 {
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.next_request_id
    }

    pub(super) fn last_request_id(&self) -> u64 {
        self.next_request_id
    }

    pub(super) fn set_last_request_id(&mut self, request_id: u64) {
        self.next_request_id = request_id;
    }

    pub(super) fn outstanding_count(&self, pending_count: usize) -> usize {
        self.inflight.len().max(pending_count)
    }

    pub(super) fn budget_available(&self, pending_count: usize) -> bool {
        self.outstanding_count(pending_count) < MAX_PENDING_SYNTAX_WINDOWS
    }

    pub(super) fn track(&mut self, request: &LoadFileSyntaxRequest) {
        self.inflight.insert(SyntaxInflightKey {
            generation: request.cache_generation,
            request_id: request.request_id,
        });
    }

    pub(super) fn finish(&mut self, generation: u64, request_id: u64) {
        self.inflight.remove(&SyntaxInflightKey {
            generation,
            request_id,
        });
    }

    pub(super) fn invalidate(&mut self) {
        self.inflight.clear();
        self.epoch = self.epoch.saturating_add(1);
    }

    #[cfg(test)]
    pub(super) fn inflight_len(&self) -> usize {
        self.inflight.len()
    }

    #[cfg(test)]
    pub(super) fn insert_inflight(&mut self, generation: u64, request_id: u64) {
        self.inflight.insert(SyntaxInflightKey {
            generation,
            request_id,
        });
    }
}

pub(super) fn reduce_action(_state: &mut AppState, action: SyntaxAction) -> Vec<Effect> {
    match action {}
}

pub(super) fn reduce_event(state: &mut AppState, event: SyntaxEvent) -> Vec<Effect> {
    match event {
        SyntaxEvent::FileSyntaxReady(payload) => state.handle_file_syntax_ready(payload),
        SyntaxEvent::SyntaxPackInstallStarted { language } => {
            state.handle_syntax_pack_install_started(&language);
            Vec::new()
        }
        SyntaxEvent::SyntaxPacksInstalled { languages } => {
            state.handle_syntax_packs_installed(&languages)
        }
        SyntaxEvent::SyntaxPackInstallFinished { language }
        | SyntaxEvent::SyntaxPackInstallFailed { language } => {
            state.handle_syntax_pack_install_finished(&language);
            Vec::new()
        }
    }
}

pub(super) const SYNTAX_INITIAL_ROWS: usize = 200;

pub(super) const SYNTAX_OVERSCAN_ROWS: usize = 160;

pub(super) const MAX_PENDING_SYNTAX_WINDOWS: usize = 96;

pub(super) fn request_syntax_for_active_file(
    active: &mut ActiveFile,
    repo_path: PathBuf,
    generation: u64,
    syntax_epoch: u64,
    window: SyntaxRowWindow,
    request_id: u64,
) -> Option<LoadFileSyntaxRequest> {
    let window = next_missing_syntax_tile(active, window)?;
    if active
        .syntax_pending
        .iter()
        .any(|pending| pending.window.contains(window))
        || active
            .syntax_covered
            .iter()
            .any(|covered| covered.contains(window))
    {
        return None;
    }

    active
        .syntax_pending
        .push(SyntaxPendingWindow { request_id, window });
    Some(LoadFileSyntaxRequest {
        repo_path,
        file_index: active.index,
        path: active.path.clone(),
        carbon_file: active.carbon_file.clone(),
        carbon_expansion: active.carbon_expansion.clone(),
        left_ref: active.left_ref.clone(),
        right_ref: active.right_ref.clone(),
        window,
        request_id,
        cache_generation: generation,
        syntax_epoch,
    })
}

pub(super) fn next_missing_syntax_tile(
    active: &ActiveFile,
    requested: SyntaxRowWindow,
) -> Option<SyntaxRowWindow> {
    let line_count = active.render_doc.lines.len();
    let start = requested.start.min(line_count);
    let end = requested.end.min(line_count);
    if line_count == 0 || end <= start {
        return None;
    }

    let tile_rows = SYNTAX_INITIAL_ROWS.max(1);
    let mut tile_start = (start / tile_rows) * tile_rows;
    while tile_start < end {
        let tile_end = tile_start.saturating_add(tile_rows).min(line_count);
        let candidate = SyntaxRowWindow {
            start: tile_start,
            end: tile_end,
        };
        let already_requested = active
            .syntax_pending
            .iter()
            .any(|pending| pending.window.contains(candidate))
            || active
                .syntax_covered
                .iter()
                .any(|covered| covered.contains(candidate));
        if !already_requested {
            return Some(candidate);
        }
        if tile_end == line_count {
            break;
        }
        tile_start = tile_end;
    }
    None
}

pub(super) fn apply_syntax_tokens_to_file(
    carbon_overlays: &mut CarbonStyleOverlays,
    token_buffer: &mut TokenBuffer,
    updates: &[SyntaxLineTokens],
) {
    for update in updates {
        if let (Some(side), Some(source_index)) = (update.side, update.source_index) {
            if update.tokens.is_empty() {
                continue;
            }
            let range = token_buffer.append(&update.tokens);
            carbon_overlays.insert_syntax(update.hunk_index as u32, side, source_index, range);
        }
    }
}

pub(super) fn active_file_matches_language(
    active: &ActiveFile,
    highlighter: &Highlighter,
    language: &str,
) -> bool {
    !active.carbon_file.is_binary
        && [
            Some(active.path.as_str()),
            active.carbon_file.old_path.as_deref(),
            active.carbon_file.new_path.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|path| {
            highlighter
                .resolve_language(path)
                .is_some_and(|resolved| resolved.name() == language)
        })
}

pub(super) fn file_change_syntax_paths(change: &FileChange) -> Vec<String> {
    let mut paths = Vec::with_capacity(2);
    if let Some(old_path) = change.old_path.as_ref() {
        paths.push(old_path.clone());
    }
    if !paths.iter().any(|path| path == &change.path) {
        paths.push(change.path.clone());
    }
    paths
}

pub(super) fn ensure_syntax_packs_for_file_change_effect(change: &FileChange) -> Effect {
    let mut paths = file_change_syntax_paths(change);
    if paths.len() == 1 {
        return SyntaxEffect::EnsureSyntaxPackForPath {
            path: paths.pop().unwrap_or_else(|| change.path.clone()),
        }
        .into();
    }
    SyntaxEffect::EnsureSyntaxPacksForPaths { paths }.into()
}

pub(super) fn reset_active_file_syntax(active: &mut ActiveFile) {
    active.syntax_pending.clear();
    active.syntax_covered.clear();
    let preserve_change_tokens = active.carbon_overlays.has_change_tokens();
    active.carbon_overlays.clear_syntax();
    if !preserve_change_tokens {
        active.token_buffer.clear();
    }
    active.render_doc = Arc::new(build_render_doc_from_carbon(
        &active.carbon_file,
        active.index,
        &active.carbon_expansion,
        &active.carbon_overlays,
        &active.token_buffer,
    ));
}

pub(super) fn push_syntax_covered_window(
    windows: &mut Vec<SyntaxRowWindow>,
    window: SyntaxRowWindow,
) {
    if window.end <= window.start {
        return;
    }
    windows.push(window);
    windows.sort_by_key(|window| window.start);
    let mut merged: Vec<SyntaxRowWindow> = Vec::with_capacity(windows.len());
    for window in windows.drain(..) {
        if let Some(last) = merged.last_mut()
            && window.start <= last.end
        {
            last.end = last.end.max(window.end);
            continue;
        }
        merged.push(window);
    }
    *windows = merged;
}

pub(super) fn remove_pending_syntax_window(
    pending: &mut Vec<SyntaxPendingWindow>,
    request_id: u64,
    window: SyntaxRowWindow,
) -> bool {
    let Some(index) = pending
        .iter()
        .position(|pending| pending.request_id == request_id && pending.window == window)
    else {
        return false;
    };
    pending.swap_remove(index);
    true
}

impl AppState {
    pub(super) fn syntax_pending_window_count(&self) -> usize {
        let active_count = self.workspace.active_file.with(&self.store, |active| {
            active
                .as_ref()
                .map_or(0, |active| active.syntax_pending.len())
        });
        let cache_count = self.workspace.file_cache.with(&self.store, |files| {
            files
                .values()
                .map(|file| file.syntax_pending.len())
                .sum::<usize>()
        });
        active_count.saturating_add(cache_count)
    }

    pub(super) fn syntax_outstanding_window_count(&self) -> usize {
        self.syntax_requests
            .outstanding_count(self.syntax_pending_window_count())
    }

    pub(super) fn syntax_request_budget_available(&self) -> bool {
        self.syntax_requests
            .budget_available(self.syntax_pending_window_count())
    }

    pub(super) fn track_syntax_request(&mut self, request: &LoadFileSyntaxRequest) {
        self.syntax_requests.track(request);
    }

    pub(super) fn finish_syntax_request(&mut self, generation: u64, request_id: u64) {
        self.syntax_requests.finish(generation, request_id);
    }

    pub(super) fn clear_syntax_pending_windows(&mut self) {
        self.workspace.active_file.update(&self.store, |active| {
            if let Some(active) = active.as_mut() {
                active.syntax_pending.clear();
            }
        });
        self.workspace.file_cache.update(&self.store, |files| {
            for active in files.values_mut() {
                active.syntax_pending.clear();
            }
        });
    }

    pub(super) fn clear_syntax_inflight(&mut self) {
        self.clear_syntax_pending_windows();
        self.syntax_requests.invalidate();
    }

    pub(super) fn syntax_epoch_effect(&self) -> Effect {
        SyntaxEffect::SetFileSyntaxEpoch {
            epoch: self.syntax_requests.epoch(),
        }
        .into()
    }

    pub(super) fn invalidate_syntax_epoch_effect(&mut self) -> Effect {
        self.clear_syntax_inflight();
        self.syntax_epoch_effect()
    }
}

impl AppState {
    pub(super) fn handle_file_syntax_ready(&mut self, payload: FileSyntaxReady) -> Vec<Effect> {
        self.finish_syntax_request(payload.generation, payload.request_id);
        if payload.generation != self.active_syntax_generation() {
            return Vec::new();
        }

        let mut applied_file = None;
        let mut applied_active = false;
        let mut matched_active = false;
        self.workspace.active_file.update(&self.store, |slot| {
            let Some(active) = slot.as_mut() else {
                return;
            };
            if active.index != payload.file_index || active.path != payload.path {
                return;
            }
            matched_active = true;

            if !remove_pending_syntax_window(
                &mut active.syntax_pending,
                payload.request_id,
                payload.window,
            ) {
                return;
            }
            if active
                .syntax_covered
                .iter()
                .any(|covered| covered.contains(payload.window))
            {
                return;
            }
            push_syntax_covered_window(&mut active.syntax_covered, payload.window);
            apply_syntax_tokens_to_file(
                &mut active.carbon_overlays,
                &mut active.token_buffer,
                &payload.tokens,
            );
            active.render_doc = Arc::new(build_render_doc_from_carbon(
                &active.carbon_file,
                active.index,
                &active.carbon_expansion,
                &active.carbon_overlays,
                &active.token_buffer,
            ));
            applied_file = Some(active.clone());
            applied_active = true;
        });
        if matched_active && applied_file.is_none() {
            tracing::debug!(
                file_index = payload.file_index,
                path = %payload.path,
                request_id = payload.request_id,
                "stale active syntax response dropped"
            );
            return Vec::new();
        }

        if applied_file.is_none() {
            self.workspace.file_cache.update(&self.store, |files| {
                let Some(active) = files.get_mut(&payload.file_index) else {
                    return;
                };
                if active.index != payload.file_index || active.path != payload.path {
                    return;
                }

                if !remove_pending_syntax_window(
                    &mut active.syntax_pending,
                    payload.request_id,
                    payload.window,
                ) {
                    return;
                }
                if active
                    .syntax_covered
                    .iter()
                    .any(|covered| covered.contains(payload.window))
                {
                    return;
                }
                push_syntax_covered_window(&mut active.syntax_covered, payload.window);
                apply_syntax_tokens_to_file(
                    &mut active.carbon_overlays,
                    &mut active.token_buffer,
                    &payload.tokens,
                );
                active.render_doc = Arc::new(build_render_doc_from_carbon(
                    &active.carbon_file,
                    active.index,
                    &active.carbon_expansion,
                    &active.carbon_overlays,
                    &active.token_buffer,
                ));
                applied_file = Some(active.clone());
            });
        }

        let Some(active_file) = applied_file else {
            return Vec::new();
        };
        self.cache_active_file(active_file);
        self.viewport_document_cache = None;

        if applied_active {
            self.request_active_file_syntax_effect()
                .into_iter()
                .collect()
        } else {
            Vec::new()
        }
    }

    pub(super) fn handle_syntax_pack_install_started(&mut self, language: &str) {
        self.ui.syntax_pack_installs.update(&self.store, |active| {
            if !active.iter().any(|item| item == language) {
                active.push(language.to_owned());
            }
        });
    }

    pub(super) fn handle_syntax_pack_install_finished(&mut self, language: &str) {
        self.ui
            .syntax_pack_installs
            .update(&self.store, |active| active.retain(|item| item != language));
    }

    pub fn syntax_pack_install_active(&self) -> bool {
        self.ui
            .syntax_pack_installs
            .with(&self.store, |active| !active.is_empty())
    }

    pub(super) fn syntax_pack_warmup_effect_for_compare(
        &self,
        exclude_paths: &[String],
    ) -> Option<Effect> {
        let highlighter = phosphor::Highlighter::new();
        let excluded_languages = exclude_paths
            .iter()
            .filter_map(|path| highlighter.guess_language(Path::new(path)))
            .collect::<HashSet<_>>();
        let active_languages = self.ui.syntax_pack_installs.with(&self.store, |active| {
            active.iter().cloned().collect::<HashSet<_>>()
        });

        self.workspace.compare_output.with(&self.store, |output| {
            let output = output.as_ref()?;
            let mut seen = HashSet::new();
            let mut warmup_paths = Vec::new();
            output.for_each_summary(|_, summary| {
                for path in [summary.paths.old_path(), summary.paths.new_path()]
                    .into_iter()
                    .flatten()
                {
                    let Some(language) = highlighter.guess_language(Path::new(path.as_ref()))
                    else {
                        continue;
                    };
                    if excluded_languages.contains(&language)
                        || active_languages.contains(language.name())
                        || highlighter.is_parser_available(language)
                    {
                        continue;
                    }
                    if seen.insert(language) {
                        warmup_paths.push(path.into_owned());
                    }
                }
            });

            (!warmup_paths.is_empty()).then_some(
                SyntaxEffect::EnsureSyntaxPacksForPaths {
                    paths: warmup_paths,
                }
                .into(),
            )
        })
    }

    pub(super) fn syntax_pack_warmup_effect_for_paths(
        &self,
        paths: &[String],
        exclude_paths: &[String],
    ) -> Option<Effect> {
        let highlighter = phosphor::Highlighter::new();
        let excluded_languages = exclude_paths
            .iter()
            .filter_map(|path| highlighter.guess_language(Path::new(path)))
            .collect::<HashSet<_>>();
        let active_languages = self.ui.syntax_pack_installs.with(&self.store, |active| {
            active.iter().cloned().collect::<HashSet<_>>()
        });

        let mut seen = HashSet::new();
        let mut warmup_paths = Vec::new();
        for path in paths {
            let Some(language) = highlighter.guess_language(Path::new(path)) else {
                continue;
            };
            if excluded_languages.contains(&language)
                || active_languages.contains(language.name())
                || highlighter.is_parser_available(language)
            {
                continue;
            }
            if seen.insert(language) {
                warmup_paths.push(path.clone());
            }
        }

        (!warmup_paths.is_empty()).then_some(
            SyntaxEffect::EnsureSyntaxPacksForPaths {
                paths: warmup_paths,
            }
            .into(),
        )
    }

    pub(super) fn handle_syntax_packs_installed(&mut self, languages: &[String]) -> Vec<Effect> {
        if languages.is_empty() {
            return Vec::new();
        }
        let mut effects = vec![self.invalidate_syntax_epoch_effect()];
        for language in languages {
            effects.extend(self.refresh_active_file_syntax_for_language(language));
            effects.extend(self.request_cached_file_syntax_effects_for_language(language));
        }
        effects
    }

    pub(super) fn refresh_active_file_syntax_for_language(
        &mut self,
        language: &str,
    ) -> Vec<Effect> {
        let highlighter = Highlighter::new();
        let mut refreshed = false;
        self.workspace.active_file.update(&self.store, |slot| {
            let Some(active) = slot.as_mut() else {
                return;
            };
            if !active_file_matches_language(active, &highlighter, language) {
                return;
            }
            reset_active_file_syntax(active);
            refreshed = true;
        });
        if !refreshed {
            return Vec::new();
        }
        self.viewport_document_cache = None;
        self.request_active_file_syntax_effect()
            .into_iter()
            .collect()
    }

    pub(super) fn request_cached_file_syntax_effects_for_language(
        &mut self,
        language: &str,
    ) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let generation = self.active_syntax_generation();
        let syntax_epoch = self.syntax_requests.epoch();
        let mut remaining_budget =
            MAX_PENDING_SYNTAX_WINDOWS.saturating_sub(self.syntax_outstanding_window_count());
        if remaining_budget == 0 {
            return Vec::new();
        }
        let active_key = self.workspace.active_file.with(&self.store, |active| {
            active.as_ref().map(ActiveFile::working_set_key)
        });
        let highlighter = Highlighter::new();
        let mut requests = Vec::new();
        let mut next_request_id = self.syntax_requests.last_request_id();

        self.workspace.file_cache.update(&self.store, |files| {
            for active in files.values_mut() {
                if remaining_budget == 0 {
                    break;
                }
                if active_key
                    .as_ref()
                    .is_some_and(|key| key == &active.working_set_key())
                {
                    continue;
                }
                if !active_file_matches_language(active, &highlighter, language) {
                    continue;
                }
                let line_count = active.render_doc.lines.len();
                if line_count == 0 {
                    continue;
                }
                reset_active_file_syntax(active);
                let window = SyntaxRowWindow {
                    start: 0,
                    end: line_count.min(SYNTAX_INITIAL_ROWS),
                };
                next_request_id = next_request_id.saturating_add(1);
                if let Some(request) = request_syntax_for_active_file(
                    active,
                    repo_path.clone(),
                    generation,
                    syntax_epoch,
                    window,
                    next_request_id,
                ) {
                    requests.push(request);
                    remaining_budget = remaining_budget.saturating_sub(1);
                }
            }
        });
        self.syntax_requests.set_last_request_id(next_request_id);

        requests
            .into_iter()
            .map(|request| {
                self.track_syntax_request(&request);
                SyntaxEffect::LoadFileSyntax(Task {
                    generation,
                    request,
                })
                .into()
            })
            .collect()
    }

    pub(super) fn request_active_file_syntax_effect(&mut self) -> Option<Effect> {
        if !self.syntax_request_budget_available() {
            return None;
        }
        let repo_path = self.compare.repo_path.get(&self.store)?;
        let window = self.desired_syntax_window()?;
        let generation = self.active_syntax_generation();
        let syntax_epoch = self.syntax_requests.epoch();
        let mut request = None;
        let request_id = self.syntax_requests.next_request_id();
        let mut active_to_cache = None;

        self.workspace.active_file.update(&self.store, |active| {
            let Some(active) = active.as_mut() else {
                return;
            };
            if let Some(next_request) = request_syntax_for_active_file(
                active,
                repo_path,
                generation,
                syntax_epoch,
                window,
                request_id,
            ) {
                active_to_cache = Some(active.clone());
                request = Some(next_request);
            }
        });
        if let Some(active_file) = active_to_cache {
            self.cache_active_file(active_file);
        }

        request.map(|request| {
            self.track_syntax_request(&request);
            SyntaxEffect::LoadFileSyntax(Task {
                generation,
                request,
            })
            .into()
        })
    }

    pub(super) fn active_syntax_generation(&self) -> u64 {
        match self.workspace.source.get(&self.store) {
            WorkspaceSource::Status => self.workspace.status_generation.get(&self.store),
            _ => self.workspace.compare_generation.get(&self.store),
        }
    }

    pub(super) fn desired_syntax_window(&self) -> Option<SyntaxRowWindow> {
        let line_count = self.workspace.active_file.with(&self.store, |active| {
            active.as_ref().map(|active| active.render_doc.lines.len())
        })?;
        if line_count == 0 {
            return None;
        }

        if let (Some(start), Some(end)) = (
            self.editor.visible_row_start.get(&self.store),
            self.editor.visible_row_end.get(&self.store),
        ) && end > start
        {
            return Some(SyntaxRowWindow {
                start: start.saturating_sub(SYNTAX_OVERSCAN_ROWS),
                end: end.saturating_add(SYNTAX_OVERSCAN_ROWS).min(line_count),
            });
        }

        let scroll = self.editor.scroll_top_px.get(&self.store) as usize;
        let viewport = self.editor.viewport_height_px.get(&self.store) as usize;
        let approx_row_height = 20usize;
        let start = scroll / approx_row_height;
        let visible = (viewport / approx_row_height).saturating_add(SYNTAX_INITIAL_ROWS);
        Some(SyntaxRowWindow {
            start: start.saturating_sub(SYNTAX_OVERSCAN_ROWS),
            end: start.saturating_add(visible).min(line_count),
        })
    }
}
