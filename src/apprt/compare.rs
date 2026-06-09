use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use super::runtime::RuntimeEventSender;
use super::services::AppServices;
use crate::effects::{
    CompareFileRequest, CompareFileStatsRequest, CompareStatsRequest, CompareWorkPriority, Task,
};
use crate::events::{CompareEvent, CompareFileStatsReady};

const FILE_STATS_STREAM_CHUNK_SIZE: usize = 8_192;
const MAX_COMPARE_WORKERS: usize = 4;

pub(crate) struct CompareScheduler {
    queue: Arc<CompareQueue>,
}

struct CompareQueue {
    state: Mutex<CompareQueueState>,
    ready: Condvar,
    /// Highest compare generation observed. Jobs and results stamped with an
    /// older generation are stale (a newer compare superseded them) and are
    /// dropped instead of being run or emitted, mirroring `SyntaxScheduler`.
    current_epoch: AtomicU64,
}

#[derive(Default)]
struct CompareQueueState {
    jobs: Vec<QueuedCompareJob>,
    next_sequence: u64,
}

struct QueuedCompareJob {
    sequence: u64,
    key: CompareJobKey,
    priority: CompareWorkPriority,
    job: CompareJob,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompareJobKey {
    TotalStats {
        generation: u64,
    },
    File {
        generation: u64,
        index: usize,
        path: String,
    },
    FileStats {
        generation: u64,
        priority: CompareWorkPriority,
    },
}

enum CompareJob {
    Stats(Task<CompareStatsRequest>),
    File(Task<CompareFileRequest>),
    FileStats(Task<CompareFileStatsRequest>),
}

impl CompareScheduler {
    pub(crate) fn new(services: AppServices, event_sender: RuntimeEventSender) -> Self {
        let queue = Arc::new(CompareQueue {
            state: Mutex::new(CompareQueueState::default()),
            ready: Condvar::new(),
            current_epoch: AtomicU64::new(0),
        });
        let worker_count = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .clamp(1, MAX_COMPARE_WORKERS);
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let services = services.clone();
            let event_sender = event_sender.clone();
            thread::spawn(move || compare_worker_loop(queue, services, event_sender));
        }
        Self { queue }
    }

    pub(crate) fn dispatch_load_stats(&self, task: Task<CompareStatsRequest>) {
        self.enqueue(CompareJob::Stats(task));
    }

    pub(crate) fn dispatch_load_file(&self, task: Task<CompareFileRequest>) {
        self.enqueue(CompareJob::File(task));
    }

    pub(crate) fn dispatch_load_file_stats(&self, task: Task<CompareFileStatsRequest>) {
        self.enqueue(CompareJob::FileStats(task));
    }

    /// Mark every compare job older than `epoch` stale. Called when a new
    /// compare request starts so rapid file/rev switching cannot leave older
    /// queued work racing newer state.
    pub(crate) fn set_epoch(&self, epoch: u64) {
        let previous = self.queue.current_epoch.fetch_max(epoch, Ordering::AcqRel);
        if epoch < previous {
            return;
        }
        let mut state = self.queue.state.lock().expect("compare queue poisoned");
        let current = self.queue.current_epoch.load(Ordering::Acquire);
        state.jobs.retain(|job| job.job.generation() >= current);
    }

    fn enqueue(&self, job: CompareJob) {
        let epoch = job.generation();
        let previous = self.queue.current_epoch.fetch_max(epoch, Ordering::AcqRel);

        let key = job.key();
        let priority = job.priority();
        let mut state = self.queue.state.lock().expect("compare queue poisoned");
        let current = self.queue.current_epoch.load(Ordering::Acquire);
        state.jobs.retain(|job| job.job.generation() >= current);
        if epoch < previous || epoch < current {
            return;
        }
        state.jobs.retain(|job| job.key != key);
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        state.jobs.push(QueuedCompareJob {
            sequence,
            key,
            priority,
            job,
        });
        drop(state);
        self.queue.ready.notify_one();
    }
}

impl CompareJob {
    fn generation(&self) -> u64 {
        match self {
            CompareJob::Stats(task) => task.generation,
            CompareJob::File(task) => task.generation,
            CompareJob::FileStats(task) => task.generation,
        }
    }

    fn priority(&self) -> CompareWorkPriority {
        match self {
            CompareJob::Stats(task) => task.request.priority,
            CompareJob::File(task) => task.request.priority,
            CompareJob::FileStats(task) => task.request.priority,
        }
    }

    fn key(&self) -> CompareJobKey {
        match self {
            CompareJob::Stats(task) => CompareJobKey::TotalStats {
                generation: task.generation,
            },
            CompareJob::File(task) => CompareJobKey::File {
                generation: task.generation,
                index: task.request.index,
                path: task.request.path.clone(),
            },
            CompareJob::FileStats(task) => CompareJobKey::FileStats {
                generation: task.generation,
                priority: task.request.priority,
            },
        }
    }
}

fn compare_worker_loop(
    queue: Arc<CompareQueue>,
    services: AppServices,
    event_sender: RuntimeEventSender,
) {
    loop {
        let job = {
            let mut state = queue.state.lock().expect("compare queue poisoned");
            loop {
                let current_epoch = queue.current_epoch.load(Ordering::Acquire);
                state
                    .jobs
                    .retain(|job| job.job.generation() >= current_epoch);
                if let Some(index) = next_job_index(&state.jobs) {
                    break state.jobs.swap_remove(index);
                }
                state = queue.ready.wait(state).expect("compare queue poisoned");
            }
        };
        if job.job.generation() < queue.current_epoch.load(Ordering::Acquire) {
            continue;
        }
        run_job(job, &services, &event_sender, &queue.current_epoch);
    }
}

fn next_job_index(jobs: &[QueuedCompareJob]) -> Option<usize> {
    jobs.iter()
        .enumerate()
        .max_by_key(|(_, job)| (job.priority.rank(), std::cmp::Reverse(job.sequence)))
        .map(|(index, _)| index)
}

fn run_job(
    job: QueuedCompareJob,
    services: &AppServices,
    event_sender: &RuntimeEventSender,
    current_epoch: &AtomicU64,
) {
    match job.job {
        CompareJob::Stats(task) => run_load_stats(task, services, event_sender, current_epoch),
        CompareJob::File(task) => run_load_file(task, services, event_sender, current_epoch),
        CompareJob::FileStats(task) => {
            run_load_file_stats(task, services, event_sender, current_epoch)
        }
    }
}

fn is_stale(generation: u64, current_epoch: &AtomicU64) -> bool {
    generation < current_epoch.load(Ordering::Acquire)
}

fn run_load_stats(
    task: Task<CompareStatsRequest>,
    services: &AppServices,
    event_sender: &RuntimeEventSender,
    current_epoch: &AtomicU64,
) {
    let generation = task.generation;
    let request = task.request;
    let event = match services.load_compare_stats(generation, request) {
        Ok(payload) => CompareEvent::CompareStatsReady(payload),
        Err(error) => CompareEvent::CompareStatsFailed {
            generation,
            message: error.to_string(),
        },
    };
    if is_stale(generation, current_epoch) {
        tracing::debug!(generation, "stale compare stats result dropped");
        return;
    }
    event_sender.send(event);
}

fn run_load_file(
    task: Task<CompareFileRequest>,
    services: &AppServices,
    event_sender: &RuntimeEventSender,
    current_epoch: &AtomicU64,
) {
    let generation = task.generation;
    let request = task.request;
    let path = request.path.clone();
    let event = match services.load_compare_file(generation, request) {
        Ok(payload) => CompareEvent::CompareFileFinished(payload),
        Err(error) => CompareEvent::CompareFileFailed {
            generation,
            path: path.clone(),
            message: error.to_string(),
        },
    };
    if is_stale(generation, current_epoch) {
        tracing::debug!(generation, path = %path, "stale compare file result dropped");
        return;
    }
    event_sender.send(event);
}

fn run_load_file_stats(
    task: Task<CompareFileStatsRequest>,
    services: &AppServices,
    event_sender: &RuntimeEventSender,
    current_epoch: &AtomicU64,
) {
    let generation = task.generation;
    let request = task.request;
    let payload = match services.load_compare_file_stats(generation, request) {
        Ok(payload) => payload,
        Err(error) => {
            if is_stale(generation, current_epoch) {
                return;
            }
            event_sender.send(CompareEvent::CompareFileStatsFailed {
                generation,
                message: error.to_string(),
            });
            return;
        }
    };
    if is_stale(generation, current_epoch) {
        tracing::debug!(generation, "stale compare file stats result dropped");
        return;
    }
    send_file_stats_payload(generation, payload, event_sender, current_epoch);
}

fn send_file_stats_payload(
    generation: u64,
    payload: CompareFileStatsReady,
    event_sender: &RuntimeEventSender,
    current_epoch: &AtomicU64,
) {
    if payload.stats.len() <= FILE_STATS_STREAM_CHUNK_SIZE {
        event_sender.send(CompareEvent::CompareFileStatsReady(payload));
        return;
    }

    let mut stats = payload.stats.into_iter();
    loop {
        // A newer compare invalidates the remainder of the stream; the
        // state-side generation guard ignores any chunks already sent.
        if is_stale(generation, current_epoch) {
            tracing::debug!(generation, "stale compare file stats stream dropped");
            return;
        }
        let chunk = stats
            .by_ref()
            .take(FILE_STATS_STREAM_CHUNK_SIZE)
            .collect::<Vec<_>>();
        if chunk.is_empty() {
            break;
        }
        event_sender.send(CompareEvent::CompareFileStatsReady(CompareFileStatsReady {
            generation,
            stats: chunk,
            request_complete: false,
        }));
    }
    event_sender.send(CompareEvent::CompareFileStatsReady(CompareFileStatsReady {
        generation,
        stats: Vec::new(),
        request_complete: true,
    }));
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::core::compare::{LayoutMode, RendererKind};
    use crate::core::vcs::model::{VcsCompareRequest, VcsCompareSpec};
    use crate::effects::CompareWorkPriority;

    fn scheduler_for_test() -> CompareScheduler {
        CompareScheduler {
            queue: Arc::new(CompareQueue {
                state: Mutex::new(CompareQueueState::default()),
                ready: Condvar::new(),
                current_epoch: AtomicU64::new(0),
            }),
        }
    }

    fn file_task(generation: u64, index: usize) -> Task<CompareFileRequest> {
        Task {
            generation,
            request: CompareFileRequest {
                repo_path: PathBuf::from("/repo"),
                request: VcsCompareRequest {
                    spec: VcsCompareSpec::WorkingCopy,
                    layout: LayoutMode::Unified,
                    renderer: RendererKind::Builtin,
                },
                path: format!("src/file-{index}.rs"),
                index,
                deferred_file: None,
                priority: CompareWorkPriority::InteractiveSelectedFile,
            },
        }
    }

    #[test]
    fn newer_compare_generation_drops_queued_older_jobs() {
        let scheduler = scheduler_for_test();
        scheduler.dispatch_load_file(file_task(1, 1));
        scheduler.dispatch_load_file(file_task(1, 2));

        scheduler.dispatch_load_file(file_task(2, 3));

        let state = scheduler
            .queue
            .state
            .lock()
            .expect("compare queue poisoned");
        assert_eq!(scheduler.queue.current_epoch.load(Ordering::Acquire), 2);
        assert_eq!(state.jobs.len(), 1);
        assert_eq!(state.jobs[0].job.generation(), 2);
    }

    #[test]
    fn explicit_compare_epoch_drops_queued_older_jobs() {
        let scheduler = scheduler_for_test();
        scheduler.dispatch_load_file(file_task(1, 1));
        scheduler.dispatch_load_file(file_task(1, 2));

        scheduler.set_epoch(2);
        scheduler.dispatch_load_file(file_task(1, 3));
        scheduler.dispatch_load_file(file_task(2, 4));

        let state = scheduler
            .queue
            .state
            .lock()
            .expect("compare queue poisoned");
        assert_eq!(scheduler.queue.current_epoch.load(Ordering::Acquire), 2);
        assert_eq!(state.jobs.len(), 1);
        assert_eq!(state.jobs[0].job.generation(), 2);
        match &state.jobs[0].job {
            CompareJob::File(task) => assert_eq!(task.request.index, 4),
            other => panic!("unexpected job kind: {:?}", other.key()),
        }
    }

    #[test]
    fn visible_diff_work_outprioritizes_stats_work() {
        assert!(
            CompareWorkPriority::InteractiveSelectedFile.rank()
                > CompareWorkPriority::VisibleViewportDiff.rank()
        );
        assert!(
            CompareWorkPriority::VisibleViewportDiff.rank()
                > CompareWorkPriority::VisibleSidebarStats.rank()
        );
        assert!(
            CompareWorkPriority::VisibleSidebarStats.rank()
                > CompareWorkPriority::TotalStats.rank()
        );
    }
}
