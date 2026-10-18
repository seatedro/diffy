use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

use crate::apprt::services::viewport::PipelineServices;
use crate::apprt::transport::{ProgressGate, ProgressKey, ScopedEventSender};
use crate::contracts::shared::{CompareGeneration, FileKey};
use crate::contracts::viewport::pipeline::{
    CompareWorkPriority, FileLoadRequest, FileStat, FileStatsRequest, PipelineEffect,
    PipelineEvent, TotalStatsRequest,
};
use crate::core::compare::{ComparePhase, ProgressSink};

const FILE_STATS_STREAM_CHUNK_SIZE: usize = 8_192;
const MAX_COMPARE_WORKERS: usize = 4;

pub struct PipelineRuntime {
    services: PipelineServices,
    queue: Arc<PipelineQueue>,
    progress: Arc<ProgressGate>,
}

impl PipelineRuntime {
    pub fn new(services: PipelineServices) -> Self {
        let queue = Arc::new(PipelineQueue::default());
        let worker_count = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .clamp(1, MAX_COMPARE_WORKERS);
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            thread::spawn(move || {
                loop {
                    let job = queue.next();
                    if !queue.is_stale(job.job.generation()) {
                        job.run(services, &queue);
                    }
                }
            });
        }
        Self {
            services,
            queue,
            progress: Arc::new(ProgressGate::default()),
        }
    }

    pub fn dispatch(&self, effect: PipelineEffect, events: ScopedEventSender) {
        let services = self.services;
        match effect {
            PipelineEffect::RunCompare(job) => {
                let generation = job.generation;
                self.queue.set_epoch(generation);
                let queue = Arc::clone(&self.queue);
                let progress = CompareProgress {
                    events: events.clone(),
                    generation,
                    gate: Arc::clone(&self.progress),
                };
                thread::spawn(move || {
                    let event = match services.run_compare(&job, &progress) {
                        Ok(manifest) => PipelineEvent::CompareFinished {
                            generation,
                            request: job.intent.request,
                            manifest,
                        },
                        Err(failure) => PipelineEvent::CompareFailed {
                            generation,
                            failure,
                        },
                    };
                    queue.send_current(generation, &events, event);
                });
            }
            PipelineEffect::RunTextCompare(request) => {
                let generation = request.generation;
                self.queue.set_epoch(generation);
                let queue = Arc::clone(&self.queue);
                thread::spawn(move || {
                    let event = match services.run_text_compare(&request) {
                        Ok(prepared) => PipelineEvent::TextCompareFinished {
                            generation,
                            prepared,
                        },
                        Err(failure) => PipelineEvent::TextCompareFailed {
                            generation,
                            failure,
                        },
                    };
                    queue.send_current(generation, &events, event);
                });
            }
            PipelineEffect::SupersedeCompare(generation) => self.queue.set_epoch(generation),
            PipelineEffect::LoadTotalStats(request) => {
                self.queue.enqueue(PipelineJob::TotalStats(request), events);
            }
            PipelineEffect::LoadFile(request) => {
                self.queue.enqueue(PipelineJob::File(request), events);
            }
            PipelineEffect::LoadFileStats(request) => {
                self.queue.enqueue(PipelineJob::FileStats(request), events);
            }
            PipelineEffect::LoadStatusDiff(request) => {
                let queue = Arc::clone(&self.queue);
                thread::spawn(move || {
                    let generation = request.generation;
                    let event = match services.load_status_diff(&request) {
                        Ok(prepared) => PipelineEvent::StatusDiffReady {
                            generation,
                            prepared,
                        },
                        Err(failure) => PipelineEvent::StatusDiffFailed {
                            generation,
                            key: request.key,
                            failure,
                        },
                    };
                    queue.send_current(generation, &events, event);
                });
            }
            PipelineEffect::LoadSources(request) => {
                let queue = Arc::clone(&self.queue);
                thread::spawn(move || {
                    let generation = request.generation;
                    let key = request.file.key;
                    let revision = request.file.revision;
                    let event = match services.load_sources(&request) {
                        Ok(sources) => PipelineEvent::SourcesLoaded {
                            generation,
                            key,
                            revision,
                            purpose: request.purpose,
                            sources,
                        },
                        Err(failure) => PipelineEvent::SourcesFailed {
                            generation,
                            key,
                            revision,
                            failure,
                        },
                    };
                    queue.send_current(generation, &events, event);
                });
            }
        }
    }
}

struct CompareProgress {
    events: ScopedEventSender,
    generation: CompareGeneration,
    gate: Arc<ProgressGate>,
}

impl ProgressSink for CompareProgress {
    fn phase(&self, phase: ComparePhase) {
        if self.gate.admit(
            ProgressKey::compare(self.generation.0, &phase),
            Instant::now(),
        ) {
            self.events.send(PipelineEvent::CompareProgress {
                generation: self.generation,
                phase,
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JobKey {
    TotalStats {
        generation: CompareGeneration,
    },
    File {
        generation: CompareGeneration,
        key: FileKey,
    },
    FileStats {
        generation: CompareGeneration,
        priority: CompareWorkPriority,
    },
}

enum PipelineJob {
    TotalStats(Box<TotalStatsRequest>),
    File(Box<FileLoadRequest>),
    FileStats(Box<FileStatsRequest>),
}

impl PipelineJob {
    fn generation(&self) -> CompareGeneration {
        match self {
            Self::TotalStats(request) => request.generation,
            Self::File(request) => request.generation,
            Self::FileStats(request) => request.generation,
        }
    }

    fn priority(&self) -> CompareWorkPriority {
        match self {
            Self::TotalStats(_) => CompareWorkPriority::TotalStats,
            Self::File(request) => request.priority,
            Self::FileStats(request) => request.priority,
        }
    }

    fn key(&self) -> JobKey {
        match self {
            Self::TotalStats(request) => JobKey::TotalStats {
                generation: request.generation,
            },
            Self::File(request) => JobKey::File {
                generation: request.generation,
                key: request.file.key,
            },
            Self::FileStats(request) => JobKey::FileStats {
                generation: request.generation,
                priority: request.priority,
            },
        }
    }
}

struct QueuedJob {
    sequence: u64,
    key: JobKey,
    priority: CompareWorkPriority,
    job: PipelineJob,
    events: ScopedEventSender,
}

impl QueuedJob {
    fn run(self, services: PipelineServices, queue: &PipelineQueue) {
        let events = self.events;
        match self.job {
            PipelineJob::TotalStats(request) => {
                let generation = request.generation;
                let event = match services.load_total_stats(&request) {
                    Ok((additions, deletions)) => PipelineEvent::TotalStatsReady {
                        generation,
                        additions,
                        deletions,
                    },
                    Err(failure) => PipelineEvent::TotalStatsFailed {
                        generation,
                        failure,
                    },
                };
                queue.send_current(generation, &events, event);
            }
            PipelineJob::File(request) => {
                let generation = request.generation;
                let event = match services.load_file(&request) {
                    Ok(prepared) => PipelineEvent::FileLoaded {
                        generation,
                        prepared,
                    },
                    Err(failure) => PipelineEvent::FileLoadFailed {
                        generation,
                        key: request.file.key,
                        revision: request.file.revision,
                        failure,
                    },
                };
                queue.send_current(generation, &events, event);
            }
            PipelineJob::FileStats(request) => {
                let generation = request.generation;
                match services.load_file_stats(&request) {
                    Ok(stats) => queue.send_file_stats(generation, &events, stats),
                    Err(failure) => queue.send_current(
                        generation,
                        &events,
                        PipelineEvent::FileStatsFailed {
                            generation,
                            failure,
                        },
                    ),
                }
            }
        }
    }
}

#[derive(Default)]
struct QueueState {
    jobs: Vec<QueuedJob>,
    next_sequence: u64,
}

#[derive(Default)]
struct PipelineQueue {
    state: Mutex<QueueState>,
    ready: Condvar,
    current_epoch: AtomicU64,
}

impl PipelineQueue {
    fn lock(&self) -> MutexGuard<'_, QueueState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_stale(&self, generation: CompareGeneration) -> bool {
        generation.0 < self.current_epoch.load(Ordering::Acquire)
    }

    fn set_epoch(&self, generation: CompareGeneration) {
        let previous = self.current_epoch.fetch_max(generation.0, Ordering::AcqRel);
        if generation.0 < previous {
            return;
        }
        let mut state = self.lock();
        state
            .jobs
            .retain(|job| !self.is_stale(job.job.generation()));
    }

    fn enqueue(&self, job: PipelineJob, events: ScopedEventSender) {
        let generation = job.generation();
        let previous = self.current_epoch.fetch_max(generation.0, Ordering::AcqRel);
        let key = job.key();
        let priority = job.priority();
        let mut state = self.lock();
        state
            .jobs
            .retain(|job| !self.is_stale(job.job.generation()));
        if generation.0 < previous || self.is_stale(generation) {
            return;
        }
        state.jobs.retain(|queued| queued.key != key);
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        state.jobs.push(QueuedJob {
            sequence,
            key,
            priority,
            job,
            events,
        });
        drop(state);
        self.ready.notify_one();
    }

    fn next(&self) -> QueuedJob {
        let mut state = self.lock();
        loop {
            state
                .jobs
                .retain(|job| !self.is_stale(job.job.generation()));
            if let Some(index) = next_job_index(&state.jobs) {
                return state.jobs.swap_remove(index);
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn send_current(
        &self,
        generation: CompareGeneration,
        events: &ScopedEventSender,
        event: PipelineEvent,
    ) {
        if self.is_stale(generation) {
            tracing::debug!(generation = generation.0, "stale pipeline result dropped");
            return;
        }
        events.send(event);
    }

    fn send_file_stats(
        &self,
        generation: CompareGeneration,
        events: &ScopedEventSender,
        stats: Vec<FileStat>,
    ) {
        if stats.len() <= FILE_STATS_STREAM_CHUNK_SIZE {
            self.send_current(
                generation,
                events,
                PipelineEvent::FileStatsReady {
                    generation,
                    stats: stats.into(),
                    complete: true,
                },
            );
            return;
        }
        for chunk in stats.chunks(FILE_STATS_STREAM_CHUNK_SIZE) {
            if self.is_stale(generation) {
                tracing::debug!(generation = generation.0, "stale file stats stream dropped");
                return;
            }
            events.send(PipelineEvent::FileStatsReady {
                generation,
                stats: chunk.into(),
                complete: false,
            });
        }
        self.send_current(
            generation,
            events,
            PipelineEvent::FileStatsReady {
                generation,
                stats: Arc::from([]),
                complete: true,
            },
        );
    }
}

fn next_job_index(jobs: &[QueuedJob]) -> Option<usize> {
    jobs.iter()
        .enumerate()
        .max_by_key(|(_, job)| (job.priority.rank(), std::cmp::Reverse(job.sequence)))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc;

    use super::*;
    use crate::apprt::transport::RuntimeEventSender;
    use crate::contracts::shared::{FileRevision, WorkspaceEpoch};
    use crate::contracts::viewport::pipeline::FileTarget;
    use crate::core::compare::stats::CompareFilePaths;
    use crate::core::compare::{LayoutMode, RendererKind};
    use crate::core::vcs::model::{VcsCompareRequest, VcsCompareSpec};

    fn events() -> ScopedEventSender {
        let (tx, _rx) = mpsc::channel();
        RuntimeEventSender::from_channel(tx).scoped(WorkspaceEpoch(1))
    }

    fn file_job(generation: u64, key: u64, priority: CompareWorkPriority) -> PipelineJob {
        PipelineJob::File(Box::new(FileLoadRequest {
            generation: CompareGeneration(generation),
            repo_path: PathBuf::from("/repo"),
            request: VcsCompareRequest {
                spec: VcsCompareSpec::WorkingCopy,
                layout: LayoutMode::Unified,
                renderer: RendererKind::Builtin,
            },
            file: FileTarget {
                key: FileKey(key),
                revision: FileRevision(1),
                paths: CompareFilePaths::from_paths(Some("a.rs"), Some("a.rs")),
            },
            priority,
        }))
    }

    #[test]
    fn newer_generation_drops_queued_older_jobs() {
        let queue = PipelineQueue::default();
        queue.enqueue(file_job(1, 1, CompareWorkPriority::Overscan), events());
        queue.enqueue(file_job(1, 2, CompareWorkPriority::Overscan), events());
        queue.enqueue(file_job(2, 3, CompareWorkPriority::Overscan), events());

        let state = queue.lock();
        assert_eq!(state.jobs.len(), 1);
        assert_eq!(state.jobs[0].job.generation(), CompareGeneration(2));
    }

    #[test]
    fn superseded_generation_rejects_late_jobs() {
        let queue = PipelineQueue::default();
        queue.enqueue(file_job(1, 1, CompareWorkPriority::Overscan), events());
        queue.set_epoch(CompareGeneration(3));
        queue.enqueue(file_job(2, 2, CompareWorkPriority::Overscan), events());

        assert!(queue.lock().jobs.is_empty());
        assert!(queue.is_stale(CompareGeneration(2)));
    }

    #[test]
    fn interactive_file_runs_before_older_overscan() {
        let queue = PipelineQueue::default();
        queue.enqueue(file_job(1, 1, CompareWorkPriority::Overscan), events());
        queue.enqueue(
            file_job(1, 2, CompareWorkPriority::InteractiveSelectedFile),
            events(),
        );
        queue.enqueue(file_job(1, 1, CompareWorkPriority::Warmup), events());

        let first = queue.next();
        assert_eq!(first.priority, CompareWorkPriority::InteractiveSelectedFile);
        let second = queue.next();
        assert_eq!(second.priority, CompareWorkPriority::Warmup);
        assert!(queue.lock().jobs.is_empty());
    }
}
