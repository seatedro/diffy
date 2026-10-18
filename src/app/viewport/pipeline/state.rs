use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use crate::contracts::commands::{ToastKind, ToastUpdate};
use crate::contracts::shared::{CompareGeneration, FileKey, FileRevision, SharedContext};
use crate::contracts::shell::{Controller, Intent, Reduction};
use crate::contracts::viewport::PipelineAction;
use crate::contracts::viewport::PipelineEvent;
use crate::contracts::viewport::pipeline::{
    CompareIntent, CompareJob, CompareManifestSnapshot, CompareResult, CompareWorkPriority,
    FileLoadRequest, FileStat, FileStats, FileStatsRequest, FileTarget, ManifestFile,
    PipelineEffect, PipelineFailure, PreparedDiff, TextCompareRequest,
};
use crate::core::compare::{CompareEntry, ComparePhase};

pub type ViewportInstall = crate::contracts::viewport::pipeline::ViewportInstall;

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
    loading: HashSet<FileKey>,
    stats_loading: HashSet<FileKey>,
    selected: Option<FileKey>,
}

#[derive(Debug, Default)]
pub struct PipelineController {
    last_generation: u64,
    active: Option<ActiveCompare>,
    text_generation: Option<CompareGeneration>,
    status: PipelineStatus,
    installs: Vec<ViewportInstall>,
}

impl PipelineController {
    pub fn take_installs(&mut self) -> Vec<ViewportInstall> {
        std::mem::take(&mut self.installs)
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

    pub fn request_text_compare(&mut self, mut request: TextCompareRequest) -> Reduction {
        let generation = self.next_generation();
        request.generation = generation;
        self.text_generation = Some(generation);
        Reduction::changed().effect(PipelineEffect::RunTextCompare(Box::new(request)))
    }

    fn next_generation(&mut self) -> CompareGeneration {
        self.last_generation += 1;
        CompareGeneration(self.last_generation)
    }

    fn start_compare(&mut self, intent: CompareIntent) -> Reduction {
        let generation = self.next_generation();
        let selected = self.active.as_ref().and_then(|active| active.selected);
        self.active = Some(ActiveCompare {
            generation,
            intent: intent.clone(),
            manifest: None,
            entries: Arc::from([]),
            loaded: HashMap::new(),
            loading: HashSet::new(),
            stats_loading: HashSet::new(),
            selected,
        });
        self.status = PipelineStatus::Comparing(None);
        self.installs.push(ViewportInstall::Begin { generation });
        Reduction::changed().effect(PipelineEffect::RunCompare(Box::new(CompareJob {
            generation,
            intent,
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
    ) -> Reduction {
        let Some(active) = self.current(generation) else {
            return stale(generation);
        };
        if result.manifest.generation != generation {
            return stale(generation);
        }
        let manifest = Rc::new((*result.manifest).clone());
        let files: Arc<[FileKey]> = manifest.files.iter().map(|file| file.key).collect();
        active.entries = result.entries;
        active.manifest = Some(manifest.clone());
        let mut installs = vec![ViewportInstall::Order { generation, files }];
        for prepared in result.prepared.iter() {
            if prepared.generation() != generation {
                continue;
            }
            active.loaded.insert(prepared.key(), prepared.clone());
            installs.push(ViewportInstall::File {
                prepared: prepared.clone(),
                remap: None,
            });
        }
        let selected = active.selected;
        self.installs.extend(installs);
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
        let file = active.manifest.as_ref()?.file(key)?;
        let index = active
            .manifest
            .as_ref()?
            .files
            .iter()
            .position(|candidate| candidate.key == key)?;
        Some(FileTarget {
            key,
            revision: file.revision,
            entry: active.entries.get(index)?.clone(),
        })
    }

    fn load(&mut self, key: FileKey, priority: CompareWorkPriority) -> Reduction {
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
        Reduction::default().effect(PipelineEffect::LoadFile(Box::new(FileLoadRequest {
            generation: active.generation,
            repo_path: active.intent.repo_path.clone(),
            request: active.intent.request.clone(),
            file,
            priority,
        })))
    }

    fn load_stats(&mut self, keys: Vec<FileKey>, priority: CompareWorkPriority) -> Reduction {
        let Some(active) = self.active.as_mut() else {
            return Reduction::default();
        };
        let mut files = Vec::new();
        for key in keys {
            let pending = active
                .manifest
                .as_ref()
                .and_then(|manifest| manifest.file(key))
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

    fn file_loaded(&mut self, generation: CompareGeneration, prepared: PreparedDiff) -> Reduction {
        let Some(active) = self.current(generation) else {
            return stale(generation);
        };
        let expected = active
            .manifest
            .as_ref()
            .and_then(|manifest| manifest.file(prepared.key()))
            .map(|file| file.revision);
        if prepared.generation() != generation || expected != Some(prepared.revision()) {
            return stale(generation);
        }
        active.loading.remove(&prepared.key());
        let stats = prepared.stats();
        let key = prepared.key();
        active.loaded.insert(key, prepared.clone());
        self.installs.push(ViewportInstall::File {
            prepared,
            remap: None,
        });
        let mut reduction = Reduction::changed();
        reduction.merge(self.update_stats(generation, &[(key, stats)]));
        reduction
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
        self.installs.push(ViewportInstall::Failed {
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
            let Some(index) = manifest.files.iter().position(|file| file.key == key) else {
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
}

fn stale(generation: CompareGeneration) -> Reduction {
    tracing::debug!(generation = generation.0, "stale pipeline event ignored");
    Reduction::default()
}

impl Controller for PipelineController {
    type Action = PipelineAction;
    type Event = PipelineEvent;

    fn reduce_action(&mut self, action: Self::Action, _cx: &SharedContext) -> Reduction {
        match action {
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
        }
    }

    fn reduce_event(&mut self, event: Self::Event, _cx: &SharedContext) -> Reduction {
        match event {
            PipelineEvent::CompareProgress { generation, phase } => {
                if self.current(generation).is_none() || self.status == PipelineStatus::Ready {
                    return Reduction::default();
                }
                self.status = PipelineStatus::Comparing(Some(phase));
                Reduction::changed()
            }
            PipelineEvent::CompareFinished {
                generation, result, ..
            } => self.finish_compare(generation, result),
            PipelineEvent::CompareFailed {
                generation,
                failure,
            } => self.compare_failed(generation, failure),
            PipelineEvent::TextCompareFinished {
                generation,
                prepared,
            } => {
                if self.text_generation != Some(generation) || prepared.generation() != generation {
                    return stale(generation);
                }
                self.installs
                    .push(ViewportInstall::TextCompare { prepared });
                Reduction::changed()
            }
            PipelineEvent::TextCompareFailed {
                generation,
                failure,
            } => {
                if self.text_generation != Some(generation) {
                    return stale(generation);
                }
                Reduction::changed().intent(Intent::Toast(ToastUpdate::Show {
                    id: None,
                    kind: ToastKind::Error,
                    message: "Text comparison failed".to_owned(),
                    description: Some(failure.message().to_owned()),
                }))
            }
            PipelineEvent::TotalStatsReady { .. } | PipelineEvent::TotalStatsFailed { .. } => {
                Reduction::default()
            }
            PipelineEvent::FileLoaded {
                generation,
                prepared,
            }
            | PipelineEvent::StatusDiffReady {
                generation,
                prepared,
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
                    .map(
                        |&FileStat {
                             key,
                             additions,
                             deletions,
                         }| {
                            (
                                key,
                                FileStats::Known {
                                    additions,
                                    deletions,
                                },
                            )
                        },
                    )
                    .collect();
                self.update_stats(generation, &stats)
            }
            PipelineEvent::FileStatsFailed { generation, .. } => {
                if let Some(active) = self.current(generation) {
                    active.stats_loading.clear();
                }
                Reduction::default()
            }
            PipelineEvent::SourcesLoaded {
                generation,
                prepared,
                ..
            } => self.file_loaded(generation, prepared),
            PipelineEvent::SourcesFailed { generation, .. } => {
                if self.current(generation).is_none() {
                    return stale(generation);
                }
                Reduction::default()
            }
        }
    }

    fn apply_intent(&mut self, intent: &Intent, _cx: &SharedContext) -> Reduction {
        match intent {
            Intent::RequestCompare(intent) => self.start_compare(intent.clone()),
            Intent::SelectFile(selected) => self.select(selected.map(|file| file.key)),
            Intent::ActivateWorkspace { .. } => {
                self.active = None;
                self.status = PipelineStatus::Idle;
                Reduction::changed()
            }
            _ => Reduction::default(),
        }
    }
}
