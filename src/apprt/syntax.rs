use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Instant;

use super::runtime::RuntimeEventSender;
use super::services::AppServices;
use crate::effects::{LoadFileSyntaxRequest, Task};
use crate::events::SyntaxEvent;

const MAX_SYNTAX_WORKERS: usize = 2;

pub(crate) struct SyntaxScheduler {
    queue: Arc<SyntaxQueue>,
}

struct SyntaxQueue {
    state: Mutex<SyntaxQueueState>,
    ready: Condvar,
    current_epoch: AtomicU64,
}

#[derive(Default)]
struct SyntaxQueueState {
    jobs: Vec<QueuedSyntaxJob>,
    next_sequence: u64,
}

struct QueuedSyntaxJob {
    sequence: u64,
    task: Task<LoadFileSyntaxRequest>,
}

impl SyntaxScheduler {
    pub(crate) fn new(services: AppServices, event_sender: RuntimeEventSender) -> Self {
        let queue = Arc::new(SyntaxQueue {
            state: Mutex::new(SyntaxQueueState::default()),
            ready: Condvar::new(),
            current_epoch: AtomicU64::new(0),
        });
        let worker_count = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .clamp(1, MAX_SYNTAX_WORKERS);
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let services = services.clone();
            let event_sender = event_sender.clone();
            thread::spawn(move || syntax_worker_loop(queue, services, event_sender));
        }
        Self { queue }
    }

    pub(crate) fn set_epoch(&self, epoch: u64) {
        let previous = self.queue.current_epoch.fetch_max(epoch, Ordering::AcqRel);
        if epoch < previous {
            return;
        }
        let mut state = self.queue.state.lock().expect("syntax queue poisoned");
        let current = self.queue.current_epoch.load(Ordering::Acquire);
        state
            .jobs
            .retain(|job| job.task.request.syntax_epoch >= current);
    }

    pub(crate) fn dispatch_load_file_syntax(&self, task: Task<LoadFileSyntaxRequest>) {
        let epoch = task.request.syntax_epoch;
        let previous = self.queue.current_epoch.fetch_max(epoch, Ordering::AcqRel);

        let mut state = self.queue.state.lock().expect("syntax queue poisoned");
        let current = self.queue.current_epoch.load(Ordering::Acquire);
        if epoch < previous || epoch < current {
            state
                .jobs
                .retain(|job| job.task.request.syntax_epoch >= current);
            return;
        }
        state
            .jobs
            .retain(|job| job.task.request.syntax_epoch >= current);
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        state.jobs.push(QueuedSyntaxJob { sequence, task });
        drop(state);
        self.queue.ready.notify_one();
    }
}

fn syntax_worker_loop(
    queue: Arc<SyntaxQueue>,
    services: AppServices,
    event_sender: RuntimeEventSender,
) {
    loop {
        let job = {
            let mut state = queue.state.lock().expect("syntax queue poisoned");
            loop {
                let current_epoch = queue.current_epoch.load(Ordering::Acquire);
                state
                    .jobs
                    .retain(|job| job.task.request.syntax_epoch >= current_epoch);
                if let Some(index) = next_job_index(&state.jobs) {
                    break state.jobs.swap_remove(index);
                }
                state = queue.ready.wait(state).expect("syntax queue poisoned");
            }
        };
        if job.task.request.syntax_epoch < queue.current_epoch.load(Ordering::Acquire) {
            continue;
        }
        run_load_file_syntax(job, &services, &event_sender, &queue.current_epoch);
    }
}

fn next_job_index(jobs: &[QueuedSyntaxJob]) -> Option<usize> {
    jobs.iter()
        .enumerate()
        .min_by_key(|(_, job)| job.sequence)
        .map(|(index, _)| index)
}

fn run_load_file_syntax(
    job: QueuedSyntaxJob,
    services: &AppServices,
    event_sender: &RuntimeEventSender,
    current_epoch: &AtomicU64,
) {
    let generation = job.task.generation;
    let request = job.task.request;
    let request_epoch = request.syntax_epoch;
    let is_current = || request_epoch >= current_epoch.load(Ordering::Acquire);
    if !is_current() {
        return;
    }
    let started = Instant::now();
    let tokens = services.load_file_syntax(&request, &is_current);
    if !is_current() {
        tracing::debug!(
            file_index = request.file_index,
            path = %request.path,
            request_epoch,
            current_epoch = current_epoch.load(Ordering::Acquire),
            run_ms = started.elapsed().as_millis() as u64,
            "stale syntax job dropped"
        );
        return;
    }
    event_sender.send(SyntaxEvent::FileSyntaxReady(
        crate::events::FileSyntaxReady {
            generation,
            request_id: request.request_id,
            file_index: request.file_index,
            path: request.path,
            window: request.window,
            tokens,
        },
    ));
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::core::syntax::annotator::SyntaxRowWindow;

    use super::*;

    fn scheduler_for_test() -> SyntaxScheduler {
        SyntaxScheduler {
            queue: Arc::new(SyntaxQueue {
                state: Mutex::new(SyntaxQueueState::default()),
                ready: Condvar::new(),
                current_epoch: AtomicU64::new(0),
            }),
        }
    }

    fn syntax_task(epoch: u64, request_id: u64) -> Task<LoadFileSyntaxRequest> {
        Task {
            generation: epoch,
            request: LoadFileSyntaxRequest {
                repo_path: PathBuf::from("/repo"),
                file_index: request_id as usize,
                path: format!("src/file-{request_id}.rs"),
                carbon_file: Arc::new(carbon::FileDiff {
                    old_path: Some(format!("src/file-{request_id}.rs")),
                    new_path: Some(format!("src/file-{request_id}.rs")),
                    ..carbon::FileDiff::default()
                }),
                carbon_expansion: carbon::ExpansionState::default(),
                left_ref: "old".to_owned(),
                right_ref: "new".to_owned(),
                window: SyntaxRowWindow { start: 0, end: 32 },
                request_id,
                cache_generation: epoch,
                syntax_epoch: epoch,
            },
        }
    }

    #[test]
    fn newer_syntax_epoch_drops_queued_older_jobs() {
        let scheduler = scheduler_for_test();
        scheduler.dispatch_load_file_syntax(syntax_task(1, 1));
        scheduler.dispatch_load_file_syntax(syntax_task(1, 2));

        scheduler.dispatch_load_file_syntax(syntax_task(2, 3));

        let state = scheduler.queue.state.lock().expect("syntax queue poisoned");
        assert_eq!(scheduler.queue.current_epoch.load(Ordering::Acquire), 2);
        assert_eq!(state.jobs.len(), 1);
        assert_eq!(state.jobs[0].task.request.syntax_epoch, 2);
    }

    #[test]
    fn explicit_syntax_epoch_cancel_drops_queued_older_jobs() {
        let scheduler = scheduler_for_test();
        scheduler.dispatch_load_file_syntax(syntax_task(1, 1));
        scheduler.dispatch_load_file_syntax(syntax_task(1, 2));

        scheduler.set_epoch(2);
        scheduler.dispatch_load_file_syntax(syntax_task(1, 3));
        scheduler.dispatch_load_file_syntax(syntax_task(2, 4));

        let state = scheduler.queue.state.lock().expect("syntax queue poisoned");
        assert_eq!(scheduler.queue.current_epoch.load(Ordering::Acquire), 2);
        assert_eq!(state.jobs.len(), 1);
        assert_eq!(state.jobs[0].task.request.request_id, 4);
    }
}
