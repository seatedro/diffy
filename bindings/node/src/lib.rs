//! The native half of `@diffy/node`; `index.js` wraps it and is the API.
//!
//! Each call copies its input on the JS thread, then does the work on a
//! thread of its own, never on libuv's pool (which a server's file system,
//! DNS, and zlib calls share), and settles its promise through a
//! thread-safe function. Results cross as one JSON string of the display
//! format, which `index.js` parses.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use diffy_compare::wire::{self, WireOptions};
use diffy_compare::{FileStatus, GrammarStore, Options};
use diffy_syntax::StoreConfig;
use napi::bindgen_prelude::{Either, Object, Uint8Array};
use napi::{Env, Error, Result, Status};
use napi_derive::napi;

/// The semantic diff may run on the request thread itself; give it the
/// stack the app gives its semantic workers. Only reserved, not committed.
const STACK_BYTES: usize = 16 << 20;

#[napi]
pub const FORMAT_VERSION: u32 = wire::FORMAT_VERSION;

/// The grammars compiled into this build, by canonical name.
#[napi]
pub fn languages() -> Vec<String> {
    diffy_syntax::bundled::languages()
        .map(str::to_owned)
        .collect()
}

/// Built on first use, off the JS thread; bundled grammars only, so it
/// never touches the network or the file system.
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

/// Stops the request it is passed to; `index.js` sets it from an
/// `AbortSignal`.
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

    #[napi]
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }
}

/// Diffs `request.files` and resolves to the display format's JSON.
#[napi]
pub fn diff<'env>(
    env: &'env Env,
    request: DiffRequest,
    cancel: &CancelToken,
) -> Result<Object<'env>> {
    // The request budget runs from the call, not from when the thread starts.
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
    spawn(env, "diffy-node-diff", move || {
        let grammars = grammars();
        let mut options = Options::new(grammars);
        options.file_budget = file_budget;
        options.deadline = deadline;
        options.cancel = Some(&flag);
        // No `vet`: renames are detected but not confirmed against the rest
        // of the repository, which the binding never sees.
        let diffs = diffy_compare::diff_files(&inputs, &options).map_err(compare_error)?;
        let document = wire::document(&inputs, &diffs, &wire.options(grammars));
        serde_json::to_string(&document).map_err(json_error)
    })
}

/// The context rows of `gap` in `file`, as the display format's JSON.
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
    spawn(env, "diffy-node-expand", move || {
        let rows = wire::expand(&input, &gap, &wire.options(grammars()));
        serde_json::to_string(&rows).map_err(json_error)
    })
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
        }
    }
}

fn wire_settings(syntax: Option<bool>, max_bytes: Option<f64>) -> Result<WireSettings> {
    let syntax_max_bytes = match max_bytes {
        Some(bytes) if bytes.is_finite() && bytes >= 0.0 && bytes.fract() == 0.0 => {
            // Saturates past usize::MAX, which means "no limit" anyway.
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

/// Runs `work` on a new thread and returns a promise of its result. A
/// panic in `work` rejects the promise; it cannot unwind into Node.
fn spawn<'env, F>(env: &'env Env, name: &str, work: F) -> Result<Object<'env>>
where
    F: FnOnce() -> Result<String> + Send + 'static,
{
    let (deferred, promise) = env.create_deferred::<String, _>()?;
    std::thread::Builder::new()
        .name(name.to_owned())
        .stack_size(STACK_BYTES)
        .spawn(move || match catch_unwind(AssertUnwindSafe(work)) {
            Ok(Ok(json)) => deferred.resolve(move |_| Ok(json)),
            Ok(Err(error)) => deferred.reject(error),
            Err(panic) => deferred.reject(panic_error(panic.as_ref())),
        })
        .map_err(|error| Error::from_reason(format!("could not start a thread: {error}")))?;
    Ok(promise)
}

fn panic_error(panic: &(dyn std::any::Any + Send)) -> Error {
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic");
    Error::new(Status::GenericFailure, format!("diffy panicked: {message}"))
}
