//! Where grammars come from: [`GrammarStore`].

#[cfg(feature = "engine")]
use std::collections::HashMap;
#[cfg(feature = "engine")]
use std::path::{Path, PathBuf};
#[cfg(feature = "engine")]
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(feature = "engine")]
use crate::engine::Grammar;
#[cfg(feature = "engine")]
use crate::pack::{self, PackManifest};
use crate::{HighlightSpan, LanguageId};

/// Whether a language can be highlighted yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageStatus {
    /// Its grammar is loaded.
    Ready,
    /// Its grammar is being fetched; text stays plain until it arrives, and
    /// subscribers hear when it does.
    Pending,
    /// No pack provides it (or its pack failed a check). Remembered for
    /// the store's lifetime, so asking again is a map lookup.
    Unavailable,
}

/// The grammars an app can highlight with, shared by every highlighter.
/// Cloning is cheap and clones share state.
///
/// Packs come from, in order:
///
/// 1. local pack roots the app opts into ([`StoreConfig::local_packs`]),
///    such as packs bundled with the app or built by `syntax-pack` in
///    development;
/// 2. with the `download` feature, the cache of a signed index the app
///    configures ([`StoreConfig::downloads`]), downloading a missing pack
///    on a background thread the first time its language is asked for.
///
/// [`GrammarStore::none`] (the default) has no grammars: everything is
/// plain. Lookups may read and load packs from disk, so they belong off the
/// UI thread; [`crate::HighlightWorker`] makes them on its own thread.
#[derive(Clone, Default)]
pub struct GrammarStore {
    #[cfg(feature = "engine")]
    inner: Option<Arc<Inner>>,
}

impl std::fmt::Debug for GrammarStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GrammarStore")
    }
}

impl GrammarStore {
    /// A store without grammars.
    pub fn none() -> Self {
        Self::default()
    }

    #[cfg(feature = "engine")]
    pub fn new(config: StoreConfig) -> Self {
        Self {
            inner: Some(Arc::new(Inner {
                config,
                state: Mutex::default(),
                listeners: Mutex::default(),
                #[cfg(feature = "download")]
                fetcher: Mutex::default(),
            })),
        }
    }

    /// Resolves `language`, loading its pack or starting its download on
    /// first use.
    pub fn status(&self, language: &LanguageId) -> LanguageStatus {
        #[cfg(feature = "engine")]
        if let Some(inner) = &self.inner {
            return match inner.lookup(language.as_str()) {
                Tag::Ready(_) => LanguageStatus::Ready,
                Tag::Pending => LanguageStatus::Pending,
                Tag::Unavailable => LanguageStatus::Unavailable,
            };
        }
        let _ = language;
        LanguageStatus::Unavailable
    }

    /// Calls `listener` (on a background thread) each time a pending
    /// language resolves, whether its grammar arrived or failed. Return
    /// `false` to unsubscribe. Apps use it to wake their event loop.
    pub fn subscribe(&self, listener: impl Fn() -> bool + Send + 'static) {
        #[cfg(feature = "engine")]
        if let Some(inner) = &self.inner {
            lock(&inner.listeners).push(Box::new(listener));
        }
        #[cfg(not(feature = "engine"))]
        let _ = listener;
    }

    /// Resolves the languages [`StoreConfig::defer`] held back and tells
    /// subscribers, as a finished download would.
    #[doc(hidden)]
    pub fn release_deferred(&self) {
        #[cfg(feature = "engine")]
        if let Some(inner) = &self.inner {
            let mut state = lock(&inner.state);
            state.released = true;
            // They resolve again on their next lookup.
            for tag in std::mem::take(&mut state.deferred) {
                state.tags.remove(&tag);
            }
            drop(state);
            inner.notify();
        }
    }

    /// Highlights `source` with `language`'s grammar and the grammars of
    /// languages embedded in it, or returns no spans while the grammar is
    /// still on its way.
    pub(crate) fn highlight(&self, language: &LanguageId, source: &str) -> Outcome {
        self.highlight_until(language, source, &|| false)
    }

    /// [`Self::highlight`], stopping with no spans once `cancelled`
    /// returns true (it is polled while parsing and searching).
    pub(crate) fn highlight_until(
        &self,
        language: &LanguageId,
        source: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Outcome {
        #[cfg(feature = "engine")]
        {
            self.highlight_within(
                language,
                source,
                crate::engine::Limits::for_source(source.len()),
                cancelled,
            )
        }
        #[cfg(not(feature = "engine"))]
        {
            let _ = (language, source, cancelled);
            Outcome::default()
        }
    }

    /// [`Self::highlight_until`] over each of `fragments` (sorted, disjoint
    /// byte ranges of `source`) as a document of its own, so no lexical
    /// state crosses from one into the next: a comment left open at the end
    /// of one hunk of a patch does not swallow the next. Spans are in
    /// `source`'s coordinates. A range that is out of order, out of bounds,
    /// or not on character boundaries is skipped.
    pub(crate) fn highlight_fragments_until(
        &self,
        language: &LanguageId,
        source: &str,
        fragments: &[std::ops::Range<u32>],
        cancelled: &dyn Fn() -> bool,
    ) -> Outcome {
        let mut out = Outcome::default();
        let mut end = 0;
        for fragment in fragments {
            let range = fragment.start as usize..fragment.end as usize;
            if range.start < end {
                continue;
            }
            let Some(text) = source.get(range.clone()) else {
                continue;
            };
            end = range.end;
            let found = self.highlight_until(language, text, cancelled);
            if cancelled() {
                return Outcome::default();
            }
            out.spans
                .extend(found.spans.into_iter().map(|span| HighlightSpan {
                    offset: span.offset + fragment.start,
                    ..span
                }));
            for language in found.unresolved {
                if !out.unresolved.contains(&language) {
                    out.unresolved.push(language);
                }
            }
            out.truncated |= found.truncated;
        }
        out
    }

    /// [`Self::highlight_until`] for a long source, window by window (see
    /// `engine::highlight_windows`): each window goes to `emit` as it is
    /// done, and the outcome returned has no spans of its own, only the
    /// languages still arriving across all windows. A source no longer than
    /// one window comes back whole, as from [`Self::highlight_until`].
    pub(crate) fn highlight_streamed(
        &self,
        language: &LanguageId,
        source: &str,
        slice: &Slice,
        cancelled: &(dyn Fn() -> bool + Sync),
        focus: &dyn Fn() -> Option<usize>,
        emit: &mut dyn FnMut(Part),
    ) -> Outcome {
        #[cfg(feature = "engine")]
        {
            self.highlight_windowed(
                language,
                source,
                crate::engine::Windowing::DEFAULT,
                slice,
                cancelled,
                focus,
                emit,
            )
        }
        #[cfg(not(feature = "engine"))]
        {
            let _ = (language, source, slice, cancelled, focus, emit);
            Outcome::default()
        }
    }

    /// [`Self::highlight_streamed`] with explicit window sizes.
    #[cfg(feature = "engine")]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn highlight_windowed(
        &self,
        language: &LanguageId,
        source: &str,
        sizes: crate::engine::Windowing,
        slice: &Slice,
        cancelled: &(dyn Fn() -> bool + Sync),
        focus: &dyn Fn() -> Option<usize>,
        emit: &mut dyn FnMut(Part),
    ) -> Outcome {
        {
            let Some(inner) = &self.inner else {
                return Outcome::default();
            };
            if source.len() <= sizes.window {
                return self.highlight_until(language, source, cancelled);
            }
            match inner.lookup(language.as_str()) {
                Tag::Ready(grammar) => {
                    let mut out = Outcome {
                        streamed: true,
                        ..Outcome::default()
                    };
                    let end = crate::engine::highlight_windows(
                        &grammar,
                        source,
                        sizes,
                        (slice.from, slice.inexact.clone(), slice.yield_after),
                        &|embedded| inner.lookup(embedded.as_str()),
                        cancelled,
                        focus,
                        &mut |window| {
                            let found = window.highlights;
                            for language in &found.unresolved {
                                if !out.unresolved.contains(language) {
                                    out.unresolved.push(language.clone());
                                }
                            }
                            out.truncated |= found.truncated;
                            emit(Part {
                                range: window.range.start as u32..window.range.end as u32,
                                exact: window.exact,
                                spans: found.spans,
                                unresolved: found.unresolved,
                            });
                        },
                    );
                    if let crate::engine::WindowsEnd::Yielded(at, inexact) = end {
                        out.resume = Some((at, inexact));
                    }
                    out
                }
                Tag::Pending => Outcome {
                    spans: Vec::new(),
                    unresolved: vec![language.clone()],
                    ..Outcome::default()
                },
                Tag::Unavailable => Outcome::default(),
            }
        }
    }

    /// [`Self::highlight_until`] with explicit bounds on embedded layers.
    #[cfg(feature = "engine")]
    pub(crate) fn highlight_within(
        &self,
        language: &LanguageId,
        source: &str,
        limits: crate::engine::Limits,
        cancelled: &dyn Fn() -> bool,
    ) -> Outcome {
        let Some(inner) = &self.inner else {
            return Outcome::default();
        };
        match inner.lookup(language.as_str()) {
            Tag::Ready(grammar) => {
                // Each lookup takes the state lock only to clone a handle;
                // parsing runs without it.
                let found = crate::engine::highlight(
                    &grammar,
                    source,
                    limits,
                    &mut |embedded| inner.lookup(embedded.as_str()),
                    cancelled,
                );
                Outcome {
                    spans: found.spans,
                    unresolved: found.unresolved,
                    truncated: found.truncated,
                    work: found.work,
                    streamed: false,
                    resume: None,
                }
            }
            Tag::Pending => Outcome {
                spans: Vec::new(),
                unresolved: vec![language.clone()],
                ..Outcome::default()
            },
            Tag::Unavailable => Outcome::default(),
        }
    }
}

/// A highlight, possibly partial because grammars are still arriving.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub(crate) spans: Vec<HighlightSpan>,
    /// Languages whose grammars are still arriving: the language itself
    /// (and `spans` is empty) or ones embedded in it (whose regions keep
    /// the host's colors for now).
    pub(crate) unresolved: Vec<LanguageId>,
    /// Bounds on embedded layers left some regions unparsed.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) truncated: bool,
    #[cfg(feature = "engine")]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) work: crate::engine::Work,
    /// The spans went out window by window as [`Part`]s; `spans` is empty.
    pub(crate) streamed: bool,
    /// A streamed highlight yielded: run it again from this byte, with
    /// the inexact window it colored last.
    pub(crate) resume: Option<(usize, Option<std::ops::Range<usize>>)>,
}

/// Where a streamed highlight starts its exact pass, and when it should
/// stop to let other work run: asked after each window it finishes, with
/// how many it finished in this call.
#[cfg_attr(not(feature = "engine"), allow(dead_code))]
pub(crate) struct Slice<'a> {
    pub(crate) from: usize,
    /// The inexact window an earlier turn colored last.
    pub(crate) inexact: Option<std::ops::Range<usize>>,
    pub(crate) yield_after: &'a dyn Fn(usize) -> bool,
}

impl Slice<'_> {
    /// From the start, never yielding.
    #[cfg(test)]
    pub(crate) const WHOLE: Slice<'static> = Slice {
        from: 0,
        inexact: None,
        yield_after: &|_| false,
    };
}

/// One window of a streamed highlight: spans of `source`, all inside
/// `range`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Part {
    pub(crate) range: std::ops::Range<u32>,
    pub(crate) exact: bool,
    pub(crate) spans: Vec<HighlightSpan>,
    pub(crate) unresolved: Vec<LanguageId>,
}

impl Outcome {
    /// Whether a fuller highlight may follow once grammars arrive.
    pub(crate) fn pending(&self) -> bool {
        !self.unresolved.is_empty()
    }
}

/// What a [`GrammarStore`] may load. Nothing is loaded unless the app
/// names it here: there is no default pack directory and no default index.
#[cfg(feature = "engine")]
#[derive(Debug, Clone, Default)]
pub struct StoreConfig {
    local_packs: Vec<PathBuf>,
    /// Languages whose local packs stay pending until
    /// [`GrammarStore::release_deferred`], as if they were downloading.
    deferred: Vec<String>,
    #[cfg(feature = "download")]
    pub(crate) downloads: Option<crate::download::Downloads>,
}

#[cfg(feature = "engine")]
impl StoreConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Trusts the packs under `root` (laid out `<root>/<target>/<language>/`,
    /// as `syntax-pack build` writes them). Local packs are not signed:
    /// loading one runs its code, so only name directories that only the
    /// app's own installer or the developer can write. Roots are searched
    /// in the order added, before any download.
    pub fn local_packs(mut self, root: impl Into<PathBuf>) -> Self {
        self.local_packs.push(root.into());
        self
    }

    /// Leaves the local packs of `languages` (pack language names) pending
    /// until [`GrammarStore::release_deferred`], the way a download leaves
    /// them, so tests can make a grammar arrive late without a server.
    #[doc(hidden)]
    pub fn defer(mut self, languages: &[&str]) -> Self {
        self.deferred
            .extend(languages.iter().map(|l| (*l).to_owned()));
        self
    }

    /// Downloads missing packs from a signed index.
    #[cfg(feature = "download")]
    pub fn downloads(mut self, downloads: crate::download::Downloads) -> Self {
        self.downloads = Some(downloads);
        self
    }
}

#[cfg(feature = "engine")]
#[derive(Clone)]
// Only downloads leave a language pending.
#[cfg_attr(not(feature = "download"), allow(dead_code))]
pub(crate) enum Tag {
    Ready(Arc<Grammar>),
    Pending,
    Unavailable,
}

#[cfg(feature = "engine")]
type Listener = Box<dyn Fn() -> bool + Send>;

#[cfg(feature = "engine")]
pub(crate) struct Inner {
    pub(crate) config: StoreConfig,
    pub(crate) state: Mutex<State>,
    listeners: Mutex<Vec<Listener>>,
    #[cfg(feature = "download")]
    pub(crate) fetcher: Mutex<Option<std::sync::mpsc::Sender<String>>>,
}

#[cfg(feature = "engine")]
#[derive(Default)]
pub(crate) struct State {
    /// Every tag asked for so far and what it resolved to.
    pub(crate) tags: HashMap<String, Tag>,
    /// Tag to pack directory over the local roots, scanned on first lookup.
    local: Option<HashMap<String, (PathBuf, Arc<PackManifest>)>>,
    /// Grammars by pack directory, so a language's aliases share one load.
    /// `None` records a pack that failed to load.
    loaded: HashMap<PathBuf, Option<Arc<Grammar>>>,
    /// [`StoreConfig::defer`]red packs have been released.
    released: bool,
    /// Tags left pending by [`StoreConfig::defer`].
    deferred: Vec<String>,
    #[cfg(feature = "download")]
    pub(crate) remote: crate::download::Remote,
}

#[cfg(feature = "engine")]
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding the lock leaves maps that are still valid.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(feature = "engine")]
impl Inner {
    pub(crate) fn lookup(self: &Arc<Self>, tag: &str) -> Tag {
        let mut state = lock(&self.state);
        if let Some(found) = state.tags.get(tag) {
            return found.clone();
        }
        let found = self.resolve(&mut state, tag);
        state.tags.insert(tag.to_owned(), found.clone());
        found
    }

    fn resolve(self: &Arc<Self>, state: &mut State, tag: &str) -> Tag {
        let local = state
            .local
            .get_or_insert_with(|| scan_local(&self.config.local_packs));
        if let Some((dir, manifest)) = local.get(tag).cloned() {
            if !state.released && self.config.deferred.contains(&manifest.language) {
                state.deferred.push(tag.to_owned());
                return Tag::Pending;
            }
            return load(state, &dir, &manifest);
        }
        #[cfg(feature = "download")]
        if let Some(downloads) = &self.config.downloads {
            return crate::download::resolve(self, downloads, state, tag);
        }
        Tag::Unavailable
    }

    pub(crate) fn notify(&self) {
        lock(&self.listeners).retain(|listener| listener());
    }
}

/// Loads (once per directory) the pack in `dir`.
#[cfg(feature = "engine")]
pub(crate) fn load(state: &mut State, dir: &Path, manifest: &PackManifest) -> Tag {
    let grammar = state.loaded.entry(dir.to_path_buf()).or_insert_with(|| {
        match Grammar::load(dir, manifest) {
            Ok(grammar) => Some(Arc::new(grammar)),
            Err(error) => {
                tracing::warn!(pack = %dir.display(), %error, "syntax pack failed to load");
                None
            }
        }
    });
    match grammar {
        Some(grammar) => Tag::Ready(grammar.clone()),
        None => Tag::Unavailable,
    }
}

/// Maps every tag of every valid pack under `roots` to its directory; the
/// first root, then the first pack in name order, wins a tag.
#[cfg(feature = "engine")]
fn scan_local(roots: &[PathBuf]) -> HashMap<String, (PathBuf, Arc<PackManifest>)> {
    let mut tags = HashMap::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root.join(pack::TARGET)) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for dir in dirs {
            if !dir.join(pack::MANIFEST).is_file() {
                continue;
            }
            let manifest = match pack::read_manifest(&dir) {
                Ok(manifest) => Arc::new(manifest),
                Err(error) => {
                    tracing::warn!(pack = %dir.display(), %error, "skipping syntax pack");
                    continue;
                }
            };
            for tag in manifest.tags() {
                tags.entry(tag.to_ascii_lowercase())
                    .or_insert_with(|| (dir.clone(), manifest.clone()));
            }
        }
    }
    tags
}
