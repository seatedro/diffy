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

    fn enqueue(&self, job: CompareJob) {
        let key = job.key();
        let priority = job.priority();
        let mut state = self.queue.state.lock().expect("compare queue poisoned");
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
        .max_by_key(|(_, job)| (job.priority.rank(), std::cmp::Reverse(job.sequence)))
        .map(|(index, _)| index)
}

fn run_job(job: QueuedCompareJob, services: &AppServices, event_sender: &RuntimeEventSender) {
    match job.job {
        CompareJob::Stats(task) => run_load_stats(task, services, event_sender),
        CompareJob::File(task) => run_load_file(task, services, event_sender),
        CompareJob::FileStats(task) => run_load_file_stats(task, services, event_sender),
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
    let payload = match services.load_compare_file_stats(generation, request) {
        Ok(payload) => payload,
        Err(error) => {
            event_sender.send(CompareEvent::CompareFileStatsFailed {
                generation,
                message: error.to_string(),
            });
            return;
        }
    };
    send_file_stats_payload(generation, payload, event_sender);
}

fn send_file_stats_payload(
    generation: u64,
    payload: CompareFileStatsReady,
    event_sender: &RuntimeEventSender,
) {
    if payload.stats.len() <= FILE_STATS_STREAM_CHUNK_SIZE {
        event_sender.send(CompareEvent::CompareFileStatsReady(payload));
        return;
    }

    let mut stats = payload.stats.into_iter();
    loop {
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
    use crate::effects::CompareWorkPriority;

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
