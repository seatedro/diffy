use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use diffy_diff::{Placeholder, Revision, SlotState, SourceCoverage, SourcePurpose};

use crate::contracts::commands::{ToastKind, ToastUpdate};
use crate::contracts::shared::{CompareGeneration, FileKey, FileRevision, SharedContext};
use crate::contracts::shell::{Controller, Intent, Reduction};
use crate::contracts::viewport::PipelineAction;
use crate::contracts::viewport::PipelineEvent;
use crate::contracts::viewport::pipeline::{
    CompareIntent, CompareJob, CompareManifestSnapshot, CompareResult, CompareWorkPriority,
    FileLoadRequest, FileStat, FileStats, FileStatsRequest, FileTarget, ManifestFile,
    PipelineEffect, PipelineFailure, Preparation, PreparedDiff, SlotUpdate, SourceLoadRequest,
    TotalStatsRequest,
};
use crate::core::compare::git::bucket_refs;
use crate::core::compare::{CompareEntry, ComparePhase};
use crate::core::vcs::model::{ChangeBucket, VcsCompareSpec};

pub use crate::contracts::viewport::interaction::ViewportInstall;

pub const INSTALL_LINES_PER_FRAME: u64 = 40_000;
pub const INSTALL_ITEMS_PER_FRAME: usize = 256;
pub const WORKING_SET_LINES: u64 = 400_000;
pub const WORKING_SET_FILES: usize = 512;

#[derive(Debug, Clone)]
pub enum PipelineInstall {
    View(ViewportInstall),
    Slot(SlotUpdate),
}

impl PipelineInstall {
    fn cost(&self) -> u64 {
        match self {
            Self::View(ViewportInstall::File { prepared, .. })
            | Self::View(ViewportInstall::TextCompare { prepared }) => {
                u64::from(prepared.estimated_lines())
            }
            _ => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PipelineStatus {
    #[default]
    Idle,
    Comparing(Option<ComparePhase>),
    Ready,
    Failed(String),
}

#[derive(Debug)]
struct ActiveCompare {
    generation: CompareGeneration,
    intent: CompareIntent,
    manifest: Option<Rc<CompareManifestSnapshot>>,
    entries: Arc<[CompareEntry]>,
    loaded: HashMap<FileKey, PreparedDiff>,
    placeholders: HashMap<FileKey, Placeholder>,
    loading: HashSet<FileKey>,
    stats_loading: HashSet<FileKey>,
    sources_loading: HashSet<FileKey>,
    totals_loading: bool,
    used: HashMap<FileKey, u64>,
    positions: HashMap<FileKey, usize>,
    selected: Option<FileKey>,
}

impl ActiveCompare {
    fn new(
        generation: CompareGeneration,
        intent: CompareIntent,
        selected: Option<FileKey>,
    ) -> Self {
        Self {
            generation,
            intent,
            manifest: None,
            entries: Arc::from([]),
            loaded: HashMap::new(),
            placeholders: HashMap::new(),
            loading: HashSet::new(),
            stats_loading: HashSet::new(),
            sources_loading: HashSet::new(),
            totals_loading: false,
            used: HashMap::new(),
            positions: HashMap::new(),
            selected,
        }
    }
}

#[derive(Debug, Default)]
pub struct PipelineController {
    last_generation: u64,
    clock: u64,
    active: Option<ActiveCompare>,
    status: PipelineStatus,
    preparation: Option<Preparation>,
    queue: VecDeque<PipelineInstall>,
    resuming: bool,
}

impl PipelineController {
    pub fn take_batch(&mut self) -> Vec<PipelineInstall> {
        let mut batch = Vec::new();
        let mut cost = 0u64;
        while let Some(next) = self.queue.front() {
            let next_cost = next.cost();
            if !batch.is_empty()
                && (cost + next_cost > INSTALL_LINES_PER_FRAME
                    || batch.len() >= INSTALL_ITEMS_PER_FRAME)
            {
                break;
            }
            cost += next_cost;
            batch.extend(self.queue.pop_front());
        }
        batch
    }

    pub fn take_installs(&mut self) -> Vec<ViewportInstall> {
        self.take_batch()
            .into_iter()
            .filter_map(|install| match install {
                PipelineInstall::View(install) => Some(install),
                PipelineInstall::Slot(_) => None,
            })
            .collect()
    }

    pub fn has_pending_installs(&self) -> bool {
        !self.queue.is_empty()
    }

    pub fn set_preparation(&mut self, preparation: Preparation) {
        self.preparation = Some(preparation);
    }

    pub fn status(&self) -> &PipelineStatus {
        &self.status
    }

    pub fn generation(&self) -> Option<CompareGeneration> {
        self.active.as_ref().map(|active| active.generation)
    }

    pub fn manifest(&self) -> Option<&Rc<CompareManifestSnapshot>> {
        self.active.as_ref()?.manifest.as_ref()
    }

    pub fn prepared(&self, key: FileKey) -> Option<&PreparedDiff> {
        self.active.as_ref()?.loaded.get(&key)
    }

    pub fn slot_state(&self, key: FileKey) -> Option<SlotState> {
        Some(self.active.as_ref()?.placeholders.get(&key)?.state)
    }

    fn push(&mut self, install: ViewportInstall) {
        self.queue.push_back(PipelineInstall::View(install));
    }

    fn push_slot(&mut self, update: SlotUpdate) {
        self.queue.push_back(PipelineInstall::Slot(update));
    }

    fn pace(&mut self, mut reduction: Reduction) -> Reduction {
        let Some(generation) = self.generation() else {
            return reduction;
        };
        let cost: u64 = self.queue.iter().map(PipelineInstall::cost).sum();
        let more = cost > INSTALL_LINES_PER_FRAME || self.queue.len() > INSTALL_ITEMS_PER_FRAME;
        if more && !self.resuming {
            self.resuming = true;
            reduction = reduction.effect(PipelineEffect::ResumeInstalls(generation));
        }
        reduction
    }

    fn next_generation(&mut self) -> CompareGeneration {
        self.last_generation += 1;
        CompareGeneration(self.last_generation)
    }

    fn touch(&mut self, key: FileKey) {
        self.clock += 1;
        let clock = self.clock;
        if let Some(active) = self.active.as_mut() {
            active.used.insert(key, clock);
        }
    }

    fn start_compare(&mut self, intent: CompareIntent) -> Reduction {
        let generation = self.next_generation();
        let selected = self.active.as_ref().and_then(|active| active.selected);
        self.active = Some(ActiveCompare::new(generation, intent.clone(), selected));
        self.status = PipelineStatus::Comparing(None);
        self.queue.clear();
        self.push(ViewportInstall::Begin { generation });
        Reduction::changed().effect(PipelineEffect::RunCompare(Box::new(CompareJob {
            generation,
            intent,
            preparation: self.preparation,
        })))
    }

    fn current(&mut self, generation: CompareGeneration) -> Option<&mut ActiveCompare> {
        self.active
            .as_mut()
            .filter(|active| active.generation == generation)
    }

    fn finish_compare(
        &mut self,
        generation: CompareGeneration,
        result: CompareResult,
        cx: &SharedContext,
    ) -> Reduction {
        let Some(active) = self.current(generation) else {
            return stale(generation);
        };
        if result.manifest.generation != generation {
            return stale(generation);
        }
        let mut manifest = (*result.manifest).clone();
        if matches!(active.intent.request.spec, VcsCompareSpec::WorkingCopy) {
            tag_buckets(&mut manifest, cx);
        }
        let manifest = Rc::new(manifest);
        let files: Arc<[FileKey]> = manifest.files.iter().map(|file| file.key).collect();
        let pending_totals = manifest.totals.is_pending() && !manifest.files.is_empty();
        active.entries = result.entries;
        active.positions = manifest
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| (file.key, index))
            .collect();
        active.manifest = Some(manifest.clone());
        let mut prepared: HashMap<FileKey, PreparedDiff> = result
            .prepared
            .iter()
            .filter(|prepared| prepared.generation() == generation)
            .map(|prepared| (prepared.key(), prepared.clone()))
            .collect();
        let mut placeholders: HashMap<FileKey, Placeholder> =
            result.placeholders.iter().cloned().collect();
        let mut budget = (0u64, 0usize);
        let mut installs = Vec::with_capacity(files.len());
        for &key in files.iter() {
            if let Some(file) = prepared.remove(&key) {
                let lines = u64::from(file.estimated_lines());
                if budget.0 + lines <= WORKING_SET_LINES && budget.1 < WORKING_SET_FILES {
                    budget = (budget.0 + lines, budget.1 + 1);
                    active.loaded.insert(key, file.clone());
                    installs.push(PipelineInstall::View(ViewportInstall::File {
                        prepared: file,
                        remap: None,
                    }));
                    continue;
                }
                let placeholder = evicted_placeholder(&file);
                placeholders.insert(key, placeholder);
            }
            if let Some(placeholder) = placeholders.get(&key) {
                active.placeholders.insert(key, placeholder.clone());
                installs.push(PipelineInstall::Slot(SlotUpdate::Placeholder {
                    key,
                    placeholder: placeholder.clone(),
                }));
            }
        }
        installs.push(PipelineInstall::View(ViewportInstall::Order {
            generation,
            files,
        }));
        let selected = active.selected;
        self.queue.extend(installs);
        self.status = PipelineStatus::Ready;
        let mut reduction = Reduction::changed().intent(Intent::PublishManifest(manifest));
        if let Some(notice) = result.notice {
            reduction = reduction.intent(Intent::Toast(ToastUpdate::Show {
                id: None,
                kind: ToastKind::Info,
                message: notice,
                description: None,
            }));
        }
        if pending_totals {
            reduction.merge(self.load_totals());
        }
        if let Some(key) = selected {
            reduction.merge(self.load(key, CompareWorkPriority::InteractiveSelectedFile));
        }
        reduction
    }

    fn select(&mut self, key: Option<FileKey>) -> Reduction {
        let Some(active) = self.active.as_mut() else {
            return Reduction::default();
        };
        active.selected = key;
        match key {
            Some(key) => self.load(key, CompareWorkPriority::InteractiveSelectedFile),
            None => Reduction::default(),
        }
    }

    fn target(active: &ActiveCompare, key: FileKey) -> Option<FileTarget> {
        let manifest = active.manifest.as_ref()?;
        let index = *active.positions.get(&key)?;
        let file = manifest.files.get(index)?;
        Some(FileTarget {
            key,
            revision: file.revision,
            entry: active.entries.get(index)?.clone(),
        })
    }

    fn set_slot(&mut self, key: FileKey, state: SlotState) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let Some(placeholder) = active.placeholders.get_mut(&key) else {
            return;
        };
        if placeholder.state == state {
            return;
        }
        *placeholder = placeholder.with_state(state);
        let placeholder = placeholder.clone();
        self.push_slot(SlotUpdate::Placeholder { key, placeholder });
    }

    fn load(&mut self, key: FileKey, priority: CompareWorkPriority) -> Reduction {
        self.touch(key);
        let preparation = self.preparation;
        let Some(active) = self.active.as_mut() else {
            return Reduction::default();
        };
        if active.loaded.contains_key(&key) || active.loading.contains(&key) {
            return Reduction::default();
        }
        let Some(file) = Self::target(active, key) else {
            return Reduction::default();
        };
        active.loading.insert(key);
        let request = FileLoadRequest {
            generation: active.generation,
            repo_path: active.intent.repo_path.clone(),
            request: active.intent.request.clone(),
            file,
            priority,
            preparation,
        };
        self.set_slot(key, SlotState::Loading);
        Reduction::changed().effect(PipelineEffect::LoadFile(Box::new(request)))
    }

    fn load_stats(&mut self, keys: Vec<FileKey>, priority: CompareWorkPriority) -> Reduction {
        let Some(active) = self.active.as_mut() else {
            return Reduction::default();
        };
        let mut files = Vec::new();
        for key in keys {
            let pending = active
                .positions
                .get(&key)
                .zip(active.manifest.as_ref())
                .and_then(|(&index, manifest)| manifest.files.get(index))
                .is_some_and(|file| file.stats.is_pending());
            if !pending || active.stats_loading.contains(&key) {
                continue;
            }
            if let Some(target) = Self::target(active, key) {
                active.stats_loading.insert(key);
                files.push(target);
            }
        }
        if files.is_empty() {
            return Reduction::default();
        }
        Reduction::default().effect(PipelineEffect::LoadFileStats(Box::new(FileStatsRequest {
            generation: active.generation,
            repo_path: active.intent.repo_path.clone(),
            request: active.intent.request.clone(),
            files,
            priority,
        })))
    }

    fn load_totals(&mut self) -> Reduction {
        let Some(active) = self.active.as_mut() else {
            return Reduction::default();
        };
        let known = active
            .manifest
            .as_ref()
            .is_none_or(|manifest| !manifest.totals.is_pending());
        if known || active.totals_loading {
            return Reduction::default();
        }
        active.totals_loading = true;
        Reduction::default().effect(PipelineEffect::LoadTotalStats(Box::new(
            TotalStatsRequest {
                generation: active.generation,
                repo_path: active.intent.repo_path.clone(),
                request: active.intent.request.clone(),
            },
        )))
    }

    fn load_sources(&mut self, key: FileKey, purpose: SourcePurpose) -> Reduction {
        self.touch(key);
        let preparation = self.preparation;
        let Some(active) = self.active.as_mut() else {
            return Reduction::default();
        };
        let Some(prepared) = active.loaded.get(&key) else {
            return Reduction::default();
        };
        let summary = prepared.summary();
        let partial = prepared.snapshot().coverage() == SourceCoverage::PatchOnly;
        if !partial || summary.binary || active.sources_loading.contains(&key) {
            return Reduction::default();
        }
        let (Some(manifest), Some(file)) = (active.manifest.as_ref(), Self::target(active, key))
        else {
            return Reduction::default();
        };
        let (old_reference, new_reference) = match file.entry.bucket {
            Some(bucket) => {
                let (old, new) = bucket_refs(bucket);
                let old = old.map_or(String::new(), |old| {
                    if old == "HEAD" {
                        manifest.resolved_left.clone()
                    } else {
                        old.to_owned()
                    }
                });
                (old, new.to_owned())
            }
            None => (
                manifest.resolved_left.clone(),
                manifest.resolved_right.clone(),
            ),
        };
        let request = SourceLoadRequest {
            generation: active.generation,
            repo_path: active.intent.repo_path.clone(),
            file,
            prepared: prepared.clone(),
            old_reference,
            new_reference,
            purpose,
            preparation,
        };
        active.sources_loading.insert(key);
        Reduction::default().effect(PipelineEffect::LoadSources(Box::new(request)))
    }

    fn file_loaded(&mut self, generation: CompareGeneration, prepared: PreparedDiff) -> Reduction {
        let Some(active) = self.current(generation) else {
            return stale(generation);
        };
        let expected = active
            .positions
            .get(&prepared.key())
            .zip(active.manifest.as_ref())
            .and_then(|(&index, manifest)| manifest.files.get(index))
            .map(|file| file.revision);
        if prepared.generation() != generation || expected != Some(prepared.revision()) {
            return stale(generation);
        }
        let key = prepared.key();
        active.loading.remove(&key);
        active.sources_loading.remove(&key);
        active.placeholders.remove(&key);
        let stats = prepared.stats();
        active.loaded.insert(key, prepared.clone());
        self.touch(key);
        self.push(ViewportInstall::File {
            prepared,
            remap: None,
        });
        self.evict(key);
        let mut reduction = Reduction::changed();
        reduction.merge(self.update_stats(generation, &[(key, stats)]));
        reduction
    }

    fn evict(&mut self, keep: FileKey) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let mut lines: u64 = active
            .loaded
            .values()
            .map(|prepared| u64::from(prepared.estimated_lines()))
            .sum();
        let mut files = active.loaded.len();
        if lines <= WORKING_SET_LINES && files <= WORKING_SET_FILES {
            return;
        }
        let mut order: Vec<(u64, FileKey)> = active
            .loaded
            .keys()
            .filter(|&&key| key != keep && Some(key) != active.selected)
            .map(|&key| (active.used.get(&key).copied().unwrap_or(0), key))
            .collect();
        order.sort_unstable();
        let mut evicted = Vec::new();
        for (_, key) in order {
            if lines <= WORKING_SET_LINES && files <= WORKING_SET_FILES {
                break;
            }
            let Some(prepared) = active.loaded.remove(&key) else {
                continue;
            };
            lines = lines.saturating_sub(u64::from(prepared.estimated_lines()));
            files -= 1;
            let placeholder = evicted_placeholder(&prepared);
            active.placeholders.insert(key, placeholder);
            evicted.push(SlotUpdate::Evict {
                key,
                revision: prepared.revision(),
                estimated_lines: prepared.estimated_lines(),
            });
        }
        for update in evicted {
            self.push_slot(update);
        }
    }

    fn file_failed(
        &mut self,
        generation: CompareGeneration,
        key: FileKey,
        revision: FileRevision,
        failure: PipelineFailure,
    ) -> Reduction {
        let Some(active) = self.current(generation) else {
            return stale(generation);
        };
        active.loading.remove(&key);
        self.set_slot(key, SlotState::Failed);
        self.push(ViewportInstall::Failed {
            generation,
            key,
            revision,
            message: failure.message().to_owned(),
        });
        Reduction::changed()
    }

    fn update_stats(
        &mut self,
        generation: CompareGeneration,
        stats: &[(FileKey, FileStats)],
    ) -> Reduction {
        let Some(active) = self.current(generation) else {
            return stale(generation);
        };
        let Some(manifest) = active.manifest.as_ref() else {
            return Reduction::default();
        };
        let mut files: Option<Vec<ManifestFile>> = None;
        for &(key, value) in stats {
            active.stats_loading.remove(&key);
            if value.is_pending() {
                continue;
            }
            let Some(&index) = active.positions.get(&key) else {
                continue;
            };
            if manifest.files[index].stats == value {
                continue;
            }
            let files = files.get_or_insert_with(|| manifest.files.to_vec());
            files[index].stats = value;
        }
        let Some(files) = files else {
            return Reduction::default();
        };
        let updated = Rc::new(CompareManifestSnapshot {
            files: files.into(),
            ..(**manifest).clone()
        });
        active.manifest = Some(updated.clone());
        Reduction::changed().intent(Intent::PublishManifest(updated))
    }

    fn totals_ready(&mut self, generation: CompareGeneration, totals: FileStats) -> Reduction {
        let Some(active) = self.current(generation) else {
            return stale(generation);
        };
        active.totals_loading = false;
        let Some(manifest) = active.manifest.as_ref() else {
            return Reduction::default();
        };
        if manifest.totals == totals {
            return Reduction::default();
        }
        let updated = Rc::new(CompareManifestSnapshot {
            totals,
            ..(**manifest).clone()
        });
        active.manifest = Some(updated.clone());
        Reduction::changed().intent(Intent::PublishManifest(updated))
    }

    fn compare_failed(
        &mut self,
        generation: CompareGeneration,
        failure: PipelineFailure,
    ) -> Reduction {
        if self.current(generation).is_none() {
            return stale(generation);
        }
        let message = failure.message().to_owned();
        self.status = PipelineStatus::Failed(message.clone());
        Reduction::changed().intent(Intent::Toast(ToastUpdate::Show {
            id: None,
            kind: ToastKind::Error,
            message: "Comparison failed".to_owned(),
            description: Some(message),
        }))
    }

    fn event(&mut self, event: PipelineEvent, cx: &SharedContext) -> Reduction {
        match event {
            PipelineEvent::InstallsResumed { generation } => {
                self.resuming = false;
                if self.current(generation).is_none() {
                    return stale(generation);
                }
                Reduction::changed()
            }
            PipelineEvent::CompareProgress { generation, phase } => {
                if self.current(generation).is_none() || self.status == PipelineStatus::Ready {
                    return Reduction::default();
                }
                self.status = PipelineStatus::Comparing(Some(phase));
                Reduction::changed()
            }
            PipelineEvent::CompareFinished {
                generation, result, ..
            } => self.finish_compare(generation, result, cx),
            PipelineEvent::CompareFailed {
                generation,
                failure,
            } => self.compare_failed(generation, failure),
            PipelineEvent::TextCompareFinished {
                generation,
                prepared,
            } => {
                if prepared.generation() != generation {
                    return stale(generation);
                }
                self.push(ViewportInstall::TextCompare { prepared });
                Reduction::changed()
            }
            PipelineEvent::TextCompareFailed { failure, .. } => {
                Reduction::changed().intent(Intent::Toast(ToastUpdate::Show {
                    id: None,
                    kind: ToastKind::Error,
                    message: "Text comparison failed".to_owned(),
                    description: Some(failure.message().to_owned()),
                }))
            }
            PipelineEvent::TotalStatsReady {
                generation,
                additions,
                deletions,
            } => self.totals_ready(
                generation,
                FileStats::Known {
                    additions,
                    deletions,
                },
            ),
            PipelineEvent::TotalStatsFailed { generation, .. } => {
                if let Some(active) = self.current(generation) {
                    active.totals_loading = false;
                }
                Reduction::default()
            }
            PipelineEvent::FileLoaded {
                generation,
                prepared,
            }
            | PipelineEvent::StatusDiffReady {
                generation,
                prepared,
            }
            | PipelineEvent::SourcesLoaded {
                generation,
                prepared,
                ..
            } => self.file_loaded(generation, prepared),
            PipelineEvent::FileLoadFailed {
                generation,
                key,
                revision,
                failure,
            } => self.file_failed(generation, key, revision, failure),
            PipelineEvent::StatusDiffFailed {
                generation,
                key,
                failure,
            } => self.file_failed(generation, key, FileRevision(generation.0), failure),
            PipelineEvent::FileStatsReady {
                generation, stats, ..
            } => {
                let stats: Vec<_> = stats
                    .iter()
                    .map(|stat: &FileStat| {
                        (
                            stat.key,
                            FileStats::Known {
                                additions: stat.additions,
                                deletions: stat.deletions,
                            },
                        )
                    })
                    .collect();
                self.update_stats(generation, &stats)
            }
            PipelineEvent::FileStatsFailed { generation, .. } => {
                if let Some(active) = self.current(generation) {
                    active.stats_loading.clear();
                }
                Reduction::default()
            }
            PipelineEvent::SourcesFailed {
                generation, key, ..
            } => {
                let Some(active) = self.current(generation) else {
                    return stale(generation);
                };
                active.sources_loading.remove(&key);
                Reduction::default()
            }
        }
    }
}

fn evicted_placeholder(prepared: &PreparedDiff) -> Placeholder {
    Placeholder::new(
        Revision(prepared.revision().0),
        SlotState::Evicted,
        prepared.estimated_lines(),
        prepared.document().files().meta[0].clone(),
    )
}

fn tag_buckets(manifest: &mut CompareManifestSnapshot, cx: &SharedContext) {
    let Some(repository) = cx.repository.as_ref() else {
        return;
    };
    let mut buckets: HashMap<&str, Vec<ChangeBucket>> = HashMap::new();
    for change in &repository.snapshot.file_changes {
        let list = buckets.entry(change.path.as_str()).or_default();
        if !list.contains(&change.bucket) {
            list.push(change.bucket);
        }
    }
    if buckets.is_empty() {
        return;
    }
    let mut files = manifest.files.to_vec();
    for file in files.iter_mut().filter(|file| file.buckets.is_empty()) {
        let path = file.paths.display_path();
        if let Some(list) = buckets.get(path.as_ref()) {
            file.buckets = list.clone();
        }
    }
    manifest.files = files.into();
}

fn stale(generation: CompareGeneration) -> Reduction {
    tracing::debug!(generation = generation.0, "stale pipeline event ignored");
    Reduction::default()
}

impl Controller for PipelineController {
    type Action = PipelineAction;
    type Event = PipelineEvent;

    fn reduce_action(&mut self, action: Self::Action, _cx: &SharedContext) -> Reduction {
        let reduction = match action {
            PipelineAction::SetRenderer(renderer) => {
                let current = self
                    .active
                    .as_ref()
                    .map(|active| active.intent.clone())
                    .filter(|intent| intent.request.renderer != renderer);
                match current {
                    Some(mut intent) => {
                        intent.request.renderer = renderer;
                        self.start_compare(intent)
                    }
                    None => Reduction::default(),
                }
            }
            PipelineAction::RequestFiles { files, priority } => {
                let mut reduction = Reduction::default();
                for key in files {
                    reduction.merge(self.load(key, priority));
                }
                reduction
            }
            PipelineAction::RequestStats { files, priority } => self.load_stats(files, priority),
            PipelineAction::RequestTotals => self.load_totals(),
            PipelineAction::RequestSources { file, purpose } => self.load_sources(file, purpose),
        };
        self.pace(reduction)
    }

    fn reduce_event(&mut self, event: Self::Event, cx: &SharedContext) -> Reduction {
        let reduction = self.event(event, cx);
        self.pace(reduction)
    }

    fn apply_intent(&mut self, intent: &Intent, _cx: &SharedContext) -> Reduction {
        let reduction = match intent {
            Intent::RequestCompare(intent) => self.start_compare(intent.clone()),
            Intent::SelectFile(selected) => self.select(selected.map(|file| file.key)),
            Intent::ActivateWorkspace { .. } => {
                self.active = None;
                self.queue.clear();
                self.status = PipelineStatus::Idle;
                Reduction::changed()
            }
            _ => Reduction::default(),
        };
        self.pace(reduction)
    }
}
