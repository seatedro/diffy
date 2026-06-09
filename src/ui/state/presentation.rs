//! Presentation-layer caches for the diff viewport: the continuous-scroll
//! virtual document model, per-slot render-doc cache, file height index, and
//! scroll anchoring. Pure code motion from `mod.rs`.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportDocumentMode {
    Single,
    Continuous,
}

#[derive(Debug, Clone)]
pub struct ViewportDocument {
    pub doc: Arc<RenderDoc>,
    pub mode: ViewportDocumentMode,
    pub generation: u64,
    pub start_index: usize,
    pub start_offset_px: u32,
    pub scroll_top_px: u32,
    pub slot_indices: Vec<usize>,
    pub slot_item_ids: Vec<VirtualDiffItemId>,
    pub stream_items: Vec<VirtualDiffStreamItem>,
    pub slot_loading: Vec<bool>,
    pub path: String,
}

impl ViewportDocument {
    pub fn single(doc: Arc<RenderDoc>, generation: u64, file_index: usize, path: String) -> Self {
        Self {
            doc,
            mode: ViewportDocumentMode::Single,
            generation,
            start_index: file_index,
            start_offset_px: 0,
            scroll_top_px: 0,
            slot_indices: vec![file_index],
            slot_item_ids: vec![VirtualDiffItemId::file(
                WorkspaceSource::None,
                generation,
                file_index,
            )],
            stream_items: Vec::new(),
            slot_loading: vec![false],
            path,
        }
    }

    pub fn is_continuous(&self) -> bool {
        self.mode == ViewportDocumentMode::Continuous
    }

    pub fn insert_stream_item(&mut self, item: VirtualDiffStreamItem) {
        let index = self
            .stream_items
            .partition_point(|existing| existing.sort_key <= item.sort_key);
        self.stream_items.insert(index, item);
    }
}

pub(super) fn virtual_stream_item_kind(
    slot: &ViewportSlotKey,
    line: &RenderLine,
) -> Option<VirtualDiffItemKind> {
    match line.row_kind() {
        RenderRowKind::FileHeader => Some(VirtualDiffItemKind::FileHeader),
        RenderRowKind::HunkSeparator
            if matches!(slot.kind, ViewportSlotKind::Loading) || line.hunk_index < 0 =>
        {
            Some(VirtualDiffItemKind::LoadingPlaceholder)
        }
        RenderRowKind::HunkSeparator => Some(VirtualDiffItemKind::Hunk),
        RenderRowKind::Context
        | RenderRowKind::Added
        | RenderRowKind::Removed
        | RenderRowKind::Modified => Some(VirtualDiffItemKind::DiffRow),
        RenderRowKind::Block => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualDiffItemKind {
    File,
    FileHeader,
    Hunk,
    DiffRow,
    ReviewThread,
    ReviewComment,
    Composer,
    LoadingPlaceholder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualDiffItemId {
    pub source: WorkspaceSource,
    pub generation: u64,
    pub kind: VirtualDiffItemKind,
    pub index: usize,
    pub ordinal: u32,
    pub stable_key: u64,
}

impl VirtualDiffItemId {
    pub(super) fn file(source: WorkspaceSource, generation: u64, index: usize) -> Self {
        Self {
            source,
            generation,
            kind: VirtualDiffItemKind::File,
            index,
            ordinal: 0,
            stable_key: 0,
        }
    }

    pub fn new(
        source: WorkspaceSource,
        generation: u64,
        kind: VirtualDiffItemKind,
        index: usize,
        ordinal: u32,
        stable_key: u64,
    ) -> Self {
        Self {
            source,
            generation,
            kind,
            index,
            ordinal,
            stable_key,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualDiffStreamItem {
    pub id: VirtualDiffItemId,
    pub sort_key: u64,
    pub estimated_height_px: u32,
    pub measured_height_px: Option<u32>,
}

impl VirtualDiffStreamItem {
    pub fn new(
        id: VirtualDiffItemId,
        sort_key: u64,
        estimated_height_px: u32,
        measured_height_px: Option<u32>,
    ) -> Self {
        Self {
            id,
            sort_key,
            estimated_height_px,
            measured_height_px,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportAnchorBias {
    PreserveTop,
    PreserveBottom,
    FollowEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportAnchor {
    pub item_id: VirtualDiffItemId,
    pub intra_item_offset_px: u32,
    pub bias: ViewportAnchorBias,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ViewportSlotKey {
    pub(super) source: WorkspaceSource,
    pub(super) index: usize,
    pub(super) path: String,
    pub(super) left_ref: String,
    pub(super) right_ref: String,
    pub(super) kind: ViewportSlotKind,
}

impl ViewportSlotKey {
    pub(super) fn working_set_key(&self) -> Option<WorkingSetFileKey> {
        if self.source == WorkspaceSource::None {
            return None;
        }
        Some(WorkingSetFileKey::new(
            self.index,
            self.path.clone(),
            self.left_ref.clone(),
            self.right_ref.clone(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ViewportSlotKind {
    Text {
        line_count: usize,
        text_len: usize,
        style_run_count: usize,
        syntax_covered_count: usize,
    },
    Binary,
    Loading,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ViewportDocumentKey {
    pub(super) source: WorkspaceSource,
    pub(super) generation: u64,
    pub(super) start_index: usize,
    pub(super) slots: Vec<ViewportSlotKey>,
}

#[derive(Debug, Clone)]
pub(super) struct ViewportDocumentCache {
    pub(super) key: ViewportDocumentKey,
    pub(super) doc: Arc<RenderDoc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScrollDirection {
    Backward,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxPendingWindow {
    pub(super) request_id: u64,
    pub(super) window: SyntaxRowWindow,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidebarWidthCache {
    pub compare_generation: u64,
    pub ui_scale_pct: u16,
    pub intrinsic_width_px: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportScrollbarMetrics {
    pub content_height_px: u32,
    pub viewport_height_px: u32,
    pub scroll_top_px: u32,
    pub max_scroll_top_px: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportScrollbarDragState {
    pub metrics: ViewportScrollbarMetrics,
    pub file_heights_px: Vec<u32>,
}
pub(super) const FILE_HEIGHT_SPARSE_MIN_COUNT: usize = 4096;

#[derive(Debug)]
pub(super) enum FileHeightIndex {
    Empty,
    Dense {
        heights: Vec<u32>,
        tree: Vec<u32>,
    },
    Sparse {
        count: usize,
        default_height: u32,
        total: u64,
        overrides: BTreeMap<usize, u32>,
        tree: Vec<u64>,
    },
}

impl Default for FileHeightIndex {
    fn default() -> Self {
        Self::Empty
    }
}

impl FileHeightIndex {
    pub(super) fn rebuild(&mut self, heights: Vec<u32>) {
        if heights.is_empty() {
            self.clear();
            return;
        }

        if let Some((default_height, overrides, total)) = sparse_height_index_parts(&heights) {
            let mut tree = vec![0; heights.len() + 1];
            for (index, height) in heights.iter().copied().enumerate() {
                height_tree_add(&mut tree, index, u64::from(height));
            }
            *self = Self::Sparse {
                count: heights.len(),
                default_height,
                total,
                overrides,
                tree,
            };
            return;
        }

        let mut tree = vec![0; heights.len() + 1];
        for (index, height) in heights.iter().copied().enumerate() {
            dense_tree_add(&mut tree, index, height);
        }
        *self = Self::Dense { heights, tree };
    }

    pub(super) fn clear(&mut self) {
        *self = Self::Empty;
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Dense { heights, .. } => heights.len(),
            Self::Sparse { count, .. } => *count,
        }
    }

    pub(super) fn total_u64(&self) -> u64 {
        match self {
            Self::Empty => 0,
            Self::Dense { heights, .. } => self.prefix_u64(heights.len()),
            Self::Sparse { total, .. } => *total,
        }
    }

    pub(super) fn total_u32(&self) -> u32 {
        self.total_u64().min(u64::from(u32::MAX)) as u32
    }

    pub(super) fn prefix_u32(&self, index: usize) -> u32 {
        self.prefix_u64(index).min(u64::from(u32::MAX)) as u32
    }

    pub(super) fn update(&mut self, index: usize, height: u32) {
        match self {
            Self::Empty => {}
            Self::Dense { heights, tree } => {
                if index >= heights.len() {
                    return;
                }
                let old = heights[index];
                if old == height {
                    return;
                }
                heights[index] = height;
                if height >= old {
                    dense_tree_add(tree, index, height - old);
                } else {
                    dense_tree_sub(tree, index, old - height);
                }
            }
            Self::Sparse {
                count,
                default_height,
                total,
                overrides,
                tree,
            } => {
                if index >= *count {
                    return;
                }
                let old = overrides.get(&index).copied().unwrap_or(*default_height);
                if old == height {
                    return;
                }
                if height == *default_height {
                    overrides.remove(&index);
                } else {
                    overrides.insert(index, height);
                }
                *total = total
                    .saturating_sub(u64::from(old))
                    .saturating_add(u64::from(height));
                if height >= old {
                    height_tree_add(tree, index, u64::from(height - old));
                } else {
                    height_tree_sub(tree, index, u64::from(old - height));
                }
                if overrides.len() > *count / 4 {
                    self.promote_sparse_to_dense();
                }
            }
        }
    }

    pub(super) fn locate(&self, target_px: u32) -> Option<(usize, u32)> {
        match self {
            Self::Empty => None,
            Self::Dense { heights, tree } => locate_dense_height(heights, tree, target_px),
            Self::Sparse {
                count, total, tree, ..
            } => locate_sparse_height(self, *count, *total, tree, target_px),
        }
    }

    pub(super) fn prefix_u64(&self, index: usize) -> u64 {
        match self {
            Self::Empty => 0,
            Self::Dense { heights, tree } => dense_prefix_u64(heights, tree, index),
            Self::Sparse { count, tree, .. } => height_tree_prefix_u64(tree, index.min(*count)),
        }
    }

    pub(super) fn height_at(&self, index: usize) -> u32 {
        match self {
            Self::Empty => 0,
            Self::Dense { heights, .. } => heights.get(index).copied().unwrap_or(0),
            Self::Sparse {
                count,
                default_height,
                overrides,
                ..
            } => {
                if index >= *count {
                    0
                } else {
                    overrides.get(&index).copied().unwrap_or(*default_height)
                }
            }
        }
    }

    pub(super) fn promote_sparse_to_dense(&mut self) {
        let Self::Sparse {
            count,
            default_height,
            overrides,
            ..
        } = self
        else {
            return;
        };
        let mut heights = vec![*default_height; *count];
        for (index, height) in overrides.iter() {
            if let Some(slot) = heights.get_mut(*index) {
                *slot = *height;
            }
        }
        self.rebuild(heights);
    }
}

#[derive(Debug, Default)]
pub(super) struct VirtualDiffDocument {
    pub(super) source: WorkspaceSource,
    pub(super) generation: u64,
    pub(super) file_count: usize,
    pub(super) height_index: FileHeightIndex,
}

impl VirtualDiffDocument {
    pub(super) fn sync_identity(
        &mut self,
        source: WorkspaceSource,
        generation: u64,
        file_count: usize,
    ) -> bool {
        let changed =
            self.source != source || self.generation != generation || self.file_count != file_count;
        if changed {
            self.source = source;
            self.generation = generation;
            self.file_count = file_count;
            self.height_index.clear();
        }
        changed
    }

    pub(super) fn clear(&mut self) {
        self.source = WorkspaceSource::None;
        self.generation = 0;
        self.file_count = 0;
        self.height_index.clear();
    }

    pub(super) fn rebuild_heights(&mut self, heights: Vec<u32>) {
        self.file_count = heights.len();
        self.height_index.rebuild(heights);
    }

    pub(super) fn item_id(&self, index: usize) -> Option<VirtualDiffItemId> {
        (index < self.file_count)
            .then(|| VirtualDiffItemId::file(self.source, self.generation, index))
    }

    pub(super) fn anchor_is_current(&self, anchor: ViewportAnchor) -> bool {
        anchor.item_id.source == self.source
            && anchor.item_id.generation == self.generation
            && anchor.item_id.kind == VirtualDiffItemKind::File
            && anchor.item_id.index < self.file_count
    }

    pub(super) fn len(&self) -> usize {
        self.height_index.len()
    }

    pub(super) fn total_u32(&self) -> u32 {
        self.height_index.total_u32()
    }

    pub(super) fn prefix_u32(&self, index: usize) -> u32 {
        self.height_index.prefix_u32(index)
    }

    pub(super) fn locate(&self, target_px: u32) -> Option<(usize, u32)> {
        self.height_index.locate(target_px)
    }

    pub(super) fn height_at(&self, index: usize) -> u32 {
        self.height_index.height_at(index)
    }

    pub(super) fn update_height(&mut self, index: usize, height: u32) {
        self.height_index.update(index, height);
    }
}

#[derive(Debug, Default)]
pub(super) struct VirtualScrollModel {
    pub(super) anchor: Option<ViewportAnchor>,
}

impl VirtualScrollModel {
    pub(super) fn clear(&mut self) {
        self.anchor = None;
    }

    pub(super) fn set_anchor(&mut self, anchor: ViewportAnchor) {
        self.anchor = Some(anchor);
    }
}

const VIRTUAL_STREAM_SORT_STRIDE: u64 = 1024;
const VIRTUAL_STREAM_ROW_OFFSET: u64 = 512;
const VIRTUAL_STREAM_BLOCK_BELOW_OFFSET: u64 = 768;

pub(super) fn virtual_row_sort_key(line_index: usize) -> u64 {
    (line_index as u64)
        .saturating_mul(VIRTUAL_STREAM_SORT_STRIDE)
        .saturating_add(VIRTUAL_STREAM_ROW_OFFSET)
}

pub fn virtual_block_below_sort_key(anchor_line_index: u32, block_order: usize) -> u64 {
    u64::from(anchor_line_index)
        .saturating_mul(VIRTUAL_STREAM_SORT_STRIDE)
        .saturating_add(VIRTUAL_STREAM_BLOCK_BELOW_OFFSET)
        .saturating_add(block_order.min(255) as u64)
}

pub fn stable_virtual_key(text: &str) -> u64 {
    let mut key = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.as_bytes() {
        key ^= u64::from(*byte);
        key = key.wrapping_mul(0x100_0000_01b3);
    }
    key
}

pub(super) fn estimated_virtual_item_height_px(kind: VirtualDiffItemKind) -> u32 {
    match kind {
        VirtualDiffItemKind::File => 192,
        VirtualDiffItemKind::FileHeader => 40,
        VirtualDiffItemKind::Hunk => 28,
        VirtualDiffItemKind::DiffRow => 24,
        VirtualDiffItemKind::ReviewThread => 160,
        VirtualDiffItemKind::ReviewComment => 96,
        VirtualDiffItemKind::Composer => 248,
        VirtualDiffItemKind::LoadingPlaceholder => 48,
    }
}

pub(super) fn virtual_row_stable_key(line: &RenderLine, local_ordinal: u32) -> u64 {
    let mut key = u64::from(line.kind);
    key = key
        .wrapping_mul(1_099_511_628_211)
        .wrapping_add(line.hunk_index as i64 as u64);
    key = key
        .wrapping_mul(1_099_511_628_211)
        .wrapping_add(u64::from(line.old_line_no));
    key = key
        .wrapping_mul(1_099_511_628_211)
        .wrapping_add(u64::from(line.new_line_no));
    key = key
        .wrapping_mul(1_099_511_628_211)
        .wrapping_add(line.line_index as i64 as u64);
    key.wrapping_mul(1_099_511_628_211)
        .wrapping_add(u64::from(local_ordinal))
}

fn sparse_height_index_parts(heights: &[u32]) -> Option<(u32, BTreeMap<usize, u32>, u64)> {
    if heights.len() < FILE_HEIGHT_SPARSE_MIN_COUNT {
        return None;
    }
    let default_height = most_common_height(heights);
    let mut overrides = BTreeMap::new();
    let mut total = 0_u64;
    for (index, height) in heights.iter().copied().enumerate() {
        total = total.saturating_add(u64::from(height));
        if height != default_height {
            overrides.insert(index, height);
        }
    }

    if overrides.len() <= heights.len() / 4 {
        Some((default_height, overrides, total))
    } else {
        None
    }
}

fn most_common_height(heights: &[u32]) -> u32 {
    let mut counts: HashMap<u32, usize> = HashMap::new();
    let mut best_height = heights[0];
    let mut best_count = 0;
    for height in heights {
        let count = counts
            .entry(*height)
            .and_modify(|count| *count += 1)
            .or_insert(1);
        if *count > best_count {
            best_height = *height;
            best_count = *count;
        }
    }
    best_height
}

fn dense_tree_add(tree: &mut [u32], index: usize, delta: u32) {
    let mut idx = index + 1;
    while idx < tree.len() {
        tree[idx] = tree[idx].saturating_add(delta);
        idx += idx & idx.wrapping_neg();
    }
}

fn dense_tree_sub(tree: &mut [u32], index: usize, delta: u32) {
    let mut idx = index + 1;
    while idx < tree.len() {
        tree[idx] = tree[idx].saturating_sub(delta);
        idx += idx & idx.wrapping_neg();
    }
}

fn height_tree_add(tree: &mut [u64], index: usize, delta: u64) {
    let mut idx = index + 1;
    while idx < tree.len() {
        tree[idx] = tree[idx].saturating_add(delta);
        idx += idx & idx.wrapping_neg();
    }
}

fn height_tree_sub(tree: &mut [u64], index: usize, delta: u64) {
    let mut idx = index + 1;
    while idx < tree.len() {
        tree[idx] = tree[idx].saturating_sub(delta);
        idx += idx & idx.wrapping_neg();
    }
}

fn dense_prefix_u64(heights: &[u32], tree: &[u32], index: usize) -> u64 {
    let mut idx = index.min(heights.len());
    let mut sum = 0_u64;
    while idx > 0 {
        sum = sum.saturating_add(u64::from(tree[idx]));
        idx &= idx - 1;
    }
    sum
}

fn height_tree_prefix_u64(tree: &[u64], index: usize) -> u64 {
    let mut idx = index.min(tree.len().saturating_sub(1));
    let mut sum = 0_u64;
    while idx > 0 {
        sum = sum.saturating_add(tree[idx]);
        idx &= idx - 1;
    }
    sum
}

fn locate_dense_height(heights: &[u32], tree: &[u32], target_px: u32) -> Option<(usize, u32)> {
    if heights.is_empty() {
        return None;
    }
    let target = u64::from(target_px);
    let total = dense_prefix_u64(heights, tree, heights.len());
    if target >= total {
        let index = heights.len() - 1;
        return Some((index, heights[index].saturating_sub(1)));
    }

    let mut idx = 0_usize;
    let mut bit = 1_usize;
    while bit < tree.len() {
        bit <<= 1;
    }
    let mut sum = 0_u64;
    while bit > 0 {
        let next = idx + bit;
        if next < tree.len() {
            let next_sum = sum.saturating_add(u64::from(tree[next]));
            if next_sum <= target {
                idx = next;
                sum = next_sum;
            }
        }
        bit >>= 1;
    }
    let index = idx.min(heights.len().saturating_sub(1));
    Some((
        index,
        target.saturating_sub(sum).min(u64::from(u32::MAX)) as u32,
    ))
}

fn locate_sparse_height(
    index: &FileHeightIndex,
    count: usize,
    total: u64,
    tree: &[u64],
    target_px: u32,
) -> Option<(usize, u32)> {
    if count == 0 {
        return None;
    }
    let target = u64::from(target_px);
    if target >= total {
        let slot = count - 1;
        return Some((slot, index.height_at(slot).saturating_sub(1)));
    }

    let mut slot = 0_usize;
    let mut bit = 1_usize;
    while bit < tree.len() {
        bit <<= 1;
    }
    let mut sum = 0_u64;
    while bit > 0 {
        let next = slot + bit;
        if next < tree.len() {
            let next_sum = sum.saturating_add(tree[next]);
            if next_sum <= target {
                slot = next;
                sum = next_sum;
            }
        }
        bit >>= 1;
    }
    let slot = slot.min(count.saturating_sub(1));
    Some((
        slot,
        target.saturating_sub(sum).min(u64::from(u32::MAX)) as u32,
    ))
}

pub(super) const CONTINUOUS_BOTTOM_ANCHOR_TOLERANCE_PX: u32 = 2;

pub(super) fn apply_scroll_delta_px(current: u32, delta: i32, max: u32) -> u32 {
    let next = if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as u32)
    };
    next.min(max)
}

impl AppState {
    pub(super) fn active_file_slot_key(
        &self,
        source: WorkspaceSource,
        active: &ActiveFile,
    ) -> ViewportSlotKey {
        let kind = if active.carbon_file.is_binary {
            ViewportSlotKind::Binary
        } else {
            ViewportSlotKind::Text {
                line_count: active.render_doc.lines.len(),
                text_len: active.render_doc.text_bytes.len(),
                style_run_count: active.render_doc.style_runs.len(),
                syntax_covered_count: active.syntax_covered.len(),
            }
        };
        ViewportSlotKey {
            source,
            index: active.index,
            path: active.path.clone(),
            left_ref: active.left_ref.clone(),
            right_ref: active.right_ref.clone(),
            kind,
        }
    }

    pub(super) fn loading_slot_key(
        &self,
        source: WorkspaceSource,
        index: usize,
        path: &str,
        left_ref: String,
        right_ref: String,
    ) -> ViewportSlotKey {
        ViewportSlotKey {
            source,
            index,
            path: path.to_owned(),
            left_ref,
            right_ref,
            kind: ViewportSlotKind::Loading,
        }
    }

    pub(super) fn compare_slot_key_at(&self, index: usize, path: &str) -> ViewportSlotKey {
        let source = match self.workspace.source.get(&self.store) {
            WorkspaceSource::TextCompare => WorkspaceSource::TextCompare,
            _ => WorkspaceSource::Compare,
        };
        let (left_ref, right_ref) = self.compare_refs();
        if let Some(key) = self.workspace.active_file.with(&self.store, |file| {
            file.as_ref()
                .filter(|file| {
                    file.index == index
                        && file.path == path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .map(|file| self.active_file_slot_key(source, file))
        }) {
            return key;
        }
        if let Some(key) = self.workspace.file_cache.with(&self.store, |files| {
            files
                .get(&index)
                .filter(|file| {
                    file.index == index
                        && file.path == path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .map(|file| self.active_file_slot_key(source, file))
        }) {
            return key;
        }
        self.loading_slot_key(source, index, path, left_ref, right_ref)
    }

    pub(super) fn status_slot_key_at(&self, index: usize, change: &FileChange) -> ViewportSlotKey {
        let (left_ref, right_ref) = self.status_refs_for_bucket(change.bucket);
        if let Some(key) = self.workspace.active_file.with(&self.store, |file| {
            file.as_ref()
                .filter(|file| {
                    file.index == index
                        && file.path == change.path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .map(|file| self.active_file_slot_key(WorkspaceSource::Status, file))
        }) {
            return key;
        }
        if let Some(key) = self.workspace.file_cache.with(&self.store, |files| {
            files
                .get(&index)
                .filter(|file| {
                    file.index == index
                        && file.path == change.path
                        && file.left_ref == left_ref
                        && file.right_ref == right_ref
                })
                .map(|file| self.active_file_slot_key(WorkspaceSource::Status, file))
        }) {
            return key;
        }
        self.loading_slot_key(
            WorkspaceSource::Status,
            index,
            &change.path,
            left_ref,
            right_ref,
        )
    }

    pub(super) fn append_viewport_slot_doc(
        &self,
        out: &mut RenderDoc,
        key: &ViewportSlotKey,
        loading_message: &str,
    ) {
        if let ViewportSlotKind::Loading = key.kind {
            out.append_doc(&build_placeholder_render_doc(&key.path, loading_message));
            return;
        }

        let mut appended = false;
        self.workspace.active_file.with(&self.store, |file| {
            let Some(active) = file.as_ref() else {
                return;
            };
            if active.index == key.index
                && active.path == key.path
                && active.left_ref == key.left_ref
                && active.right_ref == key.right_ref
            {
                append_active_file_doc(out, active);
                appended = true;
            }
        });
        if appended {
            return;
        }

        self.workspace.file_cache.with(&self.store, |files| {
            let Some(active) = files.get(&key.index).filter(|active| {
                active.index == key.index
                    && active.path == key.path
                    && active.left_ref == key.left_ref
                    && active.right_ref == key.right_ref
            }) else {
                return;
            };
            append_active_file_doc(out, active);
            appended = true;
        });

        if !appended {
            out.append_doc(&build_placeholder_render_doc(&key.path, loading_message));
        }
    }

    pub(super) fn viewport_slot_syntax_window(
        &self,
        key: &ViewportSlotKey,
        slot_top_px: u32,
        slot_height_px: u32,
        viewport_top_px: u32,
        viewport_height_px: u32,
    ) -> Option<SyntaxRowWindow> {
        let ViewportSlotKind::Text { line_count, .. } = key.kind else {
            return None;
        };
        if line_count == 0 {
            return None;
        }

        let slot_bottom_px = slot_top_px.saturating_add(slot_height_px.max(1));
        let viewport_bottom_px = viewport_top_px.saturating_add(viewport_height_px.max(1));
        let visible_top_px = slot_top_px.max(viewport_top_px);
        let visible_bottom_px = slot_bottom_px.min(viewport_bottom_px);
        if visible_bottom_px <= visible_top_px {
            return None;
        }

        let row_height_q16 = self.workspace.measured_px_per_row_q16.get(&self.store);
        let row_height_q16 = if row_height_q16 == 0 {
            24_u32 << 16
        } else {
            row_height_q16
        };
        let row_height_q16 = u64::from(row_height_q16.max(1));
        let start_px = visible_top_px.saturating_sub(slot_top_px);
        let end_px = visible_bottom_px.saturating_sub(slot_top_px);
        let row_floor = |px: u32| ((u64::from(px) << 16) / row_height_q16) as usize;
        let row_ceil = |px: u32| {
            (((u64::from(px) << 16).saturating_add(row_height_q16 - 1)) / row_height_q16) as usize
        };

        let start = row_floor(start_px)
            .saturating_sub(SYNTAX_OVERSCAN_ROWS)
            .min(line_count);
        let mut end = row_ceil(end_px)
            .saturating_add(SYNTAX_OVERSCAN_ROWS)
            .min(line_count);
        if end <= start {
            end = start.saturating_add(SYNTAX_INITIAL_ROWS).min(line_count);
        }
        Some(SyntaxRowWindow { start, end })
    }

    pub(super) fn request_viewport_slot_syntax_window(
        &mut self,
        key: &ViewportSlotKey,
        window: SyntaxRowWindow,
    ) -> Option<Effect> {
        if window.end <= window.start {
            return None;
        }
        if !self.syntax_request_budget_available() {
            return None;
        }
        let repo_path = self.compare.repo_path.get(&self.store)?;
        let generation = self.active_syntax_generation();
        let syntax_epoch = self.syntax_requests.epoch();
        let mut request = None;
        let request_id = self.syntax_requests.next_request_id();
        let mut matched_active = false;
        let mut active_to_cache = None;

        self.workspace.active_file.update(&self.store, |slot| {
            let Some(active) = slot.as_mut() else {
                return;
            };
            if active.index != key.index
                || active.path != key.path
                || active.left_ref != key.left_ref
                || active.right_ref != key.right_ref
            {
                return;
            }
            matched_active = true;
            if let Some(next_request) = request_syntax_for_active_file(
                active,
                repo_path.clone(),
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
        if matched_active {
            if let Some(request) = request {
                self.track_syntax_request(&request);
                return Some(
                    SyntaxEffect::LoadFileSyntax(Task {
                        generation,
                        request,
                    })
                    .into(),
                );
            }
            return None;
        }

        let request_id = self.syntax_requests.next_request_id();
        self.workspace.file_cache.update(&self.store, |files| {
            let Some(active) = files.get_mut(&key.index).filter(|active| {
                active.index == key.index
                    && active.path == key.path
                    && active.left_ref == key.left_ref
                    && active.right_ref == key.right_ref
            }) else {
                return;
            };
            request = request_syntax_for_active_file(
                active,
                repo_path,
                generation,
                syntax_epoch,
                window,
                request_id,
            );
        });

        request.map(|request| {
            self.track_syntax_request(&request);
            SyntaxEffect::LoadFileSyntax(Task {
                generation,
                request,
            })
            .into()
        })
    }

    pub(super) fn ensure_compare_file_cached_for_viewport(
        &mut self,
        index: usize,
        path: &str,
        priority: CompareWorkPriority,
    ) -> Vec<Effect> {
        if self.cached_compare_file_at(index, path).is_some() {
            return Vec::new();
        }
        if self.workspace.source.get(&self.store) == WorkspaceSource::TextCompare {
            if self.cache_compare_file_from_output(index, path).is_some() {
                return vec![
                    SyntaxEffect::EnsureSyntaxPackForPath {
                        path: path.to_owned(),
                    }
                    .into(),
                ];
            }
            return Vec::new();
        }
        if !self.compare_file_is_large(index)
            && self.cache_compare_file_from_output(index, path).is_some()
        {
            return vec![
                SyntaxEffect::EnsureSyntaxPackForPath {
                    path: path.to_owned(),
                }
                .into(),
            ];
        }
        if !self.should_enqueue_file_load(index, path, priority) {
            return Vec::new();
        }

        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        let deferred_file = self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .and_then(|output| compare_output_deferred_summary(output, index))
                .filter(|summary| summary.path() == path)
        });
        self.mark_file_cache_loading(index, path.to_owned(), priority);
        vec![
            SyntaxEffect::EnsureSyntaxPackForPath {
                path: path.to_owned(),
            }
            .into(),
            CompareEffect::LoadFile(Task {
                generation: self.workspace.compare_generation.get(&self.store),
                request: CompareFileRequest {
                    repo_path,
                    request: vcs_compare_request(
                        self.compare.mode.get(&self.store),
                        self.compare.left_ref.get(&self.store),
                        self.compare.right_ref.get(&self.store),
                        self.compare.layout.get(&self.store),
                        self.compare.renderer.get(&self.store),
                    ),
                    path: path.to_owned(),
                    index,
                    deferred_file,
                    priority,
                },
            })
            .into(),
        ]
    }

    pub(super) fn ensure_status_file_cached_for_viewport(&mut self, index: usize) -> Vec<Effect> {
        let Some(file_change) = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| changes.get(index).cloned())
        else {
            return Vec::new();
        };
        if self.cached_status_file_at(index, &file_change).is_some() {
            return Vec::new();
        }
        if !self.should_enqueue_file_load(
            index,
            &file_change.path,
            CompareWorkPriority::VisibleViewportDiff,
        ) {
            return Vec::new();
        }

        let Some(repo_path) = self.compare.repo_path.get(&self.store) else {
            return Vec::new();
        };
        self.mark_file_cache_loading(
            index,
            file_change.path.clone(),
            CompareWorkPriority::VisibleViewportDiff,
        );
        let generation = self.workspace.status_generation.get(&self.store);
        let renderer = self.compare.renderer.get(&self.store);
        vec![
            ensure_syntax_packs_for_file_change_effect(&file_change),
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
        ]
    }
}

impl AppState {
    pub(super) fn scroll_viewport_lines(&mut self, delta_lines: i32) -> Vec<Effect> {
        let step_px = 20_i32;
        let delta_px = delta_lines.saturating_mul(step_px);
        self.scroll_viewport_px(delta_px)
    }

    pub(super) fn scroll_viewport_px(&mut self, delta_px: i32) -> Vec<Effect> {
        if !self.settings.continuous_scroll {
            let current = self.editor.scroll_top_px.get(&self.store);
            let max = self.editor_max_scroll_top_px();
            let next = apply_scroll_delta_px(current, delta_px, max);
            self.editor.scroll_top_px.set(&self.store, next);
            return Vec::new();
        }

        if delta_px == 0 {
            return Vec::new();
        }

        let current = self.workspace.global_scroll_top_px.get(&self.store);
        let target = apply_scroll_delta_px(current, delta_px, self.global_max_scroll_top_px());
        self.scroll_viewport_to_global(target)
    }

    pub(super) fn clear_file_scroll_layout(&mut self) {
        self.workspace
            .file_content_heights
            .set(&self.store, Vec::new());
        self.workspace
            .file_scroll_total_height_px
            .set(&self.store, 0);
        self.workspace
            .pending_file_content_heights
            .set(&self.store, HashMap::new());
        self.workspace
            .file_scroll_recompute_pending
            .set(&self.store, false);
        self.workspace
            .viewport_scrollbar_drag
            .set(&self.store, None);
        self.virtual_diff_document.clear();
        self.virtual_scroll.clear();
        self.last_virtual_scroll_top_px = None;
    }

    pub(super) fn reset_file_scroll_layout(&mut self) {
        self.workspace
            .file_content_heights
            .set(&self.store, Vec::new());
        self.workspace
            .pending_file_content_heights
            .set(&self.store, HashMap::new());
        self.workspace
            .file_scroll_recompute_pending
            .set(&self.store, false);
        self.workspace
            .viewport_scrollbar_drag
            .set(&self.store, None);
        self.virtual_scroll.clear();
        self.last_virtual_scroll_top_px = None;
        self.recompute_file_scroll_total_height_px();
    }

    pub fn recompute_file_scroll_total_height_px(&mut self) {
        let count = self.workspace_file_count();
        let source = self.workspace.source.get(&self.store);
        let generation = self.workspace_render_generation();
        if self
            .virtual_diff_document
            .sync_identity(source, generation, count)
        {
            self.virtual_scroll.clear();
            self.last_virtual_scroll_top_px = None;
        }
        self.workspace
            .file_content_heights
            .update(&self.store, |heights| {
                if heights.len() > count {
                    heights.truncate(count);
                }
            });

        let heights = (0..count)
            .map(|index| self.file_scroll_height_px(index).max(1))
            .collect::<Vec<_>>();
        self.virtual_diff_document.rebuild_heights(heights);
        let total = self.virtual_diff_document.total_u32();
        self.workspace
            .file_scroll_total_height_px
            .set(&self.store, total);
    }

    pub(super) fn update_file_scroll_heights(&mut self, old_heights: Vec<(usize, u32)>) {
        let count = self.workspace_file_count();
        if self.virtual_diff_document.len() != count {
            self.recompute_file_scroll_total_height_px();
            return;
        }

        let mut total = self.workspace.file_scroll_total_height_px.get(&self.store);
        for (index, old_height) in old_heights {
            if index >= count {
                continue;
            }
            let new_height = self.file_scroll_height_px(index).max(1);
            total = total.saturating_sub(old_height).saturating_add(new_height);
            self.virtual_diff_document.update_height(index, new_height);
        }
        self.workspace
            .file_scroll_total_height_px
            .set(&self.store, total);
    }

    pub fn update_file_content_height_px(&mut self, index: usize, height: u32) -> bool {
        let count = self.workspace_file_count();
        if index >= count || height == 0 {
            return false;
        }
        if self.settings.continuous_scroll
            && self
                .workspace
                .viewport_scrollbar_drag
                .get(&self.store)
                .is_some()
        {
            self.workspace
                .pending_file_content_heights
                .update(&self.store, |pending| {
                    pending.insert(index, height);
                });
            return false;
        }
        if self.virtual_diff_document.len() != count {
            self.recompute_file_scroll_total_height_px();
        }

        let old_slot_height = self.file_scroll_height_px(index);
        let old_total = self.total_diff_height_px();
        let anchor = self
            .settings
            .continuous_scroll
            .then(|| self.current_or_derived_viewport_anchor())
            .flatten();
        let row_count = self.workspace_file_row_count(index);
        let mut recorded_changed = false;
        self.workspace
            .file_content_heights
            .update(&self.store, |heights| {
                if heights.len() < count {
                    heights.resize(count, None);
                }
                if heights[index] != Some(height) {
                    heights[index] = Some(height);
                    recorded_changed = true;
                }
            });

        let mut calibration_initialized = false;
        if let Some(rows) = row_count
            && rows > 0
        {
            let sample_q16 = (u64::from(height) << 16) / u64::from(rows);
            let prev = self.workspace.measured_px_per_row_q16.get(&self.store);
            let next = if prev == 0 {
                calibration_initialized = true;
                sample_q16 as u32
            } else {
                (((u64::from(prev) * 7) + sample_q16) / 8) as u32
            };
            self.workspace
                .measured_px_per_row_q16
                .set(&self.store, next);
        }

        if calibration_initialized {
            self.recompute_file_scroll_total_height_px();
        }

        if recorded_changed {
            let new_slot_height = self.file_scroll_height_px(index);
            let slot_height_changed = new_slot_height != old_slot_height;
            if calibration_initialized {
                self.workspace
                    .file_scroll_total_height_px
                    .set(&self.store, self.virtual_diff_document.total_u32());
            } else {
                let next_total = old_total
                    .saturating_sub(old_slot_height)
                    .saturating_add(new_slot_height);
                self.workspace
                    .file_scroll_total_height_px
                    .set(&self.store, next_total);
                self.virtual_diff_document
                    .update_height(index, new_slot_height.max(1));
            }

            if self.settings.continuous_scroll
                && slot_height_changed
                && let Some(anchor) = anchor
            {
                self.rebase_viewport_anchor(anchor);
            }
        }

        recorded_changed && old_slot_height != self.file_scroll_height_px(index)
    }

    pub fn update_virtual_diff_item_height_px(
        &mut self,
        item_id: VirtualDiffItemId,
        height: u32,
    ) -> bool {
        if item_id.kind != VirtualDiffItemKind::File
            || item_id.source != self.workspace.source.get(&self.store)
            || item_id.generation != self.workspace_render_generation()
        {
            return false;
        }
        self.update_file_content_height_px(item_id.index, height)
    }

    pub fn virtual_stream_item(
        &self,
        file_index: usize,
        kind: VirtualDiffItemKind,
        ordinal: u32,
        stable_key: u64,
        sort_key: u64,
        measured_height_px: Option<u32>,
    ) -> VirtualDiffStreamItem {
        VirtualDiffStreamItem::new(
            VirtualDiffItemId::new(
                self.workspace.source.get(&self.store),
                self.workspace_render_generation(),
                kind,
                file_index,
                ordinal,
                stable_key,
            ),
            sort_key,
            measured_height_px.unwrap_or_else(|| estimated_virtual_item_height_px(kind)),
            measured_height_px,
        )
    }

    pub(super) fn virtual_stream_items_for_viewport_doc(
        &self,
        source: WorkspaceSource,
        generation: u64,
        slots: &[ViewportSlotKey],
        doc: &RenderDoc,
    ) -> Vec<VirtualDiffStreamItem> {
        let mut items = Vec::new();
        let mut slot_pos = None::<usize>;
        let mut local_ordinal = 0_u32;

        for (line_index, line) in doc.lines.iter().enumerate() {
            if line.row_kind() == RenderRowKind::FileHeader {
                slot_pos = Some(slot_pos.map_or(0, |pos| pos.saturating_add(1)));
                local_ordinal = 0;
            }

            let Some(slot) = slot_pos.and_then(|pos| slots.get(pos)) else {
                continue;
            };
            let Some(kind) = virtual_stream_item_kind(slot, line) else {
                continue;
            };
            let ordinal = match kind {
                VirtualDiffItemKind::FileHeader => 0,
                VirtualDiffItemKind::Hunk if line.hunk_index >= 0 => line.hunk_index as u32,
                _ => local_ordinal,
            };

            items.push(VirtualDiffStreamItem::new(
                VirtualDiffItemId::new(
                    source,
                    generation,
                    kind,
                    slot.index,
                    ordinal,
                    virtual_row_stable_key(line, ordinal),
                ),
                virtual_row_sort_key(line_index),
                estimated_virtual_item_height_px(kind),
                None,
            ));
            local_ordinal = local_ordinal.saturating_add(1);
        }

        items
    }

    pub(super) fn file_scroll_height_px(&self, index: usize) -> u32 {
        self.workspace
            .file_content_heights
            .with(&self.store, |heights| heights.get(index).copied().flatten())
            .unwrap_or_else(|| self.estimated_file_height_px(index))
    }

    pub(super) fn viewport_file_scroll_height_px(&self, index: usize) -> u32 {
        if let Some(height) = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| {
                drag.as_ref()
                    .and_then(|drag| drag.file_heights_px.get(index).copied())
            })
        {
            return height;
        }
        self.file_scroll_height_px(index)
    }

    pub fn estimated_file_height_px(&self, index: usize) -> u32 {
        const BASELINE_ROWS: u32 = 8;
        let row_height_q16 = {
            let cal = self.workspace.measured_px_per_row_q16.get(&self.store);
            if cal == 0 { 24_u32 << 16 } else { cal }
        };
        let row_height_px =
            |rows: u32| ((u64::from(rows) * u64::from(row_height_q16)) >> 16) as u32;

        if matches!(
            self.workspace.source.get(&self.store),
            WorkspaceSource::Compare | WorkspaceSource::TextCompare
        ) && let Some(rows) = self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .and_then(|output| output.carbon.files.get(index))
                .map(estimated_carbon_file_rows_with_overhead)
        }) {
            return row_height_px(rows);
        }

        let line_count = match self.workspace.source.get(&self.store) {
            WorkspaceSource::Compare | WorkspaceSource::TextCompare => {
                if index < self.workspace_file_count() {
                    let meta = self.file_list_entry_meta(index);
                    meta.additions.saturating_add(meta.deletions).max(1) as u32 + BASELINE_ROWS
                } else {
                    BASELINE_ROWS
                }
            }
            WorkspaceSource::Status => BASELINE_ROWS,
            WorkspaceSource::None => BASELINE_ROWS,
        };
        row_height_px(line_count)
    }

    pub(super) fn workspace_file_row_count(&self, index: usize) -> Option<u32> {
        if !matches!(
            self.workspace.source.get(&self.store),
            WorkspaceSource::Compare | WorkspaceSource::TextCompare
        ) {
            return None;
        }
        self.workspace.compare_output.with(&self.store, |output| {
            output
                .as_ref()
                .and_then(|output| output.carbon.files.get(index))
                .map(estimated_carbon_file_rows_with_overhead)
        })
    }

    pub fn total_diff_height_px(&self) -> u32 {
        if let Some(total) = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| {
                drag.as_ref().map(|drag| drag.metrics.content_height_px)
            })
        {
            return total;
        }
        let cached = self.workspace.file_scroll_total_height_px.get(&self.store);
        if cached > 0 || self.workspace_file_count() == 0 {
            return cached;
        }

        self.virtual_diff_document.total_u32()
    }

    pub fn file_start_offset_px(&self, index: usize) -> u32 {
        if self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.is_none())
            && self.virtual_diff_document.len() == self.workspace_file_count()
        {
            return self.virtual_diff_document.prefix_u32(index);
        }
        let mut total: u32 = 0;
        for slot in 0..index.min(self.workspace_file_count()) {
            total = total.saturating_add(self.viewport_file_scroll_height_px(slot));
        }
        total
    }

    pub fn global_max_scroll_top_px(&self) -> u32 {
        if let Some(max) = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| {
                drag.as_ref().map(|drag| drag.metrics.max_scroll_top_px)
            })
        {
            return max;
        }
        let viewport = self.editor.viewport_height_px.get(&self.store);
        self.total_diff_height_px().saturating_sub(viewport.max(1))
    }

    pub(super) fn viewport_anchor_bias_for_global(&self, scroll_top_px: u32) -> ViewportAnchorBias {
        let max = self.global_max_scroll_top_px();
        if max > 0 && scroll_top_px.saturating_add(CONTINUOUS_BOTTOM_ANCHOR_TOLERANCE_PX) >= max {
            ViewportAnchorBias::FollowEnd
        } else {
            ViewportAnchorBias::PreserveTop
        }
    }

    pub(super) fn viewport_anchor_for_file_offset(
        &self,
        index: usize,
        local_offset_px: u32,
        bias: ViewportAnchorBias,
    ) -> Option<ViewportAnchor> {
        let item_id = self.virtual_diff_document.item_id(index)?;
        Some(ViewportAnchor {
            item_id,
            intra_item_offset_px: local_offset_px,
            bias,
        })
    }

    pub(super) fn viewport_anchor_for_global(
        &self,
        scroll_top_px: u32,
        bias: ViewportAnchorBias,
    ) -> Option<ViewportAnchor> {
        let target_px = match bias {
            ViewportAnchorBias::PreserveBottom => {
                scroll_top_px.saturating_add(self.editor.viewport_height_px.get(&self.store).max(1))
            }
            ViewportAnchorBias::PreserveTop | ViewportAnchorBias::FollowEnd => scroll_top_px,
        };
        let (index, local_offset_px) = self.locate_global_scroll_px(target_px)?;
        self.viewport_anchor_for_file_offset(index, local_offset_px, bias)
    }

    pub(super) fn current_or_derived_viewport_anchor(&self) -> Option<ViewportAnchor> {
        if let Some(anchor) = self.virtual_scroll.anchor
            && self.virtual_diff_document.anchor_is_current(anchor)
        {
            return Some(anchor);
        }
        let scroll_top_px = self.workspace.global_scroll_top_px.get(&self.store);
        let bias = self.viewport_anchor_bias_for_global(scroll_top_px);
        self.viewport_anchor_for_global(scroll_top_px, bias)
    }

    pub(super) fn scroll_top_for_viewport_anchor(&self, anchor: ViewportAnchor) -> Option<u32> {
        if !self.virtual_diff_document.anchor_is_current(anchor) {
            return None;
        }
        if anchor.bias == ViewportAnchorBias::FollowEnd {
            return Some(self.global_max_scroll_top_px());
        }

        let index = anchor.item_id.index;
        let item_height = self
            .viewport_file_scroll_height_px(index)
            .max(self.virtual_diff_document.height_at(index))
            .max(1);
        let local_offset = anchor
            .intra_item_offset_px
            .min(item_height.saturating_sub(1));
        let item_top = self.file_start_offset_px(index);
        let target = match anchor.bias {
            ViewportAnchorBias::PreserveTop => item_top.saturating_add(local_offset),
            ViewportAnchorBias::PreserveBottom => item_top
                .saturating_add(local_offset)
                .saturating_sub(self.editor.viewport_height_px.get(&self.store).max(1)),
            ViewportAnchorBias::FollowEnd => unreachable!(),
        };
        Some(target.min(self.global_max_scroll_top_px()))
    }

    pub(super) fn set_viewport_anchor(&mut self, anchor: ViewportAnchor) {
        if let Some(scroll_top_px) = self.scroll_top_for_viewport_anchor(anchor) {
            self.workspace
                .global_scroll_top_px
                .set(&self.store, scroll_top_px);
            self.virtual_scroll.set_anchor(anchor);
        } else {
            self.virtual_scroll.clear();
            self.clamp_global_scroll_top_px();
        }
    }

    pub(super) fn set_viewport_anchor_for_global(
        &mut self,
        scroll_top_px: u32,
        bias: ViewportAnchorBias,
    ) {
        if let Some(anchor) = self.viewport_anchor_for_global(scroll_top_px, bias) {
            self.set_viewport_anchor(anchor);
        } else {
            self.virtual_scroll.clear();
            self.workspace.global_scroll_top_px.set(&self.store, 0);
        }
    }

    pub(super) fn rebase_viewport_anchor(&mut self, anchor: ViewportAnchor) {
        self.set_viewport_anchor(anchor);
    }

    pub(super) fn clamp_global_scroll_top_px(&mut self) {
        if let Some(anchor) = self.virtual_scroll.anchor
            && let Some(scroll_top_px) = self.scroll_top_for_viewport_anchor(anchor)
        {
            self.workspace
                .global_scroll_top_px
                .set(&self.store, scroll_top_px);
            return;
        }
        let max = self.global_max_scroll_top_px();
        let current = self.workspace.global_scroll_top_px.get(&self.store);
        self.workspace
            .global_scroll_top_px
            .set(&self.store, current.min(max));
    }

    pub(super) fn locate_global_scroll_px(&self, target_px: u32) -> Option<(usize, u32)> {
        let count = self.workspace_file_count();
        if count == 0 {
            return None;
        }
        if self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.is_none())
            && self.virtual_diff_document.len() == count
        {
            return self.virtual_diff_document.locate(target_px);
        }
        let mut prior: u32 = 0;
        for index in 0..count {
            let height = self.viewport_file_scroll_height_px(index).max(1);
            let next_prior = prior.saturating_add(height);
            if target_px < next_prior || index + 1 == count {
                return Some((index, target_px.saturating_sub(prior)));
            }
            prior = next_prior;
        }
        Some((count - 1, 0))
    }

    pub(super) fn scroll_viewport_to_global(&mut self, target_px: u32) -> Vec<Effect> {
        if self.virtual_diff_document.len() != self.workspace_file_count() {
            self.recompute_file_scroll_total_height_px();
        }
        let target_px = target_px.min(self.global_max_scroll_top_px());
        let bias = self.viewport_anchor_bias_for_global(target_px);
        self.set_viewport_anchor_for_global(target_px, bias);
        let target_px = self.workspace.global_scroll_top_px.get(&self.store);
        let Some((target_index, local_offset)) = self.locate_global_scroll_px(target_px) else {
            self.workspace.global_scroll_top_px.set(&self.store, 0);
            self.virtual_scroll.clear();
            return Vec::new();
        };
        self.workspace
            .global_scroll_top_px
            .set(&self.store, target_px);
        self.workspace
            .viewport_scrollbar_drag
            .update(&self.store, |drag| {
                if let Some(drag) = drag.as_mut() {
                    drag.metrics.scroll_top_px = target_px.min(drag.metrics.max_scroll_top_px);
                }
            });

        let dragging_scrollbar = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.is_some());
        let mut effects = if dragging_scrollbar {
            Vec::new()
        } else if self.active_file_matches_workspace_file(target_index) {
            Vec::new()
        } else {
            self.select_file_inner(target_index, true)
        };

        let local_max = self.editor_max_scroll_top_px();
        self.editor
            .scroll_top_px
            .set(&self.store, local_offset.min(local_max));
        if !dragging_scrollbar {
            effects.extend(self.request_active_file_syntax_effect());
        }
        effects
    }

    pub fn global_scroll_position_px(&self) -> u32 {
        self.workspace.global_scroll_top_px.get(&self.store)
    }

    pub fn continuous_viewport_scrollbar_metrics(&self) -> ViewportScrollbarMetrics {
        if let Some(metrics) = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.as_ref().map(|drag| drag.metrics))
        {
            return metrics;
        }
        let viewport_height_px = self.editor.viewport_height_px.get(&self.store);
        let content_height_px = self.total_diff_height_px();
        ViewportScrollbarMetrics {
            content_height_px,
            viewport_height_px,
            scroll_top_px: self.global_scroll_position_px(),
            max_scroll_top_px: content_height_px.saturating_sub(viewport_height_px.max(1)),
        }
    }

    pub fn begin_viewport_scrollbar_drag(
        &mut self,
        content_height_px: u32,
        viewport_height_px: u32,
        scroll_top_px: u32,
        max_scroll_top_px: u32,
    ) {
        if !self.settings.continuous_scroll {
            return;
        }
        let file_heights_px = (0..self.workspace_file_count())
            .map(|index| self.file_scroll_height_px(index).max(1))
            .collect();
        self.workspace.viewport_scrollbar_drag.set(
            &self.store,
            Some(ViewportScrollbarDragState {
                metrics: ViewportScrollbarMetrics {
                    content_height_px,
                    viewport_height_px,
                    scroll_top_px: scroll_top_px.min(max_scroll_top_px),
                    max_scroll_top_px,
                },
                file_heights_px,
            }),
        );
    }

    pub fn end_viewport_scrollbar_drag(&mut self) {
        self.workspace
            .viewport_scrollbar_drag
            .set(&self.store, None);
        self.apply_pending_file_scroll_updates();
    }

    pub(super) fn apply_pending_file_scroll_updates(&mut self) {
        let pending_heights = self
            .workspace
            .pending_file_content_heights
            .with(&self.store, |pending| pending.clone());
        self.workspace
            .pending_file_content_heights
            .set(&self.store, HashMap::new());
        for (index, height) in pending_heights {
            self.update_file_content_height_px(index, height);
        }
        if self
            .workspace
            .file_scroll_recompute_pending
            .get(&self.store)
        {
            self.workspace
                .file_scroll_recompute_pending
                .set(&self.store, false);
            self.recompute_file_scroll_total_height_px();
            self.clamp_global_scroll_top_px();
        }
    }

    pub fn sync_editor_scroll_from_global(&mut self) -> Vec<Effect> {
        if !self.settings.continuous_scroll {
            return Vec::new();
        }
        self.clamp_global_scroll_top_px();
        let target = self.workspace.global_scroll_top_px.get(&self.store);
        let Some((_, local_offset)) = self.locate_global_scroll_px(target) else {
            self.workspace.global_scroll_top_px.set(&self.store, 0);
            self.virtual_scroll.clear();
            return Vec::new();
        };
        let max = self.editor_max_scroll_top_px();
        self.editor
            .scroll_top_px
            .set(&self.store, local_offset.min(max));
        Vec::new()
    }

    pub fn sync_global_scroll_from_editor(&mut self) {
        let Some(selected_index) = self.reconcile_selected_file_index_from_path() else {
            self.workspace.global_scroll_top_px.set(&self.store, 0);
            self.virtual_scroll.clear();
            return;
        };
        let start = self.file_start_offset_px(selected_index);
        let local = self.editor.scroll_top_px.get(&self.store);
        let target = start
            .saturating_add(local)
            .min(self.global_max_scroll_top_px());
        self.workspace.global_scroll_top_px.set(&self.store, target);
        if self.settings.continuous_scroll {
            if let Some(anchor) = self.viewport_anchor_for_file_offset(
                selected_index,
                local,
                self.viewport_anchor_bias_for_global(target),
            ) {
                self.virtual_scroll.set_anchor(anchor);
            } else {
                self.virtual_scroll.clear();
            }
        }
    }

    pub fn build_continuous_viewport_document(
        &mut self,
    ) -> (Option<ViewportDocument>, Vec<Effect>) {
        if !self.settings.continuous_scroll {
            return (None, Vec::new());
        }
        if self.virtual_diff_document.len() != self.workspace_file_count() {
            self.recompute_file_scroll_total_height_px();
        }
        self.clamp_global_scroll_top_px();
        let scroll_top_px = self.workspace.global_scroll_top_px.get(&self.store);
        let scroll_direction = match self.last_virtual_scroll_top_px {
            Some(previous) if scroll_top_px < previous => ScrollDirection::Backward,
            _ => ScrollDirection::Forward,
        };
        self.last_virtual_scroll_top_px = Some(scroll_top_px);
        let Some((anchor_index, _)) = self.locate_global_scroll_px(scroll_top_px) else {
            return (None, Vec::new());
        };

        let source = self.workspace.source.get(&self.store);
        if source == WorkspaceSource::None {
            return (None, Vec::new());
        }
        let dragging_scrollbar = self
            .workspace
            .viewport_scrollbar_drag
            .with(&self.store, |drag| drag.is_some());

        let count = self.workspace_file_count();
        let viewport = self.editor.viewport_height_px.get(&self.store).max(1);
        let follow_end = self.virtual_scroll.anchor.is_some_and(|anchor| {
            anchor.bias == ViewportAnchorBias::FollowEnd
                && self.virtual_diff_document.anchor_is_current(anchor)
        }) || self.viewport_anchor_bias_for_global(scroll_top_px)
            == ViewportAnchorBias::FollowEnd;
        let (start_index, start_offset, local_top, target_height) = if follow_end {
            let mut start_index = count.saturating_sub(1);
            let mut tail_height = self.viewport_file_scroll_height_px(start_index).max(1);
            let target_tail_height = viewport.saturating_mul(2).max(viewport);
            while start_index > 0 && tail_height < target_tail_height {
                start_index -= 1;
                tail_height = tail_height
                    .saturating_add(self.viewport_file_scroll_height_px(start_index).max(1));
            }
            (
                start_index,
                self.file_start_offset_px(start_index),
                tail_height.saturating_sub(viewport),
                tail_height.max(1),
            )
        } else {
            let mut start_index = anchor_index;
            let mut before_viewport_px = 0_u32;
            while start_index > 0 && before_viewport_px < viewport {
                start_index -= 1;
                before_viewport_px = before_viewport_px
                    .saturating_add(self.viewport_file_scroll_height_px(start_index).max(1));
            }
            let start_offset = self.file_start_offset_px(start_index);
            let local_top = self
                .workspace
                .global_scroll_top_px
                .get(&self.store)
                .saturating_sub(start_offset);
            let target_height = local_top
                .saturating_add(viewport)
                .saturating_add(viewport / 2)
                .max(1);
            (start_index, start_offset, local_top, target_height)
        };

        let mut effects = Vec::new();
        let mut slot_keys = Vec::new();
        let mut slot_loading = Vec::new();
        let mut accumulated = 0_u32;
        let mut index = start_index;
        while index < count && (slot_keys.is_empty() || accumulated < target_height) {
            let path = self
                .workspace_file_path_at(index)
                .unwrap_or_else(|| format!("File {}", index + 1));
            let slot_key = match source {
                WorkspaceSource::Compare | WorkspaceSource::TextCompare => {
                    effects.extend(self.ensure_compare_file_cached_for_viewport(
                        index,
                        &path,
                        CompareWorkPriority::VisibleViewportDiff,
                    ));
                    self.compare_slot_key_at(index, &path)
                }
                WorkspaceSource::Status => {
                    effects.extend(self.ensure_status_file_cached_for_viewport(index));
                    let file_change = self
                        .workspace
                        .status_file_changes
                        .with(&self.store, |changes| changes.get(index).cloned());
                    file_change.as_ref().map_or_else(
                        || {
                            self.loading_slot_key(
                                WorkspaceSource::Status,
                                index,
                                &path,
                                String::new(),
                                String::new(),
                            )
                        },
                        |change| self.status_slot_key_at(index, change),
                    )
                }
                WorkspaceSource::None => self.loading_slot_key(
                    WorkspaceSource::None,
                    index,
                    &path,
                    String::new(),
                    String::new(),
                ),
            };
            let slot_height = self.viewport_file_scroll_height_px(index).max(1);
            if let Some(window) = self.viewport_slot_syntax_window(
                &slot_key,
                accumulated,
                slot_height,
                local_top,
                viewport,
            ) {
                effects.extend(self.request_viewport_slot_syntax_window(&slot_key, window));
            }
            let slot_is_loading = matches!(&slot_key.kind, ViewportSlotKind::Loading);
            if !slot_is_loading {
                self.touch_viewport_slot(&slot_key);
            }
            slot_loading.push(slot_is_loading);
            slot_keys.push(slot_key);
            accumulated = accumulated.saturating_add(slot_height);
            index += 1;
        }
        let render_end_index = index;
        self.protect_working_set_slots(&slot_keys);
        self.trim_file_working_set();
        effects.extend(self.prefetch_compare_working_set(
            start_index,
            render_end_index,
            scroll_direction,
            viewport,
        ));

        let key = ViewportDocumentKey {
            source,
            generation: self.workspace_render_generation(),
            start_index,
            slots: slot_keys,
        };
        let doc = if let Some(cache) = self.viewport_document_cache.as_ref()
            && cache.key == key
        {
            cache.doc.clone()
        } else {
            let mut doc = RenderDoc::default();
            let loading_message = if dragging_scrollbar {
                ""
            } else {
                "Loading diff..."
            };
            for slot in &key.slots {
                self.append_viewport_slot_doc(&mut doc, slot, loading_message);
            }
            let doc = Arc::new(doc);
            self.viewport_document_cache = Some(ViewportDocumentCache {
                key: key.clone(),
                doc: doc.clone(),
            });
            doc
        };
        let slot_indices = key.slots.iter().map(|slot| slot.index).collect();
        let slot_item_ids = key
            .slots
            .iter()
            .map(|slot| {
                self.virtual_diff_document
                    .item_id(slot.index)
                    .unwrap_or_else(|| {
                        VirtualDiffItemId::file(
                            source,
                            self.workspace_render_generation(),
                            slot.index,
                        )
                    })
            })
            .collect();
        let stream_items = self.virtual_stream_items_for_viewport_doc(
            source,
            self.workspace_render_generation(),
            &key.slots,
            doc.as_ref(),
        );

        (
            Some(ViewportDocument {
                doc,
                mode: ViewportDocumentMode::Continuous,
                generation: self.workspace_render_generation(),
                start_index,
                start_offset_px: start_offset,
                scroll_top_px: local_top,
                slot_indices,
                slot_item_ids,
                stream_items,
                slot_loading,
                path: String::new(),
            }),
            effects,
        )
    }

    pub(super) fn scroll_viewport_pages(&mut self, delta_pages: i32) -> Vec<Effect> {
        let viewport = self.editor.viewport_height_px.get(&self.store);
        let page_px = ((viewport as f32) * 0.85).round().max(1.0) as i32;
        let delta_px = delta_pages.saturating_mul(page_px);
        if self.settings.continuous_scroll {
            return self.scroll_viewport_px(delta_px);
        }
        let current = self.editor.scroll_top_px.get(&self.store);
        let max = self.editor_max_scroll_top_px();
        let next = apply_scroll_delta_px(current, delta_px, max);
        self.editor.scroll_top_px.set(&self.store, next);
        Vec::new()
    }

    pub(super) fn scroll_viewport_half_page(&mut self, direction: i32) -> Vec<Effect> {
        let viewport = self.editor.viewport_height_px.get(&self.store);
        let half_px = ((viewport as f32) * 0.5).round().max(1.0) as i32;
        let delta_px = direction.saturating_mul(half_px);
        if self.settings.continuous_scroll {
            return self.scroll_viewport_px(delta_px);
        }
        let current = self.editor.scroll_top_px.get(&self.store);
        let max = self.editor_max_scroll_top_px();
        let next = apply_scroll_delta_px(current, delta_px, max);
        self.editor.scroll_top_px.set(&self.store, next);
        Vec::new()
    }
}
