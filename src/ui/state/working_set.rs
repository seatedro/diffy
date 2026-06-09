use std::collections::HashSet;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct WorkingSetFileKey {
    index: usize,
    path: String,
    left_ref: String,
    right_ref: String,
}

impl WorkingSetFileKey {
    pub(super) fn new(index: usize, path: String, left_ref: String, right_ref: String) -> Self {
        Self {
            index,
            path,
            left_ref,
            right_ref,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct FileWorkingSet {
    tick: u64,
    protected: HashSet<WorkingSetFileKey>,
}

impl FileWorkingSet {
    pub(super) fn reset(&mut self) {
        self.tick = 0;
        self.protected.clear();
    }

    pub(super) fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.saturating_add(1);
        self.tick
    }

    pub(super) fn protect_slots(&mut self, slots: &[ViewportSlotKey]) {
        self.protected.clear();
        self.protected
            .extend(slots.iter().filter_map(ViewportSlotKey::working_set_key));
    }

    pub(super) fn protected_snapshot(&self) -> HashSet<WorkingSetFileKey> {
        self.protected.clone()
    }
}

pub(super) const COMPARE_WORKING_SET_MAX_FILES: usize = 96;

pub(super) const COMPARE_WORKING_SET_MIN_FILES: usize = 24;

pub(super) const COMPARE_WORKING_SET_BYTE_BUDGET: usize = 64 * 1024 * 1024;

pub(super) const COMPARE_WORKING_SET_PREFETCH_PAGES: u32 = 3;

pub(super) const COMPARE_WORKING_SET_TRAILING_PAGES: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveFileLoading {
    pub index: usize,
    pub path: String,
    pub priority: CompareWorkPriority,
}

#[derive(Debug, Clone)]
pub struct PreparedActiveFile {
    pub carbon_file: carbon::FileDiff,
    pub carbon_expansion: carbon::ExpansionState,
    pub carbon_overlays: CarbonStyleOverlays,
    pub render_doc: Arc<RenderDoc>,
    pub token_buffer: TokenBuffer,
}

pub(super) fn append_active_file_doc(out: &mut RenderDoc, active: &ActiveFile) {
    if active.carbon_file.is_binary {
        out.append_doc(&build_placeholder_render_doc(
            &active.path,
            "Binary file. Diffy only shows text diffs here.",
        ));
    } else {
        out.append_doc(&active.render_doc);
    }
}

pub(super) fn apply_compare_stat_to_active_file(
    active: &mut ActiveFile,
    stat: &CompareFileStat,
) -> bool {
    if active.index != stat.index || active.path != stat.path {
        return false;
    }

    let additions = i32_to_u32_nonnegative(stat.additions);
    let deletions = i32_to_u32_nonnegative(stat.deletions);
    let carbon_file = Arc::make_mut(&mut active.carbon_file);
    if carbon_file.additions == additions
        && carbon_file.deletions == deletions
        && !carbon_file.stats_deferred
    {
        return false;
    }

    carbon_file.additions = additions;
    carbon_file.deletions = deletions;
    carbon_file.stats_deferred = false;
    active.render_doc = Arc::new(build_render_doc_from_carbon(
        &active.carbon_file,
        active.index,
        &active.carbon_expansion,
        &active.carbon_overlays,
        &active.token_buffer,
    ));
    true
}

pub(super) fn hydrate_carbon_full_text(
    file: &mut carbon::FileDiff,
    old_lines: &[String],
    new_lines: &[String],
) {
    if !old_lines.is_empty() {
        file.old_text = Some(carbon::TextStore::from_text(lines_to_text(old_lines)));
    }
    if !new_lines.is_empty() {
        file.new_text = Some(carbon::TextStore::from_text(lines_to_text(new_lines)));
    }
    for block in &mut file.blocks {
        block.old.start = block.old_line_start.saturating_sub(1);
        block.new.start = block.new_line_start.saturating_sub(1);
    }
    file.is_partial = false;
}

pub(super) fn lines_to_text(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut text =
        String::with_capacity(lines.iter().map(|line| line.len().saturating_add(1)).sum());
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text
}

pub(super) fn text_store_estimated_bytes(text: &carbon::TextStore) -> usize {
    text.as_bytes()
        .len()
        .saturating_add(text.line_count() as usize * std::mem::size_of::<u32>())
}

pub(super) fn render_doc_estimated_bytes(doc: &RenderDoc) -> usize {
    doc.text_bytes
        .len()
        .saturating_add(
            doc.style_runs.len() * std::mem::size_of::<crate::editor::diff::render_doc::StyleRun>(),
        )
        .saturating_add(
            doc.lines.len() * std::mem::size_of::<crate::editor::diff::render_doc::RenderLine>(),
        )
        .saturating_add(
            doc.file_metadata
                .iter()
                .map(|meta| {
                    meta.path
                        .len()
                        .saturating_add(meta.old_path.as_ref().map_or(0, String::len))
                })
                .sum::<usize>(),
        )
}

pub(super) fn carbon_file_estimated_bytes(file: &carbon::FileDiff) -> usize {
    file.old_path
        .as_ref()
        .map_or(0, String::len)
        .saturating_add(file.new_path.as_ref().map_or(0, String::len))
        .saturating_add(file.old_oid.as_ref().map_or(0, |oid| oid.0.len()))
        .saturating_add(file.new_oid.as_ref().map_or(0, |oid| oid.0.len()))
        .saturating_add(file.old_mode.as_ref().map_or(0, |mode| mode.0.len()))
        .saturating_add(file.new_mode.as_ref().map_or(0, |mode| mode.0.len()))
        .saturating_add(file.old_text.as_ref().map_or(0, text_store_estimated_bytes))
        .saturating_add(file.new_text.as_ref().map_or(0, text_store_estimated_bytes))
        .saturating_add(file.hunks.len() * std::mem::size_of::<carbon::Hunk>())
        .saturating_add(
            file.hunks
                .iter()
                .map(|hunk| hunk.header.len())
                .sum::<usize>(),
        )
        .saturating_add(file.blocks.len() * std::mem::size_of::<carbon::Block>())
        .saturating_add(
            file.blocks
                .iter()
                .map(|block| {
                    block.old_inline.len() * std::mem::size_of::<carbon::InlineSpan>()
                        + block.new_inline.len() * std::mem::size_of::<carbon::InlineSpan>()
                })
                .sum::<usize>(),
        )
}

pub(super) fn line_vec_estimated_bytes(lines: &Arc<Vec<String>>) -> usize {
    lines
        .iter()
        .map(|line| {
            std::mem::size_of::<String>()
                .saturating_add(line.len())
                .saturating_add(1)
        })
        .fold(0usize, usize::saturating_add)
}

pub(super) fn i32_to_u32_nonnegative(value: i32) -> u32 {
    u32::try_from(value).unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct ActiveFile {
    pub index: usize,
    pub path: String,
    pub carbon_file: Arc<carbon::FileDiff>,
    pub carbon_expansion: carbon::ExpansionState,
    pub carbon_overlays: CarbonStyleOverlays,
    pub render_doc: Arc<RenderDoc>,
    pub token_buffer: TokenBuffer,
    pub left_ref: String,
    pub right_ref: String,
    pub file_line_count: Option<u32>,
    pub old_file_lines: Option<Arc<Vec<String>>>,
    pub file_lines: Option<Arc<Vec<String>>>,
    pub syntax_pending: Vec<SyntaxPendingWindow>,
    pub syntax_covered: Vec<SyntaxRowWindow>,
    pub last_used_tick: u64,
}

impl ActiveFile {
    pub(super) fn working_set_key(&self) -> WorkingSetFileKey {
        WorkingSetFileKey::new(
            self.index,
            self.path.clone(),
            self.left_ref.clone(),
            self.right_ref.clone(),
        )
    }

    pub(super) fn working_set_bytes(&self) -> usize {
        self.path
            .len()
            .saturating_add(self.left_ref.len())
            .saturating_add(self.right_ref.len())
            .saturating_add(render_doc_estimated_bytes(&self.render_doc))
            .saturating_add(
                self.token_buffer
                    .len()
                    .saturating_mul(std::mem::size_of::<crate::core::text::DiffTokenSpan>()),
            )
            .saturating_add(carbon_file_estimated_bytes(&self.carbon_file))
            .saturating_add(
                self.old_file_lines
                    .as_ref()
                    .map_or(0, line_vec_estimated_bytes),
            )
            .saturating_add(self.file_lines.as_ref().map_or(0, line_vec_estimated_bytes))
    }
}

pub(crate) fn prepare_active_file(
    file_index: usize,
    carbon_file: &carbon::FileDiff,
) -> PreparedActiveFile {
    let token_buffer = TokenBuffer::default();
    let carbon_overlays = CarbonStyleOverlays::default();

    let carbon_expansion = carbon::ExpansionState::default();
    let render_doc = build_render_doc_from_carbon(
        carbon_file,
        file_index,
        &carbon_expansion,
        &carbon_overlays,
        &token_buffer,
    );
    PreparedActiveFile {
        carbon_file: carbon_file.clone(),
        carbon_expansion,
        carbon_overlays,
        render_doc: Arc::new(render_doc),
        token_buffer,
    }
}

impl AppState {
    pub(super) fn build_active_file(
        &self,
        index: usize,
        path: String,
        prepared: PreparedActiveFile,
        left_ref: String,
        right_ref: String,
    ) -> ActiveFile {
        ActiveFile {
            index,
            path,
            carbon_file: Arc::new(prepared.carbon_file),
            carbon_expansion: prepared.carbon_expansion.clone(),
            carbon_overlays: prepared.carbon_overlays,
            render_doc: prepared.render_doc,
            token_buffer: prepared.token_buffer,
            left_ref,
            right_ref,
            file_line_count: None,
            old_file_lines: None,
            file_lines: None,
            syntax_pending: Vec::new(),
            syntax_covered: Vec::new(),
            last_used_tick: 0,
        }
    }

    pub(super) fn clear_file_cache(&mut self) {
        self.workspace.file_cache.set(&self.store, HashMap::new());
        self.workspace
            .file_cache_loading
            .set(&self.store, HashMap::new());
        self.viewport_document_cache = None;
        self.last_virtual_scroll_top_px = None;
        self.file_working_set.reset();
    }

    pub(super) fn next_file_working_set_tick(&mut self) -> u64 {
        self.file_working_set.next_tick()
    }

    pub(super) fn protect_working_set_slots(&mut self, slots: &[ViewportSlotKey]) {
        self.file_working_set.protect_slots(slots);
    }

    pub(super) fn cache_active_file(&mut self, mut active_file: ActiveFile) -> ActiveFile {
        let index = active_file.index;
        active_file.last_used_tick = self.next_file_working_set_tick();
        let cached = active_file.clone();
        self.workspace.file_cache.update(&self.store, |files| {
            files.insert(index, cached);
        });
        self.workspace
            .file_cache_loading
            .update(&self.store, |files| {
                files.remove(&index);
            });
        self.trim_file_working_set();
        active_file
    }

    pub(super) fn touch_viewport_slot(&mut self, key: &ViewportSlotKey) {
        let tick = self.next_file_working_set_tick();
        self.workspace.active_file.update(&self.store, |slot| {
            if let Some(active) = slot.as_mut()
                && active.index == key.index
                && active.path == key.path
                && active.left_ref == key.left_ref
                && active.right_ref == key.right_ref
            {
                active.last_used_tick = tick;
            }
        });
        self.workspace.file_cache.update(&self.store, |files| {
            if let Some(active) = files.get_mut(&key.index)
                && active.index == key.index
                && active.path == key.path
                && active.left_ref == key.left_ref
                && active.right_ref == key.right_ref
            {
                active.last_used_tick = tick;
            }
        });
    }

    pub(super) fn trim_file_working_set(&mut self) {
        let mut keep = self.file_working_set.protected_snapshot();
        if let Some(active) = self.workspace.active_file.with(&self.store, |active| {
            active.as_ref().map(ActiveFile::working_set_key)
        }) {
            keep.insert(active);
        }
        if let Some(cache) = self.viewport_document_cache.as_ref() {
            keep.extend(
                cache
                    .key
                    .slots
                    .iter()
                    .filter_map(ViewportSlotKey::working_set_key),
            );
        }

        self.workspace.file_cache.update(&self.store, |files| {
            let mut bytes = files
                .values()
                .map(ActiveFile::working_set_bytes)
                .fold(0usize, usize::saturating_add);
            if files.len() <= COMPARE_WORKING_SET_MAX_FILES
                && bytes <= COMPARE_WORKING_SET_BYTE_BUDGET
            {
                return;
            }

            let mut victims = files
                .iter()
                .filter(|(_, file)| !keep.contains(&file.working_set_key()))
                .map(|(index, file)| (*index, file.last_used_tick))
                .collect::<Vec<_>>();
            victims.sort_by_key(|(_, last_used)| *last_used);

            for (index, _) in victims {
                if files.len() <= COMPARE_WORKING_SET_MAX_FILES
                    && (files.len() <= COMPARE_WORKING_SET_MIN_FILES
                        || bytes <= COMPARE_WORKING_SET_BYTE_BUDGET)
                {
                    break;
                }
                if let Some(file) = files.remove(&index) {
                    bytes = bytes.saturating_sub(file.working_set_bytes());
                }
            }
        });
    }

    pub(super) fn cached_file_at(&self, index: usize) -> Option<ActiveFile> {
        self.workspace
            .file_cache
            .with(&self.store, |files| files.get(&index).cloned())
    }

    pub(crate) fn viewport_file_snapshot(&self, index: usize) -> Option<ActiveFile> {
        if let Some(active) = self.workspace.active_file.with(&self.store, |file| {
            file.as_ref()
                .filter(|active| active.index == index)
                .cloned()
        }) {
            return Some(active);
        }
        self.cached_file_at(index)
    }

    pub(super) fn file_load_pending_priority(
        &self,
        index: usize,
        path: &str,
    ) -> Option<CompareWorkPriority> {
        self.workspace
            .active_file_loading
            .with(&self.store, |loading| {
                loading
                    .as_ref()
                    .filter(|loading| loading.index == index && loading.path == path)
                    .map(|loading| loading.priority)
            })
            .or_else(|| {
                self.workspace
                    .file_cache_loading
                    .with(&self.store, |loading| {
                        loading
                            .get(&index)
                            .filter(|loading| loading.path == path)
                            .map(|loading| loading.priority)
                    })
            })
    }

    pub(super) fn should_enqueue_file_load(
        &self,
        index: usize,
        path: &str,
        priority: CompareWorkPriority,
    ) -> bool {
        self.file_load_pending_priority(index, path)
            .is_none_or(|pending| priority.rank() > pending.rank())
    }

    pub(super) fn mark_file_cache_loading(
        &mut self,
        index: usize,
        path: String,
        priority: CompareWorkPriority,
    ) {
        self.workspace
            .file_cache_loading
            .update(&self.store, |loading| {
                loading.insert(
                    index,
                    ActiveFileLoading {
                        index,
                        path,
                        priority,
                    },
                );
            });
    }

    pub(super) fn clear_file_cache_loading(&mut self, index: usize) {
        self.workspace
            .file_cache_loading
            .update(&self.store, |loading| {
                loading.remove(&index);
            });
    }

    pub(super) fn cached_compare_file_at(&self, index: usize, path: &str) -> Option<ActiveFile> {
        let (left_ref, right_ref) = self.compare_refs();
        if let Some(active_file) = self.workspace.active_file.with(&self.store, |file| {
            file.as_ref()
                .filter(|file| {
                    file.index == index
                        && file.path == path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .cloned()
        }) {
            return Some(active_file);
        }
        self.cached_file_at(index).filter(|file| {
            file.index == index
                && file.path == path
                && file.left_ref == left_ref
                && file.right_ref == right_ref
        })
    }

    pub(super) fn cached_status_file_at(
        &self,
        index: usize,
        change: &FileChange,
    ) -> Option<ActiveFile> {
        let (left_ref, right_ref) = self.status_refs_for_bucket(change.bucket);
        if let Some(active_file) = self.workspace.active_file.with(&self.store, |file| {
            file.as_ref()
                .filter(|file| {
                    file.index == index
                        && file.path == change.path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .cloned()
        }) {
            return Some(active_file);
        }
        self.cached_file_at(index).filter(|file| {
            file.index == index
                && file.path == change.path
                && file.left_ref == left_ref
                && file.right_ref == right_ref
        })
    }

    pub(super) fn cache_compare_file_from_output(
        &mut self,
        index: usize,
        path: &str,
    ) -> Option<ActiveFile> {
        let carbon_file = self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .and_then(|output| output.carbon.files.get(index))
                .filter(|file| file.path() == path)
                .filter(|file| !(file.is_partial && file.hunks.is_empty()))
                .cloned()
        })?;
        let prepared = prepare_active_file(index, &carbon_file);
        let (left_ref, right_ref) = self.compare_refs();
        let active_file =
            self.build_active_file(index, path.to_owned(), prepared, left_ref, right_ref);
        let active_file = self.cache_active_file(active_file);
        Some(active_file)
    }

    pub(super) fn install_compare_active_file(
        &mut self,
        index: usize,
        path: String,
        prepared: PreparedActiveFile,
    ) {
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
        let active_file =
            self.build_active_file(index, path.clone(), prepared, left_ref, right_ref);
        let active_file = self.cache_active_file(active_file);
        let stats = CompareFileStat {
            index,
            path: path.clone(),
            additions: u32_to_i32_saturating(active_file.carbon_file.additions),
            deletions: u32_to_i32_saturating(active_file.carbon_file.deletions),
        };

        self.workspace
            .selected_file_index
            .set(&self.store, Some(index));
        self.workspace
            .selected_file_path
            .set(&self.store, Some(path));
        self.workspace.selected_change_bucket.set(&self.store, None);
        self.workspace.active_file_loading.set(&self.store, None);
        self.workspace
            .active_file
            .set(&self.store, Some(active_file));
        self.apply_compare_file_stats(&[stats]);
        // The first real file has landed — tear down the progress panel.
        // Subsequent file loads use the sidebar row spinner, not this.
        self.workspace.compare_progress.set(&self.store, None);
        self.editor_clear_document();
        self.editor
            .line_selection
            .update(&self.store, |ls| ls.clear());
        if self.editor.search.open.get(&self.store) {
            self.recompute_search_matches();
        }
        self.file_list.hovered_index.set(&self.store, Some(index));
    }
}

impl AppState {
    pub(super) fn prefetch_compare_working_set(
        &mut self,
        render_start_index: usize,
        render_end_index: usize,
        direction: ScrollDirection,
        viewport_height_px: u32,
    ) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::Compare {
            return Vec::new();
        }
        let count = self.workspace_file_count();
        if count == 0 {
            return Vec::new();
        }

        let forward_pages = if direction == ScrollDirection::Forward {
            COMPARE_WORKING_SET_PREFETCH_PAGES
        } else {
            COMPARE_WORKING_SET_TRAILING_PAGES
        };
        let backward_pages = if direction == ScrollDirection::Backward {
            COMPARE_WORKING_SET_PREFETCH_PAGES
        } else {
            COMPARE_WORKING_SET_TRAILING_PAGES
        };

        let mut effects = Vec::new();
        effects.extend(self.prefetch_compare_files_forward(
            render_end_index,
            viewport_height_px.saturating_mul(forward_pages).max(1),
        ));
        effects.extend(self.prefetch_compare_files_backward(
            render_start_index,
            viewport_height_px.saturating_mul(backward_pages).max(1),
        ));
        effects
    }

    pub(super) fn prefetch_compare_files_forward(
        &mut self,
        start_index: usize,
        target_height: u32,
    ) -> Vec<Effect> {
        let count = self.workspace_file_count();
        let mut effects = Vec::new();
        let mut accumulated = 0_u32;
        let mut index = start_index;
        while index < count && accumulated < target_height {
            if let Some(path) = self.workspace_file_path_at(index) {
                effects.extend(self.ensure_compare_file_cached_for_viewport(
                    index,
                    &path,
                    CompareWorkPriority::Overscan,
                ));
            }
            accumulated =
                accumulated.saturating_add(self.viewport_file_scroll_height_px(index).max(1));
            index += 1;
        }
        effects
    }

    pub(super) fn prefetch_compare_files_backward(
        &mut self,
        start_index: usize,
        target_height: u32,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        let mut accumulated = 0_u32;
        let mut index = start_index;
        while index > 0 && accumulated < target_height {
            index -= 1;
            if let Some(path) = self.workspace_file_path_at(index) {
                effects.extend(self.ensure_compare_file_cached_for_viewport(
                    index,
                    &path,
                    CompareWorkPriority::Overscan,
                ));
            }
            accumulated =
                accumulated.saturating_add(self.viewport_file_scroll_height_px(index).max(1));
        }
        effects
    }
}
