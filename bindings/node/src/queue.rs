//! The process-wide FIFO queue of `diff` requests.
//!
//! At most [`limit`](Queue::limit) requests run at once, each on a thread
//! of its own that fans out to a few more. A queued request holds no thread
//! and blocks nothing: when a request finishes, its thread takes the next
//! queued one, or exits. Requests are cancelled out of the queue at once
//! and never start.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};

use napi::{Env, Error, JsDeferred, Result, Status};

/// The semantic diff may run on the request thread itself; give it the
/// stack the app gives its semantic workers. Only reserved, not committed.
pub(crate) const STACK_BYTES: usize = 16 << 20;

/// `diffy_compare` diffs a request of four or more files on four workers,
/// then highlights them on as many, so a request can keep this many cores
/// busy.
const WORKERS_PER_REQUEST: usize = 4;

/// A promise settled with a JSON string from any thread.
pub(crate) type Deferred = JsDeferred<String, Box<dyn FnOnce(Env) -> Result<String> + Send>>;

/// A request's work: the JSON it resolves to.
pub(crate) type Work = Box<dyn FnOnce() -> Result<String> + Send>;

pub(crate) struct Job {
    pub deferred: Deferred,
    pub cancel: Arc<AtomicBool>,
    pub work: Work,
}

struct Queue {
    /// 0 until first used.
    limit: usize,
    running: usize,
    waiting: VecDeque<Job>,
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue {
    limit: 0,
    running: 0,
    waiting: VecDeque::new(),
});

fn lock() -> MutexGuard<'static, Queue> {
    // Nothing panics while the lock is held, so the queue is consistent.
    QUEUE.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Queue {
    fn limit(&mut self) -> usize {
        if self.limit == 0 {
            self.limit = default_limit();
        }
        self.limit
    }

    /// The oldest waiting job, counted as running, if a slot is free.
    /// Cancelled jobs found on the way are rejected.
    fn next(&mut self) -> Option<Job> {
        if self.running >= self.limit() {
            return None;
        }
        while let Some(job) = self.waiting.pop_front() {
            if job.cancel.load(Ordering::Relaxed) {
                reject_cancelled(job);
                continue;
            }
            self.running += 1;
            return Some(job);
        }
        None
    }
}

/// One request per [`WORKERS_PER_REQUEST`] cores, so running requests keep
/// every core busy without time-slicing each other's file budgets away.
fn default_limit() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    (cores / WORKERS_PER_REQUEST).max(1)
}

/// Queues `job`; it starts once it is the oldest and fewer than the limit
/// are running.
pub(crate) fn submit(job: Job) {
    let mut queue = lock();
    queue.waiting.push_back(job);
    let ready = queue.next();
    drop(queue);
    if let Some(job) = ready {
        start(job);
    }
}

/// Sets the limit and starts the queued jobs it now lets run. Running jobs
/// past a lowered limit finish; no new ones start until under it.
pub(crate) fn set_limit(limit: usize) {
    let mut queue = lock();
    queue.limit = limit.max(1);
    let mut ready = Vec::new();
    while let Some(job) = queue.next() {
        ready.push(job);
    }
    drop(queue);
    for job in ready {
        start(job);
    }
}

pub(crate) struct State {
    pub limit: usize,
    pub running: usize,
    pub queued: usize,
}

pub(crate) fn state() -> State {
    let mut queue = lock();
    State {
        limit: queue.limit(),
        running: queue.running,
        queued: queue.waiting.len(),
    }
}

/// Rejects the queued jobs `cancel` stops, which then never start. Running
/// ones see the flag themselves.
pub(crate) fn cancel(cancel: &Arc<AtomicBool>) {
    let mut queue = lock();
    let waiting = std::mem::take(&mut queue.waiting);
    for job in waiting {
        if Arc::ptr_eq(&job.cancel, cancel) {
            reject_cancelled(job);
        } else {
            queue.waiting.push_back(job);
        }
    }
}

fn reject_cancelled(job: Job) {
    job.deferred
        .reject(Error::new(Status::Cancelled, "cancelled".to_owned()));
}

/// Gives up the caller's slot, or hands it the next queued job.
fn finish() -> Option<Job> {
    let mut queue = lock();
    queue.running = queue.running.saturating_sub(1);
    queue.next()
}

/// Runs `job`, which holds a slot, and then queued jobs, on a new thread.
fn start(mut job: Job) {
    loop {
        // The job is sent once the thread exists: a failed spawn drops its
        // closure, and the job's promise must still be settled.
        let error = match spawn_runner() {
            Ok(sender) => match sender.send(job) {
                Ok(()) => return,
                Err(mpsc::SendError(unsent)) => {
                    job = unsent;
                    "it exited at once".to_owned()
                }
            },
            Err(error) => error.to_string(),
        };
        job.deferred.reject(Error::from_reason(format!(
            "could not start a thread: {error}"
        )));
        match finish() {
            Some(next) => job = next,
            None => return,
        }
    }
}

fn spawn_runner() -> std::io::Result<mpsc::SyncSender<Job>> {
    let (sender, receiver) = mpsc::sync_channel::<Job>(1);
    std::thread::Builder::new()
        .name("diffy-node-diff".to_owned())
        .stack_size(STACK_BYTES)
        .spawn(move || {
            let mut next = receiver.recv().ok();
            while let Some(job) = next {
                settle(job.deferred, job.work);
                next = finish();
            }
        })?;
    Ok(sender)
}

/// Runs `work` and settles `deferred` with its result. A panic in `work`
/// rejects the promise; it cannot unwind into Node.
pub(crate) fn settle(deferred: Deferred, work: Work) {
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(Ok(json)) => deferred.resolve(Box::new(move |_| Ok(json))),
        Ok(Err(error)) => deferred.reject(error),
        Err(panic) => deferred.reject(panic_error(panic.as_ref())),
    }
}

fn panic_error(panic: &(dyn std::any::Any + Send)) -> Error {
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic");
    Error::new(Status::GenericFailure, format!("diffy panicked: {message}"))
}
