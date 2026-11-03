//! Native half of `@diffy/node`; `index.js` is the API.
//!
//! Input is copied on the JS thread; work runs on threads of its own, not
//! libuv's pool (shared with fs, DNS and zlib), and settles through a
//! thread-safe function. `diff` waits in [`queue`]; `expand` starts at once.
//! Results cross as one JSON string.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use diffy_compare::wire::{self, WireOptions};
use diffy_compare::{FileStatus, GrammarStore, Options};
use diffy_syntax::StoreConfig;
use napi::bindgen_prelude::{Either, Object, Uint8Array};
use napi::{Env, Error, Result, Status};
use napi_derive::napi;

mod queue;

#[napi]
pub const FORMAT_VERSION: u32 = wire::FORMAT_VERSION;

/// Bundled grammars, by canonical name.
#[napi]
pub fn languages() -> Vec<String> {
    diffy_syntax::bundled::languages()
        .map(str::to_owned)
        .collect()
}

/// Built lazily off the JS thread; bundled only, so no network or file system.
fn grammars() -> &'static GrammarStore {
    static STORE: OnceLock<GrammarStore> = OnceLock::new();
    STORE.get_or_init(|| GrammarStore::new(StoreConfig::new().bundled()))
}

#[napi(object, object_to_js = false)]
pub struct FileInput {
    pub status: String,
    #[napi(js_name = "old_path")]
    pub old_path: Option<String>,
    #[napi(js_name = "new_path")]
    pub new_path: Option<String>,
    pub old: Option<Either<Uint8Array, String>>,
    pub new: Option<Either<Uint8Array, String>>,
}

#[napi(object, object_to_js = false)]
pub struct DiffRequest {
    pub files: Vec<FileInput>,
    #[napi(js_name = "budget_ms")]
    pub budget_ms: Option<f64>,
    #[napi(js_name = "file_budget_ms")]
    pub file_budget_ms: Option<f64>,
    pub syntax: Option<bool>,
    #[napi(js_name = "syntax_max_bytes")]
    pub syntax_max_bytes: Option<f64>,
}

#[napi(object, object_to_js = false)]
pub struct Gap {
    #[napi(js_name = "old_start")]
    pub old_start: f64,
    #[napi(js_name = "new_start")]
    pub new_start: f64,
    pub len: f64,
}

/// Set by `index.js` from an `AbortSignal`.
#[napi]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

#[napi]
impl CancelToken {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A queued request is rejected at once and never starts; a running one
    /// stops at its next file.
    #[napi]
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
        queue::cancel(&self.flag);
    }
}

#[napi(object, object_from_js = false)]
pub struct QueueState {
    pub limit: u32,
    pub running: u32,
    pub queued: u32,
}

/// Sets the `diff` concurrency limit, if given, and reports the queue.
#[napi(js_name = "queue")]
pub fn queue_state(limit: Option<f64>) -> Result<QueueState> {
    if let Some(limit) = limit {
        if !(limit.is_finite() && limit >= 1.0 && limit.fract() == 0.0) {
            return Err(invalid(format!(
                "max_concurrent_requests: {limit} is not a positive integer"
            )));
        }
        // `as` saturates; usize::MAX means no limit anyway.
        queue::set_limit(limit as usize);
    }
    let state = queue::state();
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Ok(QueueState {
        limit: count(state.limit),
        running: count(state.running),
        queued: count(state.queued),
    })
}

/// Queued; resolves to the Document's JSON.
#[napi]
pub fn diff<'env>(
    env: &'env Env,
    request: DiffRequest,
    cancel: &CancelToken,
) -> Result<Object<'env>> {
    // The request budget runs from the call, so it counts time queued.
    let start = Instant::now();
    let deadline = request
        .budget_ms
        .map(|ms| duration("budget_ms", ms).map(|budget| start + budget))
        .transpose()?;
    let file_budget = match request.file_budget_ms {
        Some(ms) => duration("file_budget_ms", ms)?,
        None => diffy_compare::FILE_BUDGET,
    };
    let wire = wire_settings(request.syntax, request.syntax_max_bytes)?;
    let inputs = request
        .files
        .into_iter()
        .map(file_input)
        .collect::<Result<Vec<_>>>()?;
    let flag = Arc::clone(&cancel.flag);
    let (deferred, promise) = env.create_deferred()?;
    let work: queue::Work = Box::new(move || {
        let grammars = grammars();
        let mut options = Options::new(grammars);
        options.file_budget = file_budget;
        options.deadline = deadline;
        options.cancel = Some(&flag);
        // No `vet`: the binding never sees the repository to confirm renames.
        let diffs = diffy_compare::diff_files(&inputs, &options).map_err(compare_error)?;
        let document = wire::document(
            &inputs,
            &diffs,
            &WireOptions {
                deadline,
                cancel: Some(&flag),
                ..wire.options(grammars)
            },
        );
        serde_json::to_string(&document).map_err(json_error)
    });
    queue::submit(queue::Job {
        deferred,
        cancel: Arc::clone(&cancel.flag),
        work,
    });
    Ok(promise)
}

/// Context rows of `gap` as JSON. Not queued: one file, and a user is waiting.
#[napi]
pub fn expand<'env>(
    env: &'env Env,
    file: FileInput,
    gap: Gap,
    syntax: Option<bool>,
) -> Result<Object<'env>> {
    let gap = wire::Gap {
        old_start: line_number("old_start", gap.old_start)?,
        new_start: line_number("new_start", gap.new_start)?,
        len: line_number("len", gap.len)?,
    };
    let wire = wire_settings(syntax, None)?;
    let input = file_input(file)?;
    let (deferred, promise) = env.create_deferred()?;
    let work: queue::Work = Box::new(move || {
        let rows = wire::expand(&input, &gap, &wire.options(grammars()));
        serde_json::to_string(&rows).map_err(json_error)
    });
    std::thread::Builder::new()
        .name("diffy-node-expand".to_owned())
        .stack_size(queue::STACK_BYTES)
        .spawn(move || queue::settle(deferred, work))
        .map_err(|error| Error::from_reason(format!("could not start a thread: {error}")))?;
    Ok(promise)
}

/// [`WireOptions`] without the borrowed store, so it can cross threads.
#[derive(Clone, Copy)]
struct WireSettings {
    syntax: bool,
    syntax_max_bytes: usize,
}

impl WireSettings {
    fn options<'a>(&self, grammars: &'a GrammarStore) -> WireOptions<'a> {
        WireOptions {
            syntax: self.syntax.then_some(grammars),
            syntax_max_bytes: self.syntax_max_bytes,
            ..WireOptions::default()
        }
    }
}

fn wire_settings(syntax: Option<bool>, max_bytes: Option<f64>) -> Result<WireSettings> {
    let syntax_max_bytes = match max_bytes {
        Some(bytes) if bytes.is_finite() && bytes >= 0.0 && bytes.fract() == 0.0 => {
            // `as` saturates; usize::MAX means no limit anyway.
            bytes as usize
        }
        Some(bytes) => return Err(invalid(format!("syntax_max_bytes: {bytes} is not a size"))),
        None => wire::SYNTAX_MAX_BYTES,
    };
    Ok(WireSettings {
        syntax: syntax.unwrap_or(false),
        syntax_max_bytes,
    })
}

fn file_input(file: FileInput) -> Result<diffy_compare::FileInput> {
    let status = match file.status.as_str() {
        "modified" => FileStatus::Modified,
        "added" => FileStatus::Added,
        "deleted" => FileStatus::Deleted,
        "renamed" => FileStatus::Renamed,
        "copied" => FileStatus::Copied,
        other => return Err(invalid(format!("status: unknown file status {other:?}"))),
    };
    if file.old_path.is_none() && file.new_path.is_none() {
        return Err(invalid("a file needs old_path or new_path".to_owned()));
    }
    Ok(diffy_compare::FileInput {
        status,
        old_path: file.old_path,
        new_path: file.new_path,
        old: bytes(file.old),
        new: bytes(file.new),
    })
}

fn bytes(content: Option<Either<Uint8Array, String>>) -> Vec<u8> {
    match content {
        Some(Either::A(array)) => array.to_vec(),
        Some(Either::B(text)) => text.into_bytes(),
        None => Vec::new(),
    }
}

fn duration(name: &str, ms: f64) -> Result<Duration> {
    Duration::try_from_secs_f64(ms / 1000.0)
        .map_err(|_| invalid(format!("{name}: {ms} is not a duration in milliseconds")))
}

fn line_number(name: &str, value: f64) -> Result<u32> {
    if value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= f64::from(u32::MAX) {
        Ok(value as u32)
    } else {
        Err(invalid(format!("{name}: {value} is not a line number")))
    }
}

fn invalid(reason: String) -> Error {
    Error::new(Status::InvalidArg, reason)
}

/// `index.js` turns the `Cancelled` code into an `AbortError`.
fn compare_error(error: diffy_compare::Error) -> Error {
    let status = match error {
        diffy_compare::Error::Cancelled => Status::Cancelled,
        _ => Status::GenericFailure,
    };
    Error::new(status, error.to_string())
}

fn json_error(error: serde_json::Error) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}
