//! Running files on worker threads.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use diffy_syntax::LanguageId;

use crate::file::diff_one;
use crate::grammar::SemanticGrammar;
use crate::{Error, FileDiff, FileInput, Options, Result};

const MAX_WORKERS: usize = 4;
const MIN_FILES_FOR_PARALLEL: usize = 4;
/// Tree-sitter parsing and the semantic matcher recurse deeply on nested
/// sources, more than a default thread stack allows.
const WORKER_STACK_BYTES: usize = 16 << 20;

type Grammars<'g> = dyn Fn(&LanguageId) -> Option<crate::SemanticGrammar> + Sync + 'g;

pub(crate) fn cancelled(options: &Options<'_>) -> bool {
    options
        .cancel
        .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
}

/// Diffs `input` under the observer's [`Observer::file`] hook.
///
/// [`Observer::file`]: crate::Observer::file
pub(crate) fn observed(
    input: &FileInput,
    options: &Options<'_>,
    grammars: &dyn Fn(&LanguageId) -> Option<SemanticGrammar>,
) -> Result<FileDiff> {
    let Some(observer) = options.observer else {
        return diff_one(input, options, grammars);
    };
    let mut out = None;
    observer.file(input.display_path(), &mut || {
        out = Some(diff_one(input, options, grammars));
    });
    match out {
        Some(out) => out,
        // The observer did not run the work; it still has to happen.
        None => diff_one(input, options, grammars),
    }
}

/// Diffs `inputs` on worker threads, which take the next file in order
/// until none remain or the request is cancelled.
pub(crate) fn diff_all(
    inputs: &[FileInput],
    options: &Options<'_>,
    grammars: &Grammars<'_>,
) -> Result<Vec<FileDiff>> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let workers = match options.workers {
        0 => worker_count(inputs.len()),
        workers => workers.clamp(1, inputs.len()),
    };
    let total = inputs.len() as u32;
    let next = AtomicUsize::new(0);
    let seen = AtomicU32::new(0);
    let mut done = std::thread::scope(|scope| {
        let handles = (0..workers)
            .map(|index| {
                std::thread::Builder::new()
                    .name(format!("diffy-semantic-{index}"))
                    .stack_size(WORKER_STACK_BYTES)
                    .spawn_scoped(scope, || {
                        let mut done = Vec::new();
                        while !cancelled(options) {
                            let at = next.fetch_add(1, Ordering::Relaxed);
                            let Some(input) = inputs.get(at) else {
                                break;
                            };
                            done.push((at, observed(input, options, grammars)));
                            let seen = seen.fetch_add(1, Ordering::Relaxed).saturating_add(1);
                            if let Some(observer) = options.observer {
                                observer.progress(seen, total);
                            }
                        }
                        done
                    })
                    .map_err(|error| Error::Worker(format!("setup failed: {error}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut done = Vec::with_capacity(inputs.len());
        for handle in handles {
            let part = handle
                .join()
                .map_err(|_| Error::Worker("a worker panicked".to_owned()))?;
            done.extend(part);
        }
        Ok::<_, Error>(done)
    })?;
    if done.len() < inputs.len() {
        return Err(Error::Cancelled);
    }
    done.sort_unstable_by_key(|(at, _)| *at);
    done.into_iter().map(|(_, diff)| diff).collect()
}

/// Runs `work` for every index in `0..len` on worker threads and returns
/// the results in index order. The calling thread works too, so a worker
/// that cannot start only costs parallelism; a worker's panic is resumed
/// on the caller.
#[cfg(feature = "serde")]
pub(crate) fn map_parallel<T: Send>(len: usize, work: &(dyn Fn(usize) -> T + Sync)) -> Vec<T> {
    let next = AtomicUsize::new(0);
    let run = || {
        let mut done = Vec::new();
        loop {
            let at = next.fetch_add(1, Ordering::Relaxed);
            if at >= len {
                return done;
            }
            done.push((at, work(at)));
        }
    };
    let mut done = std::thread::scope(|scope| {
        let handles: Vec<_> = (1..worker_count(len))
            .filter_map(|index| {
                std::thread::Builder::new()
                    .name(format!("diffy-wire-{index}"))
                    .stack_size(WORKER_STACK_BYTES)
                    .spawn_scoped(scope, run)
                    .ok()
            })
            .collect();
        let mut done = run();
        for handle in handles {
            match handle.join() {
                Ok(part) => done.extend(part),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        done
    });
    done.sort_unstable_by_key(|(at, _)| *at);
    done.into_iter().map(|(_, value)| value).collect()
}

fn worker_count(files: usize) -> usize {
    if files < MIN_FILES_FOR_PARALLEL {
        return 1;
    }
    let available = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    files.min(available).clamp(1, MAX_WORKERS)
}
