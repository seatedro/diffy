//! Process-wide FIFO of `diff` requests.
//!
//! At most [`limit`](Queue::limit) run at once, each on its own thread. A
//! queued request holds no thread: a finishing thread takes the next one or
//! exits. A cancelled queued request is rejected at once and never starts.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};

use napi::{Env, Error, JsDeferred, Result, Status};

/// The semantic diff may run on this thread; same stack as the app's
/// semantic workers. Reserved, not committed.
pub(crate) const STACK_BYTES: usize = 16 << 20;

/// `diffy_compare`'s per-request worker fan-out.
const WORKERS_PER_REQUEST: usize = 4;

/// A promise settled with a JSON string from any thread.
pub(crate) type Deferred = JsDeferred<String, Box<dyn FnOnce(Env) -> Result<String> + Send>>;

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

/// Fills the cores without requests time-slicing away each other's file
/// budgets.
fn default_limit() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    (cores / WORKERS_PER_REQUEST).max(1)
}

pub(crate) fn submit(job: Job) {
    let mut queue = lock();
    queue.waiting.push_back(job);
    let ready = queue.next();
    drop(queue);
    if let Some(job) = ready {
        start(job);
    }
}

/// Raising the limit starts queued jobs; lowering it lets running ones finish.
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

/// Rejects queued jobs with this flag; running ones poll it themselves.
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

/// Runs `job`, which already holds a slot, then queued jobs, on a new thread.
fn start(mut job: Job) {
    loop {
        // Sent only once the thread exists: a failed spawn drops its closure,
        // and the promise must still be settled.
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

/// A panic in `work` rejects the promise instead of unwinding into Node.
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
