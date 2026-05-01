use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Instant;

use super::runtime::RuntimeEventSender;
use super::services::AppServices;
use crate::core::perf::PerfSpan;
use crate::effects::{
    CompareFileRequest, CompareFileStatsRequest, CompareStatsRequest, CompareWorkPriority, Task,
};
use crate::events::{CompareEvent, CompareFileStatsReady};

const FILE_STATS_STREAM_CHUNK_SIZE: usize = 16;
const MAX_COMPARE_WORKERS: usize = 4;

pub(crate) struct CompareScheduler {
    queue: Arc<CompareQueue>,
}

struct CompareQueue {
    state: Mutex<CompareQueueState>,
    ready: Condvar,
}

#[derive(Default)]
struct CompareQueueState {
    jobs: Vec<QueuedCompareJob>,
    next_sequence: u64,
}

struct QueuedCompareJob {
    sequence: u64,
    enqueued_at: Instant,
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
    LoadStats(Task<CompareStatsRequest>),
    LoadFile(Task<CompareFileRequest>),
    LoadFileStats(Task<CompareFileStatsRequest>),
}

impl CompareScheduler {
    pub(crate) fn new(services: AppServices, event_sender: RuntimeEventSender) -> Self {
        let queue = Arc::new(CompareQueue {
            state: Mutex::new(CompareQueueState::default()),
            ready: Condvar::new(),
        });
        let worker_count = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .min(MAX_COMPARE_WORKERS)
            .max(1);
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let services = services.clone();
            let event_sender = event_sender.clone();
            thread::spawn(move || compare_worker_loop(queue, services, event_sender));
        }
        Self { queue }
    }

    pub(crate) fn dispatch_load_stats(&self, task: Task<CompareStatsRequest>) {
        self.enqueue(CompareJob::LoadStats(task));
    }

    pub(crate) fn dispatch_load_file(&self, task: Task<CompareFileRequest>) {
        self.enqueue(CompareJob::LoadFile(task));
    }

    pub(crate) fn dispatch_load_file_stats(&self, task: Task<CompareFileStatsRequest>) {
        self.enqueue(CompareJob::LoadFileStats(task));
    }

    fn enqueue(&self, job: CompareJob) {
        let key = job.key();
        let priority = job.priority();
        let mut state = self.queue.state.lock().expect("compare queue poisoned");
        let dropped = state.jobs.iter().filter(|job| job.key == key).count();
        state.jobs.retain(|job| job.key != key);
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        let depth_after = state.jobs.len() + 1;
        tracing::debug!(
            target: "diffy::perf",
            ?priority,
            ?key,
            dropped,
            depth_after,
            "compare scheduler enqueue"
        );
        state.jobs.push(QueuedCompareJob {
            sequence,
            enqueued_at: Instant::now(),
            key,
            priority,
            job,
        });
        drop(state);
        self.queue.ready.notify_one();
    }
}

impl CompareJob {
    fn priority(&self) -> CompareWorkPriority {
        match self {
            CompareJob::LoadStats(task) => task.request.priority,
            CompareJob::LoadFile(task) => task.request.priority,
            CompareJob::LoadFileStats(task) => task.request.priority,
        }
    }

    fn key(&self) -> CompareJobKey {
        match self {
            CompareJob::LoadStats(task) => CompareJobKey::TotalStats {
                generation: task.generation,
            },
            CompareJob::LoadFile(task) => CompareJobKey::File {
                generation: task.generation,
                index: task.request.index,
                path: task.request.path.clone(),
            },
            CompareJob::LoadFileStats(task) => CompareJobKey::FileStats {
                generation: task.generation,
                priority: task.request.priority,
            },
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            CompareJob::LoadStats(_) => "total_stats",
            CompareJob::LoadFile(_) => "file_diff",
            CompareJob::LoadFileStats(_) => "file_stats",
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
                if let Some(index) = next_job_index(&state.jobs) {
                    break state.jobs.swap_remove(index);
                }
                state = queue.ready.wait(state).expect("compare queue poisoned");
            }
        };
        run_job(job, &services, &event_sender);
    }
}

fn next_job_index(jobs: &[QueuedCompareJob]) -> Option<usize> {
    jobs.iter()
        .enumerate()
        .max_by_key(|(_, job)| {
            (
                priority_score(job.priority),
                std::cmp::Reverse(job.sequence),
            )
        })
        .map(|(index, _)| index)
}

fn priority_score(priority: CompareWorkPriority) -> u8 {
    match priority {
        CompareWorkPriority::InteractiveSelectedFile => 60,
        CompareWorkPriority::VisibleSidebarStats => 50,
        CompareWorkPriority::VisibleViewportDiff => 40,
        CompareWorkPriority::Overscan => 30,
        CompareWorkPriority::TotalStats => 20,
        CompareWorkPriority::Warmup => 10,
    }
}

fn run_job(job: QueuedCompareJob, services: &AppServices, event_sender: &RuntimeEventSender) {
    let queue_wait_ms = job.enqueued_at.elapsed().as_millis();
    let kind = job.job.kind();
    let priority = job.priority;
    let _span = PerfSpan::new(
        "scheduler.run_job",
        format!("kind={kind} priority={priority:?} queue_wait_ms={queue_wait_ms}"),
    );
    match job.job {
        CompareJob::LoadStats(task) => run_load_stats(task, services, event_sender),
        CompareJob::LoadFile(task) => run_load_file(task, services, event_sender),
        CompareJob::LoadFileStats(task) => run_load_file_stats(task, services, event_sender),
    }
}

fn run_load_stats(
    task: Task<CompareStatsRequest>,
    services: &AppServices,
    event_sender: &RuntimeEventSender,
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
    event_sender.send(event);
}

fn run_load_file(
    task: Task<CompareFileRequest>,
    services: &AppServices,
    event_sender: &RuntimeEventSender,
) {
    let generation = task.generation;
    let request = task.request;
    let path = request.path.clone();
    let event = match services.load_compare_file(generation, request) {
        Ok(payload) => CompareEvent::CompareFileFinished(payload),
        Err(error) => CompareEvent::CompareFileFailed {
            generation,
            path,
            message: error.to_string(),
        },
    };
    event_sender.send(event);
}

fn run_load_file_stats(
    task: Task<CompareFileStatsRequest>,
    services: &AppServices,
    event_sender: &RuntimeEventSender,
) {
    let generation = task.generation;
    let request = task.request;
    if request.files.len() <= FILE_STATS_STREAM_CHUNK_SIZE {
        let event = match services.load_compare_file_stats(generation, request) {
            Ok(payload) => CompareEvent::CompareFileStatsReady(payload),
            Err(error) => CompareEvent::CompareFileStatsFailed {
                generation,
                message: error.to_string(),
            },
        };
        event_sender.send(event);
        return;
    }

    let repo_path = request.repo_path;
    let priority = request.priority;
    let requested_at_ms = request.requested_at_ms;
    thread::scope(|scope| {
        for files in request.files.chunks(FILE_STATS_STREAM_CHUNK_SIZE) {
            let services = services.clone();
            let event_sender = event_sender.clone();
            let repo_path = repo_path.clone();
            let files = files.to_vec();
            scope.spawn(move || {
                let request = CompareFileStatsRequest {
                    repo_path,
                    files,
                    priority,
                    requested_at_ms,
                };
                let event = match services.load_compare_file_stats(generation, request) {
                    Ok(mut payload) => {
                        payload.request_complete = false;
                        CompareEvent::CompareFileStatsReady(payload)
                    }
                    Err(error) => CompareEvent::CompareFileStatsFailed {
                        generation,
                        message: error.to_string(),
                    },
                };
                event_sender.send(event);
            });
        }
    });
    event_sender.send(CompareEvent::CompareFileStatsReady(CompareFileStatsReady {
        generation,
        stats: Vec::new(),
        request_complete: true,
        requested_at_ms,
    }));
}
