//! Highlighting on a background thread.

use std::collections::HashMap;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use crate::store::{Outcome, Part, Slice};
use crate::{GrammarStore, HighlightSpan, LanguageId};

/// Which queued request the worker takes next: every [`Priority::Visible`]
/// one before any [`Priority::Background`] one, each in the order it was
/// first requested.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// On screen now.
    #[default]
    Visible,
    /// Not shown yet, such as a file further down a diff.
    Background,
}

/// What to highlight for a slot.
#[derive(Debug, Clone)]
pub struct HighlightRequest {
    pub language: LanguageId,
    pub source: Arc<str>,
    /// Byte ranges of `source` to highlight, each as a document of its own,
    /// or `None` for the whole source as one document. See
    /// [`HighlightRequest::fragments`].
    pub fragments: Option<Arc<[Range<u32>]>>,
    pub priority: Priority,
    /// Where the reader is in a long source, read while it is highlighted
    /// window by window; see [`HighlightRequest::focus`].
    pub focus: Option<HighlightFocus>,
}

/// A byte offset of a source that a long highlight colors first, shared
/// with the worker so it can move while the highlight runs: a scroll
/// position, typically. Unset means none.
#[derive(Debug, Clone, Default)]
pub struct HighlightFocus(Arc<AtomicU64>);

impl HighlightFocus {
    pub fn new() -> Self {
        let focus = Self::default();
        focus.set(None);
        focus
    }

    pub fn set(&self, byte: Option<usize>) {
        let value = byte.map_or(u64::MAX, |b| b as u64);
        self.0.store(value, Ordering::Relaxed);
    }

    pub fn get(&self) -> Option<usize> {
        match self.0.load(Ordering::Relaxed) {
            u64::MAX => None,
            byte => Some(byte as usize),
        }
    }
}

impl HighlightRequest {
    /// The whole of `source`, visible.
    pub fn new(language: LanguageId, source: Arc<str>) -> Self {
        Self {
            language,
            source,
            fragments: None,
            priority: Priority::Visible,
            focus: None,
        }
    }

    /// Highlights only `ranges` (sorted and disjoint, on character
    /// boundaries), each parsed on its own so no lexical state carries
    /// from one to the next. For text made of excerpts, such as the hunks
    /// of a patch: a comment opened in one hunk must not color the next,
    /// whose surroundings are unknown. Result spans stay in `source`'s
    /// coordinates and cover only the ranges.
    pub fn fragments(mut self, ranges: impl Into<Arc<[Range<u32>]>>) -> Self {
        self.fragments = Some(ranges.into());
        self
    }

    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Asks for a source longer than a couple of mebibytes to be
    /// highlighted window by window, each window arriving as a result of
    /// its own (see [`Highlighted::part`]); the lines around `focus` are
    /// colored first, wherever it is when the worker gets to them. Without
    /// a focus a request always gets one whole result.
    pub fn focus(mut self, focus: HighlightFocus) -> Self {
        self.focus = Some(focus);
        self
    }
}

/// One handle's request for `slot`.
#[derive(Clone)]
struct Job {
    client: u64,
    slot: u64,
    generation: u64,
    request: HighlightRequest,
    /// Where a streamed highlight that yielded goes on from (0 to start),
    /// and the inexact window it colored last.
    resume: (usize, Option<Range<usize>>),
}

/// Where one handle's results go.
struct Reply {
    done: Sender<Highlighted>,
    wake: Wake,
}

/// Called after each result a handle receives.
type Wake = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;

enum Message {
    /// A handle's results go to this reply from now on.
    Register(u64, Reply),
    Job(Job),
    Prioritize {
        client: u64,
        slot: u64,
        priority: Priority,
    },
    /// A pending grammar resolved: rerun the jobs waiting for one.
    Retry,
    /// A handle was dropped: drop its jobs.
    Forget(u64),
    Stop,
}

/// A finished highlight of the source requested for `slot` at
/// `generation`.
///
/// A request can get several results while grammars arrive: each has the
/// same generation and a higher `revision`, and the last has `pending`
/// unset. Keep a result when its `(generation, revision)` is newer than the
/// one held for the slot, and drop it otherwise.
#[derive(Debug, Clone)]
pub struct Highlighted {
    pub slot: u64,
    pub generation: u64,
    /// Counts the results for this slot and generation, from 0.
    pub revision: u32,
    pub source: Arc<str>,
    pub spans: Vec<HighlightSpan>,
    /// Grammars in `unresolved` are still arriving, so another result for
    /// this slot and generation follows once one of them resolves.
    pub pending: bool,
    /// Languages whose grammars are still arriving. When the requested
    /// language is one, `spans` is empty; otherwise these are languages
    /// embedded in the source, whose regions keep the host's colors until
    /// they arrive.
    pub unresolved: Vec<LanguageId>,
    /// `None` for a whole highlight. A long source comes in parts instead,
    /// each with a higher revision, whose spans cover only its range.
    pub part: Option<HighlightPart>,
}

/// What part of a long source a [`Highlighted`] colors.
///
/// Exact parts run from the start of the source and follow one another
/// without gaps; together they color it as a whole-file highlight would,
/// and the source is done when one ends at its end. An exact part starting
/// at 0 begins a new pass (a grammar arrived), whose parts replace the old
/// ones. An inexact part is the reader's focus colored on its own ahead of
/// the exact pass: hold it only where no exact part has arrived.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HighlightPart {
    pub range: Range<u32>,
    pub exact: bool,
}

/// The worker thread is gone (it could not be spawned), so no more results
/// will arrive. Requests sent since are lost; make a new worker and send
/// them again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerGone;

/// Highlights a request, stopping early once the callback returns true. A
/// streamed highlight hands its windows to the last argument and returns
/// an outcome marked `streamed`.
type Highlight = dyn Fn(&HighlightRequest, &Slice, &(dyn Fn() -> bool + Sync), &mut dyn FnMut(Part)) -> Outcome
    + Send
    + Sync;
type HighlightFn = Arc<Highlight>;

/// A handle's slot.
type Key = (u64, u64);

/// The newest generation requested per slot, written by
/// [`HighlightWorker::request`] before the job is sent, so a highlight in
/// progress can see that it was superseded without draining the channel.
/// A slot's entry goes once its newest generation is done.
type Latest = Arc<Mutex<HashMap<Key, u64>>>;

/// The thread every handle of one worker shares; it stops when the last
/// handle goes.
struct Thread {
    jobs: Sender<Message>,
    handle: Option<JoinHandle<()>>,
    /// Set on drop so the thread stops at the next checkpoint instead of
    /// finishing its batch while the dropping thread waits.
    cancel: Arc<AtomicBool>,
    latest: Latest,
    next_client: AtomicU64,
}

impl Drop for Thread {
    fn drop(&mut self) {
        // The flag stops a highlight in progress; `Stop` ends the thread's
        // loop (the store's subscription keeps a sender, so the channel does
        // not close). The wait is at most one checkpoint.
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.jobs.send(Message::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Highlights requests on one background thread with a [`GrammarStore`]'s
/// grammars. A slot is one code block; when several requests for a slot
/// are queued, only the newest generation is highlighted, so a block that
/// streams faster than it highlights does not build a backlog. Results
/// arrive in [`HighlightWorker::try_recv`]; callers still compare
/// generations, since a result can land after a newer request was sent.
///
/// [`HighlightWorker::share`] makes another handle on the same thread with
/// slots and results of its own, so several views highlight on one thread
/// without seeing each other's results. Queued requests run by
/// [`Priority`], then in the order they were first requested.
///
/// A request whose grammars are still downloading gets the best result
/// available at once (plain when its own grammar is missing, host colors
/// when only embedded ones are) marked [`Highlighted::pending`], then a
/// result with a higher revision each time an arriving grammar changes it,
/// unless a newer request for the slot superseded it.
///
/// A highlight in progress stops at its next checkpoint (tree-sitter polls
/// one every hundred or so parse or query steps) once a newer request for
/// its slot arrives, and yields no result.
///
/// A highlight that panics yields a result with no spans (the block stays
/// plain) and the thread keeps serving requests.
pub struct HighlightWorker {
    thread: Arc<Thread>,
    client: u64,
    done: Receiver<Highlighted>,
    wake: Wake,
}

impl HighlightWorker {
    pub fn new(store: GrammarStore) -> Self {
        let highlighter = store.clone();
        let worker = Self::with_highlighter(Arc::new(move |request, slice, cancelled, emit| {
            let HighlightRequest {
                language,
                source,
                fragments,
                focus,
                ..
            } = request;
            match (fragments, focus) {
                (Some(ranges), _) => {
                    highlighter.highlight_fragments_until(language, source, ranges, cancelled)
                }
                // Only a request with a focus expects parts.
                (None, Some(focus)) => {
                    let focus = || focus.get();
                    highlighter.highlight_streamed(language, source, slice, cancelled, &focus, emit)
                }
                (None, None) => highlighter.highlight_until(language, source, cancelled),
            }
        }));
        let jobs = worker.thread.jobs.clone();
        store.subscribe(move || jobs.send(Message::Retry).is_ok());
        worker
    }

    fn with_highlighter(highlight: HighlightFn) -> Self {
        let (jobs, job_rx) = channel::<Message>();
        let cancel = Arc::new(AtomicBool::new(false));
        let latest = Latest::default();
        let (stop, newest) = (cancel.clone(), latest.clone());
        let handle = std::thread::Builder::new()
            .name("diffy-syntax".to_owned())
            .spawn(move || run(&job_rx, &stop, &newest, &*highlight))
            .ok();
        Self::client_of(Arc::new(Thread {
            jobs,
            handle,
            cancel,
            latest,
            next_client: AtomicU64::new(0),
        }))
    }

    /// A new handle on `thread`. When the thread is gone the registration
    /// is dropped with its sender, so the handle reports [`WorkerGone`].
    fn client_of(thread: Arc<Thread>) -> Self {
        let client = thread.next_client.fetch_add(1, Ordering::Relaxed);
        let (done_tx, done) = channel();
        let wake = Wake::default();
        let reply = Reply {
            done: done_tx,
            wake: wake.clone(),
        };
        let _ = thread.jobs.send(Message::Register(client, reply));
        Self {
            thread,
            client,
            done,
            wake,
        }
    }

    /// Another handle on this worker's thread, with its own slots (slot 1
    /// of one handle is not slot 1 of another) and its own results. The
    /// thread stops when its last handle is dropped.
    pub fn share(&self) -> Self {
        Self::client_of(self.thread.clone())
    }

    /// Calls `wake` on the worker thread after each result this handle
    /// receives, so an app can wake its event loop to take it instead of
    /// polling.
    pub fn set_wake(&self, wake: impl Fn() + Send + Sync + 'static) {
        *lock(&self.wake) = Some(Arc::new(wake));
    }

    /// A worker whose thread is already gone, as when spawning fails. For
    /// testing callers' recovery.
    #[doc(hidden)]
    pub fn gone() -> Self {
        let (jobs, _) = channel();
        Self::client_of(Arc::new(Thread {
            jobs,
            handle: None,
            cancel: Arc::new(AtomicBool::new(true)),
            latest: Latest::default(),
            next_client: AtomicU64::new(0),
        }))
    }

    pub fn request(&self, slot: u64, generation: u64, language: LanguageId, source: Arc<str>) {
        self.request_with(slot, generation, HighlightRequest::new(language, source));
    }

    pub fn request_with(&self, slot: u64, generation: u64, request: HighlightRequest) {
        let mut latest = lock(&self.thread.latest);
        let newest = latest.entry((self.client, slot)).or_insert(generation);
        *newest = (*newest).max(generation);
        drop(latest);
        let _ = self.thread.jobs.send(Message::Job(Job {
            client: self.client,
            slot,
            generation,
            request,
            resume: (0, None),
        }));
    }

    /// Moves `slot`'s queued request, if it has not started, to
    /// `priority`.
    pub fn prioritize(&self, slot: u64, priority: Priority) {
        let _ = self.thread.jobs.send(Message::Prioritize {
            client: self.client,
            slot,
            priority,
        });
    }

    /// A finished highlight, without blocking: `Ok(None)` when none is ready
    /// yet.
    pub fn try_recv(&self) -> Result<Option<Highlighted>, WorkerGone> {
        match self.done.try_recv() {
            Ok(result) => Ok(Some(result)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(WorkerGone),
        }
    }

    /// The next finished highlight, blocking until one arrives.
    pub fn recv(&self) -> Result<Highlighted, WorkerGone> {
        self.done.recv().map_err(|_| WorkerGone)
    }
}

impl Default for HighlightWorker {
    /// A worker without grammars: every result is plain.
    fn default() -> Self {
        Self::new(GrammarStore::none())
    }
}

impl Drop for HighlightWorker {
    fn drop(&mut self) {
        // The thread itself stops when the last handle drops `Thread`.
        let _ = self.thread.jobs.send(Message::Forget(self.client));
    }
}

/// A job still waiting for grammars, and what it last published.
struct Parked {
    job: Job,
    revision: u32,
    spans: Vec<HighlightSpan>,
    unresolved: Vec<LanguageId>,
}

/// A job waiting to run, with what it last published when it is a rerun
/// for an arriving grammar.
struct Queued {
    job: Job,
    previous: Option<Parked>,
}

fn run(
    messages: &Receiver<Message>,
    cancel: &AtomicBool,
    latest: &Mutex<HashMap<Key, u64>>,
    highlight: &Highlight,
) {
    let mut replies: HashMap<u64, Reply> = HashMap::new();
    // Jobs whose grammars were pending, newest per slot, rerun on `Retry`.
    let mut parked: HashMap<Key, Parked> = HashMap::new();
    // The newest job per slot, and the slots in the order first queued.
    let mut queued: HashMap<Key, Queued> = HashMap::new();
    let mut order: Vec<Key> = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        // Wait while idle; otherwise take whatever arrived since the last
        // job without waiting, so a newer request or priority counts before
        // the next job is picked.
        let first = if queued.is_empty() {
            match messages.recv() {
                Ok(message) => Some(message),
                Err(_) => return,
            }
        } else {
            None
        };
        let mut retry = false;
        for message in first.into_iter().chain(messages.try_iter()) {
            match message {
                Message::Register(client, reply) => {
                    replies.insert(client, reply);
                }
                Message::Job(job) => {
                    let key = (job.client, job.slot);
                    parked.remove(&key);
                    match queued.get(&key) {
                        Some(held) if held.job.generation > job.generation => {}
                        Some(_) => {
                            queued.insert(
                                key,
                                Queued {
                                    job,
                                    previous: None,
                                },
                            );
                        }
                        None => {
                            order.push(key);
                            queued.insert(
                                key,
                                Queued {
                                    job,
                                    previous: None,
                                },
                            );
                        }
                    }
                }
                Message::Prioritize {
                    client,
                    slot,
                    priority,
                } => {
                    let key = (client, slot);
                    if let Some(held) = queued.get_mut(&key) {
                        held.job.request.priority = priority;
                    }
                    if let Some(waiting) = parked.get_mut(&key) {
                        waiting.job.request.priority = priority;
                    }
                }
                Message::Retry => retry = true,
                Message::Forget(client) => {
                    replies.remove(&client);
                    parked.retain(|key, _| key.0 != client);
                    queued.retain(|key, _| key.0 != client);
                    order.retain(|key| key.0 != client);
                    lock(latest).retain(|key, _| key.0 != client);
                }
                Message::Stop => return,
            }
        }
        // Slots rerun for a retry carry what each last published: a rerun
        // that changes nothing is parked again without another result.
        if retry {
            for (key, waiting) in parked.drain() {
                order.push(key);
                queued.insert(
                    key,
                    Queued {
                        job: waiting.job.clone(),
                        previous: Some(waiting),
                    },
                );
            }
        }
        let next = order
            .iter()
            .enumerate()
            .min_by_key(|&(at, key)| (queued.get(key).map(|q| q.job.request.priority), at))
            .map(|(at, _)| at);
        let Some(key) = next.map(|at| order.remove(at)) else {
            continue;
        };
        let Some(Queued { job, previous }) = queued.remove(&key) else {
            continue;
        };
        let superseded = || {
            cancel.load(Ordering::Relaxed)
                || lock(latest)
                    .get(&key)
                    .is_some_and(|&newest| newest > job.generation)
        };
        // Parts of a streamed highlight go out as they are done, each with
        // the next revision.
        let mut next_revision = previous.as_ref().map_or(0, |p| p.revision + 1);
        let mut emit = |part: Part| {
            if superseded() {
                return;
            }
            let Some(reply) = replies.get(&job.client) else {
                return;
            };
            let result = Highlighted {
                slot: job.slot,
                generation: job.generation,
                revision: next_revision,
                source: job.request.source.clone(),
                spans: part.spans,
                pending: false,
                unresolved: part.unresolved,
                part: Some(HighlightPart {
                    range: part.range,
                    exact: part.exact,
                }),
            };
            next_revision += 1;
            if reply.done.send(result).is_ok()
                && let Some(wake) = lock(&reply.wake).clone()
            {
                wake();
            }
        };
        // A long source takes turns with the jobs waiting behind it: one
        // window when it starts (so every visible side gets colors early),
        // then four at a time.
        let others = !queued.is_empty();
        let turn = if job.resume.0 == 0 { 1 } else { 4 };
        let yield_after = move |handed: usize| others && handed >= turn;
        let slice = Slice {
            from: job.resume.0,
            inexact: job.resume.1.clone(),
            yield_after: &yield_after,
        };
        // A grammar bug must not take the thread down: every later block
        // would silently stay plain.
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            highlight(&job.request, &slice, &superseded, &mut emit)
        }))
        .unwrap_or_default();
        if superseded() {
            // The newer job is queued and replaces this one, parked state
            // included.
            continue;
        }
        {
            let mut latest = lock(latest);
            if latest
                .get(&key)
                .is_some_and(|&newest| newest <= job.generation)
            {
                latest.remove(&key);
            }
        }
        if outcome.streamed {
            // Languages still arriving over every turn of this pass.
            let mut unresolved = previous.map_or_else(Vec::new, |p| p.unresolved);
            for language in outcome.unresolved {
                if !unresolved.contains(&language) {
                    unresolved.push(language);
                }
            }
            let carried = Parked {
                job: Job {
                    resume: (0, None),
                    ..job.clone()
                },
                revision: next_revision.saturating_sub(1),
                spans: Vec::new(),
                unresolved,
            };
            if let Some(resume) = outcome.resume {
                // Its turn is over: back of the line, carrying what it
                // published.
                order.push(key);
                queued.insert(
                    key,
                    Queued {
                        job: Job { resume, ..job },
                        previous: Some(carried),
                    },
                );
            } else if !carried.unresolved.is_empty() {
                // Its parts are out; a pass with grammars still arriving
                // runs again when one does.
                parked.insert(key, carried);
            }
            continue;
        }
        let mut previous = previous;
        if let Some(unchanged) = previous.take_if(|previous| {
            previous.spans == outcome.spans && previous.unresolved == outcome.unresolved
        }) {
            // Still waiting on the same grammars.
            parked.insert(key, unchanged);
            continue;
        }
        let revision = previous.map_or(0, |previous| previous.revision + 1);
        let pending = outcome.pending();
        if pending {
            parked.insert(
                key,
                Parked {
                    job: job.clone(),
                    revision,
                    spans: outcome.spans.clone(),
                    unresolved: outcome.unresolved.clone(),
                },
            );
        }
        let result = Highlighted {
            slot: job.slot,
            generation: job.generation,
            revision,
            source: job.request.source,
            spans: outcome.spans,
            pending,
            unresolved: outcome.unresolved,
            part: None,
        };
        let Some(reply) = replies.get(&job.client) else {
            continue;
        };
        if reply.done.send(result).is_err() {
            // Its handle is gone; `Forget` follows.
            continue;
        }
        let wake = lock(&reply.wake).clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // The map stays valid if a panic interrupted an update.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panics_on_boom(
        request: &HighlightRequest,
        _: &Slice,
        _: &(dyn Fn() -> bool + Sync),
        _: &mut dyn FnMut(Part),
    ) -> Outcome {
        let source = &*request.source;
        assert_ne!(source, "boom", "highlighter bug");
        Outcome {
            spans: vec![HighlightSpan {
                offset: 0,
                length: source.len() as u32,
                kind: crate::HighlightKind::Keyword,
            }],
            ..Outcome::default()
        }
    }

    fn rust() -> LanguageId {
        LanguageId::from_fence("rust").unwrap()
    }

    // Catches streamed parts going out with one revision (so a bridge keeps
    // only the first) or a whole result following them that would replace
    // them: each part arrives with the next revision and its range, and the
    // slot's next result is the next request's.
    #[test]
    fn streamed_parts_arrive_with_rising_revisions_and_no_whole_result() {
        let worker =
            HighlightWorker::with_highlighter(Arc::new(|request, slice, cancelled, emit| {
                if &*request.source != "long" {
                    return panics_on_boom(request, slice, cancelled, emit);
                }
                for (range, exact) in [(3..4, false), (0..2, true), (2..4, true)] {
                    emit(Part {
                        range,
                        exact,
                        spans: Vec::new(),
                        unresolved: Vec::new(),
                    });
                }
                Outcome {
                    streamed: true,
                    ..Outcome::default()
                }
            }));
        worker.request(1, 1, rust(), Arc::from("long"));
        worker.request(2, 1, rust(), Arc::from("next"));
        let seen: Vec<_> = (0..4)
            .map(|_| {
                let r = worker.recv().unwrap();
                (r.slot, r.revision, r.part.map(|p| (p.range, p.exact)))
            })
            .collect();

        assert_eq!(
            seen,
            [
                (1, 0, Some((3..4, false))),
                (1, 1, Some((0..2, true))),
                (1, 2, Some((2..4, true))),
                (2, 0, None),
            ]
        );
    }

    // Catches one long source holding the thread until it is done, so the
    // other side of a diff stays plain for its whole pass: two long sources
    // queued together take turns, one window each first, then several.
    #[test]
    fn long_sources_take_turns() {
        let (started_tx, started) = channel();
        let (resume_tx, resume) = channel::<()>();
        let resume = Mutex::new(resume);
        let worker = HighlightWorker::with_highlighter(Arc::new(
            move |request: &HighlightRequest, slice: &Slice, _: &_, emit: &mut dyn FnMut(Part)| {
                if &*request.source == "hold" {
                    started_tx.send(()).unwrap();
                    resume.lock().unwrap().recv().unwrap();
                    return Outcome::default();
                }
                let mut handed = 0;
                for k in slice.from as u32..3 {
                    emit(Part {
                        range: k..k + 1,
                        exact: true,
                        spans: Vec::new(),
                        unresolved: Vec::new(),
                    });
                    handed += 1;
                    if k + 1 < 3 && (slice.yield_after)(handed) {
                        return Outcome {
                            streamed: true,
                            resume: Some((k as usize + 1, None)),
                            ..Outcome::default()
                        };
                    }
                }
                Outcome {
                    streamed: true,
                    ..Outcome::default()
                }
            },
        ));
        worker.request(9, 1, rust(), Arc::from("hold"));
        started.recv().unwrap();
        worker.request(1, 1, rust(), Arc::from("a"));
        worker.request(2, 1, rust(), Arc::from("b"));
        resume_tx.send(()).unwrap();
        let seen: Vec<_> = (0..7)
            .map(|_| {
                let r = worker.recv().unwrap();
                (r.slot, r.part.map(|p| p.range.start))
            })
            .collect();

        assert_eq!(
            seen,
            [
                (9, None),
                (1, Some(0)),
                (2, Some(0)),
                (1, Some(1)),
                (1, Some(2)),
                (2, Some(1)),
                (2, Some(2)),
            ]
        );
    }

    // Catches a panicking highlight killing the thread, after which every
    // block would stay plain.
    #[test]
    fn worker_survives_a_panicking_highlight() {
        let worker = HighlightWorker::with_highlighter(Arc::new(panics_on_boom));
        worker.request(1, 1, rust(), Arc::from("boom"));
        let failed = worker.recv().map(|r| (r.slot, r.spans.len()));
        worker.request(2, 2, rust(), Arc::from("fn"));
        let next = worker.recv().map(|r| (r.slot, r.spans.len()));

        assert_eq!((failed, next), (Ok((1, 0)), Ok((2, 1))));
    }

    // Catches a superseded highlight running to the end and publishing:
    // once a newer generation of its slot is requested, the highlight in
    // progress sees it at its next checkpoint and sends no result.
    #[test]
    fn superseded_highlight_stops_at_a_checkpoint_without_a_result() {
        let (started_tx, started) = channel();
        let (resume_tx, resume) = channel::<()>();
        let (seen_tx, seen) = channel();
        let resume = Mutex::new(resume);
        let worker =
            HighlightWorker::with_highlighter(Arc::new(move |request, slice, cancelled, emit| {
                if &*request.source == "first" {
                    started_tx.send(()).unwrap();
                    resume.lock().unwrap().recv().unwrap();
                    // The checkpoint a real highlight reaches while parsing.
                    seen_tx.send(cancelled()).unwrap();
                }
                panics_on_boom(request, slice, cancelled, emit)
            }));
        worker.request(1, 1, rust(), Arc::from("first"));
        started.recv().unwrap();
        worker.request(1, 2, rust(), Arc::from("second"));
        resume_tx.send(()).unwrap();
        let first_result = worker.recv().map(|r| (r.generation, r.source));

        assert_eq!(
            (seen.recv(), first_result),
            (Ok(true), Ok((2, Arc::from("second"))))
        );
    }

    /// A worker whose highlight of `"hold"` waits for the returned sender,
    /// after signalling the first receiver that it started.
    fn held_worker() -> (HighlightWorker, Receiver<()>, Sender<()>) {
        let (started_tx, started) = channel();
        let (resume_tx, resume) = channel::<()>();
        let resume = Mutex::new(resume);
        let worker =
            HighlightWorker::with_highlighter(Arc::new(move |request, slice, cancelled, emit| {
                if &*request.source == "hold" {
                    started_tx.send(()).unwrap();
                    resume.lock().unwrap().recv().unwrap();
                }
                panics_on_boom(request, slice, cancelled, emit)
            }));
        (worker, started, resume_tx)
    }

    // Catches views sharing a worker seeing each other's results or
    // superseding each other's requests: the same slot on two handles is
    // two slots, and each handle receives only its own result.
    #[test]
    fn shared_handles_keep_their_own_slots_and_results() {
        let (first, started, resume) = held_worker();
        let second = first.share();
        first.request(9, 1, rust(), Arc::from("hold"));
        started.recv().unwrap();
        first.request(1, 1, rust(), Arc::from("one"));
        second.request(1, 1, rust(), Arc::from("two"));
        resume.send(()).unwrap();
        let mine: Vec<Arc<str>> = (0..2).map(|_| first.recv().unwrap().source).collect();
        let theirs = second.recv().unwrap().source;

        assert_eq!(
            (mine, theirs),
            (vec![Arc::from("hold"), Arc::from("one")], Arc::from("two"))
        );
    }

    // Catches queued requests running in arrival order regardless of
    // priority, or a reprioritized request keeping its old place: a file
    // scrolled into view is highlighted before files queued ahead of it.
    #[test]
    fn visible_requests_run_before_background_ones() {
        let (worker, started, resume) = held_worker();
        worker.request(9, 1, rust(), Arc::from("hold"));
        started.recv().unwrap();
        let background = |source: &str| {
            HighlightRequest::new(rust(), Arc::from(source)).priority(Priority::Background)
        };
        worker.request_with(1, 1, background("offscreen"));
        worker.request_with(2, 1, background("scrolled to"));
        worker.request_with(3, 1, HighlightRequest::new(rust(), Arc::from("shown")));
        worker.prioritize(2, Priority::Visible);
        resume.send(()).unwrap();
        let order: Vec<Arc<str>> = (0..4).map(|_| worker.recv().unwrap().source).collect();

        assert_eq!(
            order,
            ["hold", "scrolled to", "shown", "offscreen"].map(Arc::from)
        );
    }

    // Catches results that wait in the channel until the app happens to
    // poll: the wake callback runs once a result can be taken.
    #[test]
    fn wake_runs_once_a_result_is_ready() {
        let worker = HighlightWorker::with_highlighter(Arc::new(panics_on_boom));
        let (woke_tx, woke) = channel();
        worker.set_wake(move || woke_tx.send(()).unwrap());
        worker.request(1, 1, rust(), Arc::from("fn"));
        woke.recv_timeout(std::time::Duration::from_secs(60))
            .unwrap();

        assert_eq!(worker.try_recv().map(|r| r.map(|r| r.slot)), Ok(Some(1)));
    }
}
